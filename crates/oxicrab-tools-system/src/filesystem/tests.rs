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
