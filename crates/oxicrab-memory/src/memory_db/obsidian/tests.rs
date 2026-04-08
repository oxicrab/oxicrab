use super::super::MemoryDB;

#[test]
fn test_upsert_and_get_obsidian_sync() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.upsert_obsidian_sync("vault1", "notes/hello.md", "abc123", 1000, 42)
        .unwrap();

    let row = db
        .get_obsidian_sync("vault1", "notes/hello.md")
        .unwrap()
        .unwrap();
    assert_eq!(row.content_hash, "abc123");
    assert_eq!(row.last_synced_at, 1000);
    assert_eq!(row.size, 42);
}

#[test]
fn test_upsert_obsidian_sync_replaces() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.upsert_obsidian_sync("vault1", "a.md", "old", 100, 10)
        .unwrap();
    db.upsert_obsidian_sync("vault1", "a.md", "new", 200, 20)
        .unwrap();

    let row = db.get_obsidian_sync("vault1", "a.md").unwrap().unwrap();
    assert_eq!(row.content_hash, "new");
    assert_eq!(row.last_synced_at, 200);
    assert_eq!(row.size, 20);
}

#[test]
fn test_get_obsidian_sync_not_found() {
    let db = MemoryDB::new(":memory:").unwrap();
    assert!(
        db.get_obsidian_sync("vault1", "missing.md")
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_list_obsidian_sync() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
        .unwrap();
    db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
        .unwrap();
    db.upsert_obsidian_sync("vault2", "c.md", "h3", 300, 30)
        .unwrap();

    let map = db.list_obsidian_sync("vault1").unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map["a.md"].content_hash, "h1");
    assert_eq!(map["b.md"].content_hash, "h2");
}

#[test]
fn test_remove_obsidian_sync() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
        .unwrap();
    assert!(db.remove_obsidian_sync("vault1", "a.md").unwrap());
    assert!(!db.remove_obsidian_sync("vault1", "a.md").unwrap());
    assert!(db.get_obsidian_sync("vault1", "a.md").unwrap().is_none());
}

#[test]
fn test_clear_obsidian_sync() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
        .unwrap();
    db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
        .unwrap();
    db.upsert_obsidian_sync("vault2", "c.md", "h3", 300, 30)
        .unwrap();

    assert_eq!(db.clear_obsidian_sync("vault1").unwrap(), 2);
    assert!(db.list_obsidian_sync("vault1").unwrap().is_empty());
    assert_eq!(db.list_obsidian_sync("vault2").unwrap().len(), 1);
}

#[test]
fn test_get_last_full_sync() {
    let db = MemoryDB::new(":memory:").unwrap();

    assert_eq!(db.get_last_full_sync("vault1").unwrap(), 0);

    db.upsert_obsidian_sync("vault1", "a.md", "h1", 100, 10)
        .unwrap();
    db.upsert_obsidian_sync("vault1", "b.md", "h2", 200, 20)
        .unwrap();

    assert_eq!(db.get_last_full_sync("vault1").unwrap(), 100);
}

#[test]
fn test_add_and_list_obsidian_queue() {
    let db = MemoryDB::new(":memory:").unwrap();

    let id1 = db
        .add_obsidian_queue("vault1", "a.md", "content1", "write", 1000, Some("hash1"))
        .unwrap();
    let id2 = db
        .add_obsidian_queue("vault1", "b.md", "content2", "append", 2000, None)
        .unwrap();

    assert!(id1 > 0);
    assert!(id2 > id1);

    let queue = db.list_obsidian_queue("vault1").unwrap();
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0].path, "a.md");
    assert_eq!(queue[0].content, "content1");
    assert_eq!(queue[0].operation, "write");
    assert_eq!(queue[0].queued_at, 1000);
    assert_eq!(queue[0].pre_write_hash, Some("hash1".to_string()));
    assert_eq!(queue[1].path, "b.md");
    assert!(queue[1].pre_write_hash.is_none());
}

#[test]
fn test_remove_obsidian_queue() {
    let db = MemoryDB::new(":memory:").unwrap();

    let id = db
        .add_obsidian_queue("vault1", "a.md", "content", "write", 1000, None)
        .unwrap();
    assert!(db.remove_obsidian_queue(id).unwrap());
    assert!(!db.remove_obsidian_queue(id).unwrap());
    assert!(db.list_obsidian_queue("vault1").unwrap().is_empty());
}

#[test]
fn test_clear_obsidian_queue() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
        .unwrap();
    db.add_obsidian_queue("vault1", "b.md", "c2", "write", 2000, None)
        .unwrap();
    db.add_obsidian_queue("vault2", "c.md", "c3", "write", 3000, None)
        .unwrap();

    assert_eq!(db.clear_obsidian_queue("vault1").unwrap(), 2);
    assert!(db.list_obsidian_queue("vault1").unwrap().is_empty());
    assert_eq!(db.list_obsidian_queue("vault2").unwrap().len(), 1);
}

#[test]
fn test_count_obsidian_queue() {
    let db = MemoryDB::new(":memory:").unwrap();

    assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 0);

    db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
        .unwrap();
    db.add_obsidian_queue("vault1", "b.md", "c2", "write", 2000, None)
        .unwrap();

    assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 2);
}

#[test]
fn test_queue_vault_isolation() {
    let db = MemoryDB::new(":memory:").unwrap();

    db.add_obsidian_queue("vault1", "a.md", "c1", "write", 1000, None)
        .unwrap();
    db.add_obsidian_queue("vault2", "b.md", "c2", "write", 2000, None)
        .unwrap();

    assert_eq!(db.count_obsidian_queue("vault1").unwrap(), 1);
    assert_eq!(db.count_obsidian_queue("vault2").unwrap(), 1);
}
