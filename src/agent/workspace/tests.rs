use super::*;
use crate::agent::memory::memory_db::MemoryDB;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

// --- FileCategory::as_str / from_str round-trip ---

#[test]
fn test_category_as_str() {
    assert_eq!(FileCategory::Code.as_str(), "code");
    assert_eq!(FileCategory::Documents.as_str(), "documents");
    assert_eq!(FileCategory::Data.as_str(), "data");
    assert_eq!(FileCategory::Images.as_str(), "images");
    assert_eq!(FileCategory::Downloads.as_str(), "downloads");
    assert_eq!(FileCategory::Temp.as_str(), "temp");
}

#[test]
fn test_category_from_str() {
    assert_eq!(FileCategory::from_str("code"), Ok(FileCategory::Code));
    assert_eq!(
        FileCategory::from_str("documents"),
        Ok(FileCategory::Documents)
    );
    assert_eq!(FileCategory::from_str("data"), Ok(FileCategory::Data));
    assert_eq!(FileCategory::from_str("images"), Ok(FileCategory::Images));
    assert_eq!(
        FileCategory::from_str("downloads"),
        Ok(FileCategory::Downloads)
    );
    assert_eq!(FileCategory::from_str("temp"), Ok(FileCategory::Temp));
}

#[test]
fn test_category_from_str_unknown() {
    assert_eq!(
        FileCategory::from_str("unknown"),
        Err("unknown file category: unknown".to_string())
    );
    assert_eq!(
        FileCategory::from_str(""),
        Err("unknown file category: ".to_string())
    );
    assert_eq!(
        FileCategory::from_str("Code"),
        Err("unknown file category: Code".to_string())
    );
}

#[test]
fn test_category_round_trip_all() {
    for &cat in &FileCategory::ALL {
        let s = cat.as_str();
        let back = FileCategory::from_str(s).expect("round-trip should succeed");
        assert_eq!(back, cat);
    }
}

// --- infer_category ---

#[test]
fn test_infer_category_code_extensions() {
    let code_exts = [
        "main.py",
        "lib.rs",
        "app.js",
        "index.ts",
        "component.tsx",
        "widget.jsx",
        "script.sh",
        "run.bash",
        "app.rb",
        "main.go",
        "App.java",
        "hello.c",
        "hello.cpp",
        "header.h",
        "header.hpp",
        "page.html",
        "style.css",
        "query.sql",
        "init.lua",
        "index.php",
        "main.swift",
        "Main.kt",
        "Main.scala",
        "analysis.r",
        "script.pl",
        "main.zig",
        "app.nim",
        "mix.ex",
        "mix.exs",
        "server.erl",
    ];
    for name in &code_exts {
        assert_eq!(
            infer_category(Path::new(name)),
            FileCategory::Code,
            "expected Code for {name}"
        );
    }
}

#[test]
fn test_infer_category_document_extensions() {
    let doc_exts = [
        "readme.md",
        "notes.txt",
        "report.doc",
        "report.docx",
        "letter.rtf",
        "todo.org",
        "spec.rst",
        "guide.adoc",
        "paper.tex",
        "output.log",
    ];
    for name in &doc_exts {
        assert_eq!(
            infer_category(Path::new(name)),
            FileCategory::Documents,
            "expected Documents for {name}"
        );
    }
}

#[test]
fn test_infer_category_data_extensions() {
    let data_exts = [
        "data.csv",
        "config.json",
        "config.yaml",
        "config.yml",
        "feed.xml",
        "settings.toml",
        "warehouse.parquet",
        "export.tsv",
        "stream.ndjson",
        "stream.jsonl",
        "app.sqlite",
        "app.sqlite3",
        "app.db",
    ];
    for name in &data_exts {
        assert_eq!(
            infer_category(Path::new(name)),
            FileCategory::Data,
            "expected Data for {name}"
        );
    }
}

#[test]
fn test_infer_category_image_extensions() {
    let img_exts = [
        "photo.png",
        "photo.jpg",
        "photo.jpeg",
        "anim.gif",
        "logo.svg",
        "banner.webp",
        "icon.bmp",
        "favicon.ico",
        "scan.tiff",
        "scan.tif",
        "modern.avif",
        "apple.heic",
    ];
    for name in &img_exts {
        assert_eq!(
            infer_category(Path::new(name)),
            FileCategory::Images,
            "expected Images for {name}"
        );
    }
}

#[test]
fn test_infer_category_download_extensions() {
    let dl_exts = [
        "manual.pdf",
        "archive.zip",
        "backup.tar",
        "data.gz",
        "data.bz2",
        "data.xz",
        "archive.7z",
        "archive.rar",
        "book.epub",
        "book.mobi",
        "package.whl",
        "package.deb",
        "package.rpm",
        "installer.dmg",
        "disk.iso",
        "app.apk",
    ];
    for name in &dl_exts {
        assert_eq!(
            infer_category(Path::new(name)),
            FileCategory::Downloads,
            "expected Downloads for {name}"
        );
    }
}

#[test]
fn test_infer_category_unknown_extension() {
    assert_eq!(infer_category(Path::new("file.xyz")), FileCategory::Temp);
    assert_eq!(
        infer_category(Path::new("file.unknown")),
        FileCategory::Temp
    );
}

#[test]
fn test_infer_category_no_extension() {
    assert_eq!(infer_category(Path::new("Makefile")), FileCategory::Temp);
    assert_eq!(infer_category(Path::new("LICENSE")), FileCategory::Temp);
}

#[test]
fn test_infer_category_case_insensitive() {
    assert_eq!(infer_category(Path::new("Main.PY")), FileCategory::Code);
    assert_eq!(infer_category(Path::new("DATA.CSV")), FileCategory::Data);
    assert_eq!(
        infer_category(Path::new("README.MD")),
        FileCategory::Documents
    );
}

// --- WorkspaceManager::resolve_path ---

#[test]
fn test_resolve_path_infers_category() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);
    let path = ws.resolve_path("script.py", None);

    // Should be {workspace}/code/{YYYY-MM-DD}/script.py
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let expected = PathBuf::from(format!("/tmp/workspace/code/{today}/script.py"));
    assert_eq!(path, expected);
}

#[test]
fn test_resolve_path_with_category_hint() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);
    let path = ws.resolve_path("notes.txt", Some(FileCategory::Data));

    // category_hint overrides inferred category
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let expected = PathBuf::from(format!("/tmp/workspace/data/{today}/notes.txt"));
    assert_eq!(path, expected);
}

#[test]
fn test_resolve_path_unknown_falls_to_temp() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);
    let path = ws.resolve_path("Makefile", None);

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let expected = PathBuf::from(format!("/tmp/workspace/temp/{today}/Makefile"));
    assert_eq!(path, expected);
}

#[test]
fn test_resolve_path_various_categories() {
    let ws = WorkspaceManager::new("/tmp/ws".into(), None);
    let today = Utc::now().format("%Y-%m-%d").to_string();

    let cases = [
        ("report.md", "documents"),
        ("data.csv", "data"),
        ("logo.png", "images"),
        ("archive.zip", "downloads"),
        ("random.xyz", "temp"),
    ];

    for (filename, expected_cat) in &cases {
        let path = ws.resolve_path(filename, None);
        let expected = PathBuf::from(format!("/tmp/ws/{expected_cat}/{today}/{filename}"));
        assert_eq!(path, expected, "wrong path for {filename}");
    }
}

// --- WorkspaceManager::is_managed_path ---

#[test]
fn test_is_managed_path_category_dirs() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);

    assert!(ws.is_managed_path(Path::new("/tmp/workspace/code/2025-01-01/main.py")));
    assert!(ws.is_managed_path(Path::new("/tmp/workspace/documents/2025-01-01/notes.md")));
    assert!(ws.is_managed_path(Path::new("/tmp/workspace/data/file.csv")));
    assert!(ws.is_managed_path(Path::new("/tmp/workspace/images/logo.png")));
    assert!(ws.is_managed_path(Path::new("/tmp/workspace/downloads/archive.zip")));
    assert!(ws.is_managed_path(Path::new("/tmp/workspace/temp/scratch.txt")));
}

#[test]
fn test_is_managed_path_reserved_dirs() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);

    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/memory/notes.md")));
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/knowledge/faq.md")));
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/skills/my_skill/SKILL.md")));
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/sessions/abc.json")));
}

#[test]
fn test_is_managed_path_root_level_files() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);

    // Files directly in workspace root are not managed
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/README.md")));
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace/config.json")));
}

#[test]
fn test_is_managed_path_outside_workspace() {
    let ws = WorkspaceManager::new("/tmp/workspace".into(), None);

    assert!(!ws.is_managed_path(Path::new("/other/path/code/file.py")));
    assert!(!ws.is_managed_path(Path::new("/tmp/workspace2/code/file.py")));
}

// --- Edge cases ---

#[test]
fn test_all_constant_has_six_categories() {
    assert_eq!(FileCategory::ALL.len(), 6);
}

#[test]
fn test_resolve_path_filename_with_dots() {
    let ws = WorkspaceManager::new("/tmp/ws".into(), None);
    let path = ws.resolve_path("my.backup.tar.gz", None);

    // .gz maps to Downloads
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let expected = PathBuf::from(format!("/tmp/ws/downloads/{today}/my.backup.tar.gz"));
    assert_eq!(path, expected);
}

#[test]
fn test_resolve_path_rejects_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), None);
    let resolved = mgr.resolve_path("../../../etc/passwd", None);
    // Should extract just "passwd", not allow traversal
    assert!(resolved.starts_with(tmp.path()));
    assert!(resolved.to_string_lossy().contains("passwd"));
    assert!(!resolved.to_string_lossy().contains(".."));
}

#[test]
fn test_is_managed_path_rejects_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = WorkspaceManager::new(tmp.path().to_path_buf(), None);
    assert!(!mgr.is_managed_path(&tmp.path().join("code/../../etc/passwd")));
    assert!(!mgr.is_managed_path(&tmp.path().join("code/../memory/MEMORY.md")));
}

#[test]
fn test_category_display() {
    assert_eq!(FileCategory::Code.to_string(), "code");
    assert_eq!(FileCategory::Documents.to_string(), "documents");
    assert_eq!(FileCategory::Data.to_string(), "data");
    assert_eq!(FileCategory::Images.to_string(), "images");
    assert_eq!(FileCategory::Downloads.to_string(), "downloads");
    assert_eq!(FileCategory::Temp.to_string(), "temp");
}

// ── Manifest integration tests ──────────────────────────────

/// Helper: create a `WorkspaceManager` backed by a real `MemoryDB` in a temp dir.
fn test_manager() -> (tempfile::TempDir, WorkspaceManager) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("memory.db");
    let db = Arc::new(MemoryDB::new(&db_path).unwrap());
    let ws_root = dir.path().join("workspace");
    std::fs::create_dir_all(&ws_root).unwrap();
    let mgr = WorkspaceManager::new(ws_root, Some(db));
    (dir, mgr)
}

#[test]
fn test_register_file_adds_to_manifest() {
    let (_dir, mgr) = test_manager();

    // Create a file on disk in the code category
    let code_dir = mgr.workspace_root().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file_path = code_dir.join("main.py");
    std::fs::write(&file_path, "print('hello')").unwrap();

    // Register it
    mgr.register_file(&file_path, Some("code_gen"), Some("sess-1"))
        .unwrap();

    // Verify it appears in list_files
    let files = mgr
        .list_files(Some(FileCategory::Code), None, None)
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "code/2026-02-27/main.py");
    assert_eq!(files[0].category, "code");
    assert_eq!(files[0].original_name.as_deref(), Some("main.py"));
    assert_eq!(files[0].source_tool.as_deref(), Some("code_gen"));
    assert_eq!(files[0].session_key.as_deref(), Some("sess-1"));
    assert!(files[0].size_bytes > 0);
}

#[test]
fn test_remove_file_deletes_file_and_manifest() {
    let (_dir, mgr) = test_manager();

    // Create and register a file
    let data_dir = mgr.workspace_root().join("data/2026-02-27");
    std::fs::create_dir_all(&data_dir).unwrap();
    let file_path = data_dir.join("export.csv");
    std::fs::write(&file_path, "a,b,c\n1,2,3").unwrap();
    mgr.register_file(&file_path, None, None).unwrap();

    // Verify file exists
    assert!(file_path.exists());
    assert_eq!(mgr.list_files(None, None, None).unwrap().len(), 1);

    // Remove it
    mgr.remove_file(&file_path).unwrap();

    // Both disk and manifest should be empty
    assert!(!file_path.exists());
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
}

#[test]
fn test_sync_manifest_removes_stale_entries() {
    let (_dir, mgr) = test_manager();
    let db = mgr.db().unwrap();

    // Manually add a manifest entry for a nonexistent file
    db.register_workspace_file(
        "images/2026-01-01/ghost.png",
        "images",
        Some("ghost.png"),
        1024,
        None,
        None,
    )
    .unwrap();
    assert_eq!(mgr.list_files(None, None, None).unwrap().len(), 1);

    // Sync should detect and remove the stale entry
    let (removed, discovered) = mgr.sync_manifest().unwrap();
    assert_eq!(removed, 1);
    assert_eq!(discovered, 0);
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
}

#[test]
fn test_sync_manifest_discovers_untracked_files() {
    let (_dir, mgr) = test_manager();

    // Create a file on disk without registering it
    let img_dir = mgr.workspace_root().join("images/2026-02-27");
    std::fs::create_dir_all(&img_dir).unwrap();
    std::fs::write(img_dir.join("photo.png"), "fake png data").unwrap();

    assert!(mgr.list_files(None, None, None).unwrap().is_empty());

    // Sync should discover the untracked file
    let (removed, discovered) = mgr.sync_manifest().unwrap();
    assert_eq!(removed, 0);
    assert_eq!(discovered, 1);

    let files = mgr
        .list_files(Some(FileCategory::Images), None, None)
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].original_name.as_deref(), Some("photo.png"));
    assert_eq!(files[0].category, "images");
}

#[test]
fn test_move_file_changes_category() {
    let (_dir, mgr) = test_manager();

    // Create file in code/
    let code_dir = mgr.workspace_root().join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file_path = code_dir.join("notes.txt");
    std::fs::write(&file_path, "some notes").unwrap();
    mgr.register_file(&file_path, None, None).unwrap();

    // Move to data/
    let new_path = mgr.move_file(&file_path, FileCategory::Data).unwrap();

    // Old path should be gone, new path should exist
    assert!(!file_path.exists());
    assert!(new_path.exists());
    assert!(new_path.starts_with(mgr.workspace_root().join("data")));

    // Manifest should reflect the move
    let code_files = mgr
        .list_files(Some(FileCategory::Code), None, None)
        .unwrap();
    assert!(code_files.is_empty());

    let data_files = mgr
        .list_files(Some(FileCategory::Data), None, None)
        .unwrap();
    assert_eq!(data_files.len(), 1);
    assert_eq!(data_files[0].category, "data");
    assert!(data_files[0].path.starts_with("data/"));
}

#[test]
fn test_cleanup_expired_removes_old_files() {
    let (_dir, mgr) = test_manager();
    let db = mgr.db().unwrap();

    // Create a file on disk
    let tmp_dir = mgr.workspace_root().join("temp/2026-01-01");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let file_path = tmp_dir.join("old_scratch.txt");
    std::fs::write(&file_path, "old data").unwrap();

    // Register with a backdated created_at (60 days ago)
    db.register_workspace_file_with_date(
        "temp/2026-01-01/old_scratch.txt",
        "temp",
        Some("old_scratch.txt"),
        8,
        None,
        None,
        "2025-12-01 00:00:00",
    )
    .unwrap();

    assert!(file_path.exists());

    // Cleanup with 30-day TTL for temp
    let mut ttl_map = HashMap::new();
    ttl_map.insert("temp".to_string(), Some(30u64));
    ttl_map.insert("code".to_string(), None); // no expiry for code

    let removed = mgr.cleanup_expired(&ttl_map).unwrap();
    assert_eq!(removed, 1);
    assert!(!file_path.exists());
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
}

#[test]
fn test_operations_without_db_are_graceful() {
    let dir = tempfile::tempdir().unwrap();
    let ws_root = dir.path().join("workspace");
    std::fs::create_dir_all(&ws_root).unwrap();
    let mgr = WorkspaceManager::new(ws_root.clone(), None);

    // Create a real file for operations that touch the filesystem
    let code_dir = ws_root.join("code/2026-02-27");
    std::fs::create_dir_all(&code_dir).unwrap();
    let file_path = code_dir.join("test.py");
    std::fs::write(&file_path, "pass").unwrap();

    // All methods should return Ok/empty without panicking
    assert!(mgr.register_file(&file_path, None, None).is_ok());
    assert!(mgr.list_files(None, None, None).unwrap().is_empty());
    assert!(mgr.search_files("test").unwrap().is_empty());
    assert!(mgr.tag_file(&file_path, "foo").is_ok());
    assert!(mgr.touch_file(&file_path).is_ok());
    assert_eq!(mgr.cleanup_expired(&HashMap::new()).unwrap(), 0);
    assert_eq!(mgr.sync_manifest().unwrap(), (0, 0));
}
