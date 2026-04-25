use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn append_and_read_round_trip() {
    let dir = tempdir().unwrap();
    let j = ActivityJournal::new(dir.path().join("act.ndjson"), 200).unwrap();
    j.append("s1", "user", "hello world").await.unwrap();
    j.append("s1", "agent", "hi back").await.unwrap();
    let all = j.read_all().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].role, "user");
    assert_eq!(all[1].content, "hi back");
}

#[tokio::test]
async fn truncates_long_content() {
    let dir = tempdir().unwrap();
    let j = ActivityJournal::new(dir.path().join("act.ndjson"), 64).unwrap();
    let big = "a".repeat(500);
    j.append("s1", "user", &big).await.unwrap();
    let all = j.read_all().unwrap();
    let stored_chars = all[0].content.chars().count();
    // 64 truncation chars + 1 ellipsis.
    assert_eq!(stored_chars, 65);
    assert!(all[0].content.ends_with('…'));
}

#[tokio::test]
async fn malformed_lines_are_skipped() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("act.ndjson");
    tokio::fs::write(&path, "not json\n{\"timestamp\":\"2026-04-25T12:00:00Z\",\"session_key\":\"s\",\"role\":\"user\",\"content\":\"ok\"}\n").await.unwrap();
    let j = ActivityJournal::new(path, 200).unwrap();
    let all = j.read_all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].content, "ok");
}

#[tokio::test]
async fn missing_file_reads_empty() {
    let dir = tempdir().unwrap();
    let j = ActivityJournal::new(dir.path().join("never_written.ndjson"), 200).unwrap();
    assert!(j.read_all().unwrap().is_empty());
}
