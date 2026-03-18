//! Shell AST security analysis.

use brush_parser::ast;

/// Categories of structural security violations detected by AST analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViolationKind {
    Subshell,
    CommandSubstitution,
    ProcessSubstitution,
    EvalLike,
    InterpreterInlineExec,
    DangerousRedirection,
    DangerousPipeTarget,
    FunctionDefinition,
    Unparseable,
}

/// A single structural security violation found in a shell command.
#[derive(Debug, Clone)]
pub struct AstViolation {
    pub kind: ViolationKind,
    pub description: String,
}

const INTERPRETERS: &[&str] = &[
    "python", "python3", "python2", "perl", "ruby", "node", "php", "bash", "sh", "zsh", "dash",
    "lua", "tclsh",
];

const INLINE_EXEC_FLAGS: &[&str] = &["-c", "-e", "-r", "-x", "-E", "--eval"];

const DANGEROUS_PIPE_TARGETS: &[&str] = &[
    "bash", "sh", "zsh", "dash", "ksh", "csh", "tcsh", "fish", "python", "python3", "python2",
    "perl", "ruby", "node", "php",
];

const DANGEROUS_DEVICE_PREFIXES: &[&str] = &["/dev/sd", "/dev/nv", "/dev/hd", "/dev/vd"];

/// Analyze a shell command for structural security violations.
pub fn analyze_command(command: &str) -> Vec<AstViolation> {
    let cursor = std::io::Cursor::new(command);
    let reader = std::io::BufReader::new(cursor);
    let options = brush_parser::ParserOptions::default();
    let source_info = brush_parser::SourceInfo::default();

    let mut parser = brush_parser::Parser::new(reader, &options, &source_info);
    let Ok(program) = parser.parse_program() else {
        return vec![AstViolation {
            kind: ViolationKind::Unparseable,
            description: "command could not be parsed by shell analyzer".to_string(),
        }];
    };

    let mut violations = Vec::new();
    walk_program(&program, &mut violations);
    violations
}

fn walk_program(program: &ast::Program, violations: &mut Vec<AstViolation>) {
    for complete_command in &program.complete_commands {
        walk_compound_list(complete_command, violations);
    }
}

fn walk_compound_list(list: &ast::CompoundList, violations: &mut Vec<AstViolation>) {
    for item in &list.0 {
        walk_and_or_list(&item.0, violations);
    }
}

fn walk_and_or_list(and_or: &ast::AndOrList, violations: &mut Vec<AstViolation>) {
    walk_pipeline(&and_or.first, violations, false);
    for additional in &and_or.additional {
        let pipeline = match additional {
            ast::AndOr::And(p) | ast::AndOr::Or(p) => p,
        };
        walk_pipeline(pipeline, violations, false);
    }
}

fn walk_pipeline(pipeline: &ast::Pipeline, violations: &mut Vec<AstViolation>, _nested: bool) {
    let is_pipe = pipeline.seq.len() > 1;
    for (i, cmd) in pipeline.seq.iter().enumerate() {
        let is_last_in_pipe = is_pipe && i == pipeline.seq.len() - 1;
        walk_command(cmd, violations, is_last_in_pipe);
    }
}

fn walk_command(cmd: &ast::Command, violations: &mut Vec<AstViolation>, is_pipe_target: bool) {
    match cmd {
        ast::Command::Simple(simple) => {
            walk_simple_command(simple, violations, is_pipe_target);
        }
        ast::Command::Compound(compound, redirects) => {
            walk_compound_command(compound, violations);
            if let Some(redir_list) = redirects {
                for redir in &redir_list.0 {
                    check_io_redirect(redir, violations);
                }
            }
        }
        ast::Command::Function(func_def) => {
            violations.push(AstViolation {
                kind: ViolationKind::FunctionDefinition,
                description: format!(
                    "function definition '{}' can hide arbitrary code",
                    func_def.fname.value
                ),
            });
        }
        ast::Command::ExtendedTest(_) => {}
    }
}

fn walk_compound_command(compound: &ast::CompoundCommand, violations: &mut Vec<AstViolation>) {
    match compound {
        ast::CompoundCommand::Subshell(sub) => {
            violations.push(AstViolation {
                kind: ViolationKind::Subshell,
                description: "subshell (...) can hide commands from analysis".to_string(),
            });
            walk_compound_list(&sub.list, violations);
        }
        ast::CompoundCommand::BraceGroup(bg) => {
            walk_compound_list(&bg.list, violations);
        }
        ast::CompoundCommand::IfClause(ic) => {
            walk_compound_list(&ic.condition, violations);
            walk_compound_list(&ic.then, violations);
            if let Some(elses) = &ic.elses {
                for else_clause in elses {
                    if let Some(cond) = &else_clause.condition {
                        walk_compound_list(cond, violations);
                    }
                    walk_compound_list(&else_clause.body, violations);
                }
            }
        }
        ast::CompoundCommand::WhileClause(wc) | ast::CompoundCommand::UntilClause(wc) => {
            walk_compound_list(&wc.0, violations);
            walk_compound_list(&wc.1.list, violations);
        }
        ast::CompoundCommand::ForClause(fc) => {
            walk_compound_list(&fc.body.list, violations);
        }
        ast::CompoundCommand::CaseClause(cc) => {
            for case_item in &cc.cases {
                if let Some(cmd) = &case_item.cmd {
                    walk_compound_list(cmd, violations);
                }
            }
        }
        ast::CompoundCommand::ArithmeticForClause(afc) => {
            walk_compound_list(&afc.body.list, violations);
        }
        ast::CompoundCommand::Arithmetic(_) => {}
    }
}

fn walk_simple_command(
    cmd: &ast::SimpleCommand,
    violations: &mut Vec<AstViolation>,
    is_pipe_target: bool,
) {
    let cmd_name = cmd.word_or_name.as_ref().map_or("", |w| w.value.as_str());
    let basename = cmd_name.rsplit('/').next().unwrap_or(cmd_name);

    if matches!(basename, "eval" | "source" | ".") {
        violations.push(AstViolation {
            kind: ViolationKind::EvalLike,
            description: format!("'{basename}' executes arbitrary code"),
        });
    }

    if is_pipe_target && DANGEROUS_PIPE_TARGETS.contains(&basename) {
        violations.push(AstViolation {
            kind: ViolationKind::DangerousPipeTarget,
            description: format!("piping into '{basename}' allows arbitrary code execution"),
        });
    }

    if INTERPRETERS.iter().any(|i| {
        basename == *i
            || (basename.starts_with(i)
                && basename[i.len()..]
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.'))
    }) {
        check_interpreter_inline_exec(basename, cmd, violations);
    }

    if let Some(prefix) = &cmd.prefix {
        for item in &prefix.0 {
            check_prefix_suffix_item(item, violations);
        }
    }

    if let Some(suffix) = &cmd.suffix {
        for item in &suffix.0 {
            check_prefix_suffix_item(item, violations);
        }
    }

    if let Some(word) = &cmd.word_or_name {
        check_word_for_substitution(word, violations);
    }
}

fn check_interpreter_inline_exec(
    basename: &str,
    cmd: &ast::SimpleCommand,
    violations: &mut Vec<AstViolation>,
) {
    let suffix_words: Vec<&str> = cmd
        .suffix
        .as_ref()
        .map(|s| {
            s.0.iter()
                .filter_map(|item| match item {
                    ast::CommandPrefixOrSuffixItem::Word(w) => Some(w.value.as_str()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for word in &suffix_words {
        if word.starts_with('-') {
            if INLINE_EXEC_FLAGS.contains(word) {
                violations.push(AstViolation {
                    kind: ViolationKind::InterpreterInlineExec,
                    description: format!(
                        "'{basename}' with '{word}' flag allows inline code execution"
                    ),
                });
                return;
            }
        } else {
            break;
        }
    }
}

fn check_prefix_suffix_item(
    item: &ast::CommandPrefixOrSuffixItem,
    violations: &mut Vec<AstViolation>,
) {
    match item {
        ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
            check_io_redirect(redir, violations);
        }
        ast::CommandPrefixOrSuffixItem::ProcessSubstitution(_, _sub) => {
            violations.push(AstViolation {
                kind: ViolationKind::ProcessSubstitution,
                description: "process substitution can execute hidden commands".to_string(),
            });
        }
        ast::CommandPrefixOrSuffixItem::Word(w)
        | ast::CommandPrefixOrSuffixItem::AssignmentWord(_, w) => {
            check_word_for_substitution(w, violations);
        }
    }
}

fn check_word_for_substitution(word: &ast::Word, violations: &mut Vec<AstViolation>) {
    let value = &word.value;
    if value.contains("$(") {
        violations.push(AstViolation {
            kind: ViolationKind::CommandSubstitution,
            description: "command substitution $(...) can execute hidden commands".to_string(),
        });
    }
    if value.contains('`') {
        violations.push(AstViolation {
            kind: ViolationKind::CommandSubstitution,
            description: "backtick command substitution can execute hidden commands".to_string(),
        });
    }
}

fn check_io_redirect(redir: &ast::IoRedirect, violations: &mut Vec<AstViolation>) {
    if let ast::IoRedirect::File(_, kind, target) = redir {
        if matches!(
            kind,
            ast::IoFileRedirectKind::Write
                | ast::IoFileRedirectKind::Append
                | ast::IoFileRedirectKind::Clobber
        ) && let ast::IoFileRedirectTarget::Filename(word) = target
        {
            let path = &word.value;
            for prefix in DANGEROUS_DEVICE_PREFIXES {
                if path.starts_with(prefix) {
                    violations.push(AstViolation {
                        kind: ViolationKind::DangerousRedirection,
                        description: format!("writing to device '{path}' can destroy data"),
                    });
                    return;
                }
            }
        }
        if let ast::IoFileRedirectTarget::ProcessSubstitution(_, _) = target {
            violations.push(AstViolation {
                kind: ViolationKind::ProcessSubstitution,
                description: "process substitution in redirect can execute hidden commands"
                    .to_string(),
            });
        }
    }
}
