use super::*;
use crate::agent::memory::memory_db::MemoryDB;
use std::sync::Arc;
use tempfile::tempdir;

fn db() -> (Arc<MemoryDB>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Arc::new(MemoryDB::new(dir.path().join("t.db")).unwrap());
    (db, dir)
}

#[tokio::test]
async fn logger_writes_through_to_db() {
    let (db, _g) = db();
    let logger = TrajectoryLogger::new(db.clone());
    logger.log_tool_call("s1", 1, "github", Some("list_issues"));
    logger.log_tool_result("s1", 1, "github", Some("list_issues"), false, 42);
    logger.log_turn_end("s1", 1);
    // log() spawns a blocking task — give it a moment to flush.
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    assert_eq!(db.count_trajectory_events().unwrap(), 3);
}
