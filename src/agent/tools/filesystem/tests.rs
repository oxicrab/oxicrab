use super::*;
use crate::agent::tools::base::ExecutionContext;
use std::fs;

// --- check_path_allowed ---

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
    // Non-existent paths inside an allowed root should be allowed (for write operations)
    let roots = Some(vec![std::env::temp_dir()]);
    let result = check_path_allowed(Path::new("/tmp/does_not_exist_12345"), roots.as_ref());
    assert!(result.is_ok());
}

#[test]
fn test_check_path_allowed_nonexistent_traversal_blocked() {
    // Non-existent paths that use `..` to escape the root must be rejected
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

// --- ReadFileTool ---

#[tokio::test]
async fn test_read_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_read");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("test.txt");
    fs::write(&file, "hello world").unwrap();

    let tool = ReadFileTool::new(None);
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
    let tool = ReadFileTool::new(None);
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
async fn test_read_file_missing_param() {
    let tool = ReadFileTool::new(None);
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_read_file_not_a_file() {
    let tool = ReadFileTool::new(None);
    let result = tool
        .execute(
            serde_json::json!({"path": "/tmp"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Not a file (path is a directory)"));
}

#[tokio::test]
async fn test_read_file_path_restriction() {
    let dir = std::env::temp_dir().join("oxicrab_test_read_restricted");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret.txt");
    fs::write(&file, "secret").unwrap();

    // Allow only a different root
    let other = std::env::temp_dir().join("oxicrab_test_other_root");
    fs::create_dir_all(&other).unwrap();
    let tool = ReadFileTool::new(Some(vec![other.clone()]));
    let result = tool
        .execute(
            serde_json::json!({"path": file.to_str().unwrap()}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("outside the allowed directories"));

    fs::remove_dir_all(&dir).unwrap();
    fs::remove_dir_all(&other).unwrap();
}

// --- WriteFileTool ---

#[tokio::test]
async fn test_write_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_write");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("output.txt");

    let tool = WriteFileTool::new(None, None);
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
async fn test_write_file_creates_parent_dirs() {
    let dir = std::env::temp_dir().join("oxicrab_test_write_nested/a/b/c");
    let file = dir.join("deep.txt");

    let tool = WriteFileTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({"path": file.to_str().unwrap(), "content": "deep"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(fs::read_to_string(&file).unwrap(), "deep");

    fs::remove_dir_all(std::env::temp_dir().join("oxicrab_test_write_nested")).unwrap();
}

// --- EditFileTool ---

#[tokio::test]
async fn test_edit_file_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("edit.txt");
    fs::write(&file, "hello world").unwrap();

    let tool = EditFileTool::new(None, None);
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
async fn test_edit_file_old_text_not_found() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_nf");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("edit.txt");
    fs::write(&file, "hello world").unwrap();

    let tool = EditFileTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "missing text",
                "new_text": "replacement"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("old_text not found"));

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_edit_file_ambiguous_match() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_ambig");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("edit.txt");
    fs::write(&file, "foo bar foo baz").unwrap();

    let tool = EditFileTool::new(None, None);
    let result = tool
        .execute(
            serde_json::json!({
                "path": file.to_str().unwrap(),
                "old_text": "foo",
                "new_text": "qux"
            }),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("appears 2 times"));

    fs::remove_dir_all(&dir).unwrap();
}

// --- ListDirTool ---

#[tokio::test]
async fn test_list_dir_success() {
    let dir = std::env::temp_dir().join("oxicrab_test_listdir");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("a.txt"), "").unwrap();
    fs::write(dir.join("b.txt"), "").unwrap();
    fs::create_dir_all(dir.join("subdir")).unwrap();

    let tool = ListDirTool::new(None);
    let result = tool
        .execute(
            serde_json::json!({"path": dir.to_str().unwrap()}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("b.txt"));
    assert!(result.content.contains("subdir/"));

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_list_dir_not_found() {
    let tool = ListDirTool::new(None);
    let result = tool
        .execute(
            serde_json::json!({"path": "/tmp/oxicrab_nonexistent_dir_12345"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found") || result.content.contains("Cannot resolve"));
}

#[tokio::test]
async fn test_list_dir_not_a_directory() {
    let dir = std::env::temp_dir().join("oxicrab_test_listdir_file");
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    fs::write(&dir, "not a dir").unwrap();

    let tool = ListDirTool::new(None);
    let result = tool
        .execute(
            serde_json::json!({"path": dir.to_str().unwrap()}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("Not a directory"));

    fs::remove_file(&dir).unwrap();
}

// --- backup_file ---

#[tokio::test]
async fn test_backup_creates_copy() {
    let dir = std::env::temp_dir().join("oxicrab_test_backup_basic");
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
    let name = backups[0].file_name().to_string_lossy().to_string();
    assert!(
        name.starts_with("test.md."),
        "backup name should be prefixed: {}",
        name
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_backup_skips_nonexistent_file() {
    let dir = std::env::temp_dir().join("oxicrab_test_backup_skip");
    let backup_dir = dir.join("backups");
    let _ = fs::remove_dir_all(&dir);

    backup_file(&dir.join("nope.md"), &backup_dir).await;

    assert!(!backup_dir.exists());
}

#[tokio::test]
async fn test_backup_prunes_old_copies() {
    let dir = std::env::temp_dir().join("oxicrab_test_backup_prune");
    let backup_dir = dir.join("backups");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&backup_dir).unwrap();

    let file = dir.join("data.md");
    fs::write(&file, "content").unwrap();

    // Create 16 fake old backups (exceed MAX_BACKUPS of 14)
    for i in 0..16 {
        let name = format!("data.md.20250101-{:06}", i);
        fs::write(backup_dir.join(&name), format!("v{}", i)).unwrap();
    }

    // Trigger backup which should prune to 14
    backup_file(&file, &backup_dir).await;

    let count = fs::read_dir(&backup_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("data.md."))
        .count();
    assert_eq!(
        count, MAX_BACKUPS,
        "should keep exactly {} backups",
        MAX_BACKUPS
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_write_file_creates_backup() {
    let dir = std::env::temp_dir().join("oxicrab_test_write_backup");
    let backup_dir = dir.join("backups");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let file = dir.join("target.md");
    fs::write(&file, "before").unwrap();

    let tool = WriteFileTool::new(None, Some(backup_dir.clone()));
    let result = tool
        .execute(
            serde_json::json!({"path": file.to_str().unwrap(), "content": "after"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(fs::read_to_string(&file).unwrap(), "after");

    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read_to_string(backups[0].path()).unwrap(), "before");

    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn test_edit_file_creates_backup() {
    let dir = std::env::temp_dir().join("oxicrab_test_edit_backup");
    let backup_dir = dir.join("backups");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    let file = dir.join("target.md");
    fs::write(&file, "hello world").unwrap();

    let tool = EditFileTool::new(None, Some(backup_dir.clone()));
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

    let backups: Vec<_> = fs::read_dir(&backup_dir).unwrap().flatten().collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read_to_string(backups[0].path()).unwrap(),
        "hello world"
    );

    fs::remove_dir_all(&dir).unwrap();
}
