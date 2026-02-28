use super::*;
use std::path::Path;
use std::str::FromStr;

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
    assert!(FileCategory::from_str("unknown").is_err());
    assert!(FileCategory::from_str("").is_err());
    assert!(FileCategory::from_str("Code").is_err());
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
