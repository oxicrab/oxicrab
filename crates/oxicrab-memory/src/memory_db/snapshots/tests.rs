//! Regression tests for user-restorable memory snapshots.
//!
//! These defend the round-trip / reversibility contract of
//! `snapshot_memory` + `restore_snapshot`: a snapshot captures the exact
//! set of `memory_entries` at capture time, and restoring it swaps live
//! memory back to that set (entries added afterwards vanish), while the
//! auto-captured `pre-restore` snapshot makes the restore itself undoable.

use super::*;
use tempfile::tempdir;

fn db() -> (MemoryDB, tempfile::TempDir) {
    let d = tempdir().unwrap();
    (MemoryDB::new(d.path().join("t.db")).unwrap(), d)
}

/// A fresh snapshot records its label and the count of live entries at
/// capture time, and surfaces as the newest entry in `list_snapshots`.
#[test]
fn snapshot_then_list() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "alpha").unwrap();
    db.insert_memory("knowledge:b", "beta").unwrap();

    db.snapshot_memory("s1").unwrap();

    let snaps = db.list_snapshots(10).unwrap();
    let newest = &snaps[0];
    assert_eq!(newest.label, "s1");
    assert_eq!(newest.entry_count, 2);
}

/// Restoring a snapshot brings back exactly the entries present at capture
/// time and drops everything inserted afterwards. This is the core
/// contract: it FAILS if restore stops clearing the live table (post-
/// snapshot rows would survive) or miscounts what it restored.
#[test]
fn restore_round_trip() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    let id_a = db.snapshot_memory("a").unwrap();

    db.insert_memory("knowledge:b", "BBB").unwrap();
    // State B: both entries live.
    assert!(!db.get_recent_entries("knowledge:a", 10).unwrap().is_empty());
    assert!(!db.get_recent_entries("knowledge:b", 10).unwrap().is_empty());

    let outcome = db.restore_snapshot(id_a).unwrap();

    // entryA is back, entryB (inserted after the snapshot) is gone.
    assert!(
        db.get_recent_entries("knowledge:a", 10)
            .unwrap()
            .contains(&"AAA".to_string())
    );
    assert!(db.get_recent_entries("knowledge:b", 10).unwrap().is_empty());
    assert_eq!(outcome.restored_entries, 1);
}

/// The auto-captured `pre_restore_snapshot_id` snapshots the state that a
/// restore is about to overwrite, so restoring it undoes the restore and
/// brings the post-snapshot entry back.
#[test]
fn restore_is_reversible() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    let id_a = db.snapshot_memory("a").unwrap();

    db.insert_memory("knowledge:b", "BBB").unwrap();

    // Restore state A: entryB is now gone.
    let outcome = db.restore_snapshot(id_a).unwrap();
    assert!(db.get_recent_entries("knowledge:b", 10).unwrap().is_empty());

    // Restore the pre-restore snapshot (state B): entryB comes back.
    db.restore_snapshot(outcome.pre_restore_snapshot_id)
        .unwrap();
    assert!(
        db.get_recent_entries("knowledge:b", 10)
            .unwrap()
            .contains(&"BBB".to_string())
    );
    assert!(
        db.get_recent_entries("knowledge:a", 10)
            .unwrap()
            .contains(&"AAA".to_string())
    );
}

/// Restoring a nonexistent snapshot id is an error, not a silent no-op
/// that would wipe live memory to an empty set.
#[test]
fn restore_missing_id_errors() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    assert!(db.restore_snapshot(9999).is_err());
    // Live memory is untouched by the failed restore.
    assert!(
        db.get_recent_entries("knowledge:a", 10)
            .unwrap()
            .contains(&"AAA".to_string())
    );
}

/// `delete_snapshot` reports whether it removed a row (true once, false
/// thereafter) and the deleted snapshot disappears from `list_snapshots`.
#[test]
fn delete_snapshot_removes() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    let id = db.snapshot_memory("doomed").unwrap();

    assert!(db.delete_snapshot(id).unwrap());
    assert!(!db.delete_snapshot(id).unwrap());
    assert!(db.list_snapshots(10).unwrap().iter().all(|s| s.id != id));
}

/// Content survives a snapshot/restore byte-for-byte, including newlines
/// and non-ASCII. Guards against any lossy re-encoding in the payload
/// serialization or reinsert path.
#[test]
fn snapshot_preserves_content_exactly() {
    let (db, _g) = db();
    let exact = "line one\nlíne twö 🦀\n\ttrailing tab\tend";
    db.insert_memory("knowledge:x", exact).unwrap();
    let id = db.snapshot_memory("exact").unwrap();

    // Mutate live memory away from the snapshot state.
    db.insert_memory("knowledge:x", "some other content")
        .unwrap();

    db.restore_snapshot(id).unwrap();

    let restored = db.get_recent_entries("knowledge:x", 10).unwrap();
    assert!(restored.contains(&exact.to_string()));
    assert!(!restored.contains(&"some other content".to_string()));
}

/// The integrity gate recomputes SHA-256 of the stored payload and rejects
/// a restore whose payload no longer matches its recorded `content_sha256`,
/// so a corrupted snapshot can't silently overwrite intact live memory.
/// FAILS if the hash gate is removed: restore would proceed and wipe memory
/// to the corrupt (empty) payload.
#[test]
fn restore_rejects_corrupt_payload() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    db.insert_memory("knowledge:b", "BBB").unwrap();
    let id = db.snapshot_memory("s").unwrap();

    // Corrupt the stored payload so its SHA-256 no longer matches the
    // recorded content_sha256. `[]` is a valid (empty) SnapshotEntry list,
    // so only the hash gate — not deserialization — can catch this.
    {
        let conn = db.lock_conn().unwrap();
        conn.execute(
            "UPDATE memory_snapshots SET payload = '[]' WHERE id = ?1",
            [id],
        )
        .unwrap();
    }

    let err = db.restore_snapshot(id).unwrap_err();
    assert!(
        err.to_string().contains("integrity check"),
        "expected integrity check error, got: {err}"
    );

    // Live memory is untouched: both pre-corruption entries survive.
    assert!(
        db.get_recent_entries("knowledge:a", 10)
            .unwrap()
            .contains(&"AAA".to_string())
    );
    assert!(
        db.get_recent_entries("knowledge:b", 10)
            .unwrap()
            .contains(&"BBB".to_string())
    );
}

/// The auto `pre-restore` snapshot is captured INSIDE the restore
/// transaction, after the pre-mutation gates, so no failed restore leaves a
/// ghost `pre-restore` row. Two failure classes are covered: (a) gate
/// failures (bad id, corrupt payload) bail before the tx even opens, and
/// (b) a mid-tx failure AFTER the capture rolls the capture back with the
/// aborted tx. FAILS if the capture is moved before the gates (a corrupt /
/// bad-id restore would persist a pre-restore row) or committed outside the
/// tx (the aborted restore would leave one behind).
#[test]
fn failed_restore_leaves_no_pre_restore_snapshot() {
    let count_pre = |d: &MemoryDB| {
        d.list_snapshots(1000)
            .unwrap()
            .into_iter()
            .filter(|s| s.label == "pre-restore")
            .count()
    };

    // (a) Gate failures bail before the transaction opens.
    let (db1, _g) = db();
    db1.insert_memory("knowledge:a", "AAA").unwrap();
    let id = db1.snapshot_memory("s").unwrap();
    assert!(db1.restore_snapshot(9999).is_err());
    {
        let conn = db1.lock_conn().unwrap();
        conn.execute(
            "UPDATE memory_snapshots SET payload = '[]' WHERE id = ?1",
            [id],
        )
        .unwrap();
    }
    assert!(db1.restore_snapshot(id).is_err());
    assert_eq!(count_pre(&db1), 0);

    // (b) Mid-tx failure: a valid snapshot, but a trigger aborts the DELETE
    // that runs AFTER the in-tx pre-restore capture. The whole tx rolls
    // back, so the just-captured pre-restore row must not persist.
    let (db2, _g2) = db();
    db2.insert_memory("knowledge:a", "AAA").unwrap();
    let good = db2.snapshot_memory("s").unwrap();
    {
        let conn = db2.lock_conn().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER block_del BEFORE DELETE ON memory_entries \
             BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    }
    assert!(db2.restore_snapshot(good).is_err());
    assert_eq!(count_pre(&db2), 0);
}

/// `MAX_PRE_RESTORE_SNAPSHOTS` caps the auto `pre-restore` snapshots: each
/// restore appends one and prunes the oldest beyond the cap, so repeated
/// restores can't grow the snapshot table unboundedly. Manual/labeled
/// snapshots are never pruned. FAILS if the prune DELETE is removed: the
/// pre-restore count would exceed the cap.
#[test]
fn pre_restore_snapshots_are_bounded() {
    let (db, _g) = db();
    db.insert_memory("knowledge:a", "AAA").unwrap();
    let manual = db.snapshot_memory("manual").unwrap();

    // Restore well past the cap; each restore appends one pre-restore snap.
    for _ in 0..13 {
        db.restore_snapshot(manual).unwrap();
    }

    let snaps = db.list_snapshots(1000).unwrap();
    let pre_count = snaps.iter().filter(|s| s.label == "pre-restore").count();
    assert_eq!(pre_count, MAX_PRE_RESTORE_SNAPSHOTS);

    // The manual snapshot is never pruned.
    assert!(snaps.iter().any(|s| s.id == manual && s.label == "manual"));
}
