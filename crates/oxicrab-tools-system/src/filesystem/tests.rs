use super::*;
use oxicrab_core::tools::base::ExecutionContext;
use std::fs;

#[test]
fn test_check_path_allowed_none_allows_all() {
    let tmp = std::env::temp_dir();
    assert!(check_path_allowed(&tmp, None).is_ok());
}

#[test]
fn test_check_path_allowed_within_root() {
    let tmp = std::env::temp_dir();
    let roots = Some(vec![tmp.clone()]);
    assert!(check_path_allowed(&tmp, roots.as_ref()).is_ok());
}

#[test]
fn test_check_path_allowed_outside_root() {
    let roots = Some(vec![PathBuf::from("/tmp/oxicrab_test_nonexistent_root")]);
    let result = check_path_allowed(&std::env::temp_dir(), roots.as_ref());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("outside the allowed directories"));
}

#[test]
fn test_check_path_allowed_nonexistent_inside_root() {
    let tmp = std::env::temp_dir();
    let roots = Some(vec![tmp.clone()]);
    let result = check_path_allowed(&tmp.join("does_not_exist_12345"), roots.as_ref());
    assert!(result.is_ok());
}

#[test]
fn test_check_path_allowed_nonexistent_traversal_blocked() {
    let roots = Some(vec![std::env::temp_dir()]);
    let result = check_path_allowed(Path::new("/tmp/../etc/passwd"), roots.as_ref());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outside the allowed directories")
    );
}

#[test]
fn test_open_confined_normal_read() {
    let dir = std::env::temp_dir().join("oxicrab_test_confined_read");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("hello.txt"), "confined content").unwrap();

    let roots = vec![dir.clone()];
    let (cap_dir, relative) = open_confined(&dir.join("hello.txt"), &roots).unwrap();
    let content = cap_dir.read_to_string(&relative).unwrap();
    assert_eq!(content, "confined content");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_open_confined_dir_traversal_blocked() {
    let dir = std::env::temp_dir().join("oxicrab_test_confined_traversal");
    fs::create_dir_all(&dir).unwrap();

    let roots = vec![dir.clone()];
    let result = open_confined(&dir.join("../../etc/passwd"), &roots);
    assert!(result.is_err());

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_read_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_read_sys");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("test.txt");
    fs::write(&file, "hello world").unwrap();

    let tool = ReadFileTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({"path": file.to_str().unwrap()}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.content, "hello world");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_read_file_not_found() {
    let tool = ReadFileTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({"path": "/tmp/oxicrab_nonexistent_file_12345.txt"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found") || result.content.contains("Cannot resolve"));
}

#[tokio::test]
async fn test_write_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_write_sys");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("output.txt");

    let tool = WriteFileTool::new(None, None, None);
    let result = tool
        .execute(
            serde_json::json!({"path": file.to_str().unwrap(), "content": "test content"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("File written"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "test content");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_edit_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_sys");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("edit.txt");
    fs::write(&file, "hello world").unwrap();

    let tool = EditFileTool::new(None, None, None);
    let result = tool
        .execute(
            serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "hello",
                "new_text": "goodbye"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(fs::read_to_string(&file).unwrap(), "goodbye world");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_list_dir_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_listdir_sys");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.txt"), "").unwrap();
    fs::create_dir_all(dir.join("subdir")).unwrap();

    let tool = ListDirTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({"path": dir.to_str().unwrap()}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("subdir/"));

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_backup_creates_copy() {
    let dir = std::env::temp_dir().join("oxicrab_test_backup_basic_sys");
    let backup_dir = dir.join("backups");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("test.md");
    fs::write(&file, "original content").unwrap();

    backup_file(&file, &backup_dir).await;

    assert!(backup_dir.exists());
    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    let backup_content = fs::read_to_string(backups[0].path()).unwrap();
    assert_eq!(backup_content, "original content");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_expand_path_tilde() {
    let result = expand_path(Path::new("~/test.txt")).await;
    let expanded = result.expect("expand_path should succeed for ~/test.txt");
    assert!(
        expanded.to_str().unwrap().ends_with("test.txt"),
        "expanded path should end with test.txt, got: {}",
        expanded.display()
    );
    assert!(
        !expanded.to_str().unwrap().starts_with("~"),
        "expanded path should not start with ~, got: {}",
        expanded.display()
    );
}

#[tokio::test]
async fn test_expand_path_nonexistent_returns_normalized() {
    let result = expand_path(Path::new("/nonexistent/deep/path.txt")).await;
    let expanded = result.expect("expand_path should succeed for nonexistent paths");
    assert!(
        expanded
            .to_str()
            .unwrap()
            .contains("nonexistent/deep/path.txt"),
        "lexical normalize should preserve path components, got: {}",
        expanded.display()
    );
}

// ---- line-grounding: resolve_edit_occurrence / EditLineGuards / span_lines ----

/// `    x = 1` on lines 2, 3, and 4. No trailing newline in `DUP_OLD`, so each
/// occurrence covers exactly one line (no span overlap between neighbours).
const DUP_LINES: &str = "fn main() {\n    x = 1\n    x = 1\n    x = 1\n}\n";
const DUP_OLD: &str = "    x = 1";

#[test]
fn test_resolve_unique_no_guards_returns_offset() {
    let content = "fn main() {\n    println!(\"hi\");\n}\n";
    let expected = content.find("println!").unwrap();
    let got = resolve_edit_occurrence(content, "println!", EditLineGuards::default())
        .expect("unique old_text with no guards should resolve");
    assert_eq!(got, expected);
    assert_eq!(&content[got..got + "println!".len()], "println!");
}

#[test]
fn test_resolve_duplicate_no_guards_is_ambiguous_error() {
    let err = resolve_edit_occurrence(DUP_LINES, DUP_OLD, EditLineGuards::default())
        .expect_err("duplicate old_text with no guards must be ambiguous");
    assert!(
        err.contains("appears"),
        "message should say it appears N times: {err}"
    );
    assert!(
        err.contains("3 times"),
        "message should report the count 3: {err}"
    );
}

#[test]
fn test_resolve_target_line_picks_that_occurrence() {
    // Independent oracle: collect every occurrence offset ourselves.
    let offsets: Vec<usize> = DUP_LINES.match_indices(DUP_OLD).map(|(i, _)| i).collect();
    assert_eq!(offsets.len(), 3, "fixture must contain 3 occurrences");

    // target_line = 3 lands inside the SECOND occurrence's span.
    let guards = EditLineGuards {
        target_line: Some(3),
        ..Default::default()
    };
    let got = resolve_edit_occurrence(DUP_LINES, DUP_OLD, guards)
        .expect("target_line should disambiguate to one occurrence");
    assert_eq!(
        got, offsets[1],
        "should pick the 2nd occurrence, not the first"
    );

    // That offset really begins (and ends) on line 3.
    let (start_line, end_line) = span_lines(DUP_LINES, got, DUP_OLD.len());
    assert_eq!((start_line, end_line), (3, 3));

    // Behavioural check: splicing only changes line 3.
    let mut spliced = String::new();
    spliced.push_str(&DUP_LINES[..got]);
    spliced.push_str("    x = 2");
    spliced.push_str(&DUP_LINES[got + DUP_OLD.len()..]);
    assert_eq!(spliced, "fn main() {\n    x = 1\n    x = 2\n    x = 1\n}\n");
}

#[test]
fn test_resolve_guard_matching_no_occurrence_lists_actual_lines() {
    let guards = EditLineGuards {
        target_start_line: Some(99),
        ..Default::default()
    };
    let err = resolve_edit_occurrence(DUP_LINES, DUP_OLD, guards)
        .expect_err("a guard that matches nothing must error");
    assert!(
        err.contains("line(s)"),
        "message should list where the text is: {err}"
    );
    assert!(
        err.contains("2, 3, 4"),
        "message should name the actual lines: {err}"
    );
}

#[test]
fn test_resolve_not_found_errors() {
    let err = resolve_edit_occurrence(DUP_LINES, "no_such_text", EditLineGuards::default())
        .expect_err("absent old_text must error");
    assert!(
        err.contains("not found"),
        "message should say not found: {err}"
    );
}

#[test]
fn test_edit_line_guards_from_params_parses_and_filters() {
    let all = EditLineGuards::from_params(&serde_json::json!({
        "target_line": 3,
        "target_start_line": 1,
        "line_hint": 10,
    }));
    assert_eq!(all.target_line, Some(3));
    assert_eq!(all.target_start_line, Some(1));
    assert_eq!(all.line_hint, Some(10));
    assert!(!all.is_empty());

    // 0 and negatives are rejected by the `n >= 1` filter.
    let rejected = EditLineGuards::from_params(&serde_json::json!({
        "target_line": 0,
        "target_start_line": -5,
        "line_hint": 0,
    }));
    assert_eq!(rejected.target_line, None);
    assert_eq!(rejected.target_start_line, None);
    assert_eq!(rejected.line_hint, None);
    assert!(rejected.is_empty());

    // Missing keys => empty guards.
    assert!(EditLineGuards::from_params(&serde_json::json!({})).is_empty());
}

#[test]
fn test_span_lines_reports_start_and_end() {
    let content = "aaa\nbbb\nccc\nddd\n";
    struct Case {
        name: &'static str,
        needle: &'static str,
        want: (usize, usize),
    }
    let cases = [
        Case {
            name: "single line mid-file",
            needle: "bbb",
            want: (2, 2),
        },
        Case {
            name: "two-line span",
            needle: "bbb\nccc",
            want: (2, 3),
        },
        Case {
            name: "three-line span from start",
            needle: "aaa\nbbb\nccc",
            want: (1, 3),
        },
        Case {
            name: "first line",
            needle: "aaa",
            want: (1, 1),
        },
    ];
    for c in cases {
        let start = content.find(c.needle).unwrap();
        let got = span_lines(content, start, c.needle.len());
        assert_eq!(got, c.want, "case: {}", c.name);
    }
}

#[tokio::test]
async fn test_edit_file_target_line_disambiguates() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_target_line_sys");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("dup.rs");
    fs::write(&file, "fn main() {\n    x = 1\n    x = 1\n    x = 1\n}\n").unwrap();

    let tool = EditFileTool::new(None, None, None);
    let result = tool
        .execute(
            serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "    x = 1",
                "new_text": "    x = 2",
                "target_line": 3
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "guarded edit should succeed: {}",
        result.content
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "fn main() {\n    x = 1\n    x = 2\n    x = 1\n}\n",
        "only the line-3 occurrence should change"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_edit_file_ambiguous_without_guard_errors_and_leaves_file() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_ambiguous_sys");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("dup.rs");
    let original = "fn main() {\n    x = 1\n    x = 1\n    x = 1\n}\n";
    fs::write(&file, original).unwrap();

    let tool = EditFileTool::new(None, None, None);
    let result = tool
        .execute(
            serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "    x = 1",
                "new_text": "    x = 2"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error, "ambiguous edit must be rejected");
    assert!(
        result.content.contains("appears"),
        "should explain ambiguity: {}",
        result.content
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        original,
        "failed edit must not mutate the file"
    );

    fs::remove_dir_all(&dir).unwrap();
}
