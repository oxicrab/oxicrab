use super::*;
use crate::agent::tools::Tool;

#[test]
fn test_format_movie_search_results() {
    let results = vec![
        serde_json::json!({
            "title": "Interstellar",
            "year": 2014,
            "tmdbId": 157_336,
            "overview": "A team of explorers travel through a wormhole in space.",
            "id": 0
        }),
        serde_json::json!({
            "title": "Interstellar Wars",
            "year": 2016,
            "tmdbId": 390_876,
            "overview": "Aliens attack Earth.",
            "id": 42
        }),
    ];

    let output = format_movie_search_results(&results);
    assert!(output.contains("Found 2 movies"));
    assert!(output.contains("Interstellar (2014)"));
    assert!(output.contains("TMDB: 157336"));
    assert!(output.contains("Not in library"));
    assert!(output.contains("Interstellar Wars (2016)"));
    assert!(output.contains("Already in library"));
}

#[test]
fn test_format_movie_search_results_empty() {
    let output = format_movie_search_results(&[]);
    assert_eq!(output, "No movies found.");
}

#[test]
fn test_format_series_search_results() {
    let results = vec![serde_json::json!({
        "title": "Breaking Bad",
        "year": 2008,
        "tvdbId": 81189,
        "overview": "A high school chemistry teacher turned methamphetamine manufacturer.",
        "seasons": [{"seasonNumber": 1}, {"seasonNumber": 2}, {"seasonNumber": 3}, {"seasonNumber": 4}, {"seasonNumber": 5}],
        "id": 0
    })];

    let output = format_series_search_results(&results);
    assert!(output.contains("Found 1 series"));
    assert!(output.contains("Breaking Bad (2008)"));
    assert!(output.contains("TVDB: 81189"));
    assert!(output.contains("5 seasons"));
    assert!(output.contains("Not in library"));
}

#[test]
fn test_format_series_search_results_empty() {
    let output = format_series_search_results(&[]);
    assert_eq!(output, "No series found.");
}

#[test]
fn test_format_movie_detail() {
    let movie = serde_json::json!({
        "title": "Blade Runner 2049",
        "year": 2017,
        "id": 5,
        "tmdbId": 335_984,
        "status": "released",
        "hasFile": true,
        "sizeOnDisk": 5_368_709_120_i64,
        "path": "/movies/Blade Runner 2049 (2017)",
        "overview": "Thirty years after the events of the first film."
    });

    let output = format_movie_detail(&movie);
    assert!(output.contains("Blade Runner 2049 (2017)"));
    assert!(output.contains("Radarr ID: 5"));
    assert!(output.contains("TMDB: 335984"));
    assert!(output.contains("Downloaded (5.0 GB)"));
    assert!(output.contains("/movies/Blade Runner 2049 (2017)"));
}

#[test]
fn test_format_series_detail() {
    let series = serde_json::json!({
        "title": "The Office",
        "year": 2005,
        "id": 10,
        "tvdbId": 73244,
        "status": "ended",
        "path": "/tv/The Office",
        "overview": "A mockumentary on a group of typical office workers.",
        "statistics": {
            "seasonCount": 9,
            "episodeFileCount": 186,
            "episodeCount": 201,
            "sizeOnDisk": 107_374_182_400_i64
        }
    });

    let output = format_series_detail(&series);
    assert!(output.contains("The Office (2005)"));
    assert!(output.contains("Sonarr ID: 10"));
    assert!(output.contains("TVDB: 73244"));
    assert!(output.contains("9 seasons"));
    assert!(output.contains("Episodes: 186/201 downloaded"));
    assert!(output.contains("100.0 GB"));
}

#[test]
fn test_format_quality_profiles() {
    let profiles = vec![
        serde_json::json!({"id": 1, "name": "Any"}),
        serde_json::json!({"id": 4, "name": "HD-1080p"}),
        serde_json::json!({"id": 6, "name": "Ultra-HD"}),
    ];

    let output = format_quality_profiles(&profiles, "radarr");
    assert!(output.contains("Quality profiles (radarr)"));
    assert!(output.contains("ID: 1 — Any"));
    assert!(output.contains("ID: 4 — HD-1080p"));
    assert!(output.contains("ID: 6 — Ultra-HD"));
}

#[test]
fn test_format_quality_profiles_empty() {
    let output = format_quality_profiles(&[], "sonarr");
    assert_eq!(output, "No quality profiles found in sonarr.");
}

#[test]
fn test_format_root_folders() {
    let folders = vec![serde_json::json!({
        "id": 1,
        "path": "/movies",
        "freeSpace": 536_870_912_000_i64
    })];

    let output = format_root_folders(&folders, "radarr");
    assert!(output.contains("Root folders (radarr)"));
    assert!(output.contains("ID: 1 — /movies"));
    assert!(output.contains("500.0 GB free"));
}

#[test]
fn test_format_root_folders_empty() {
    let output = format_root_folders(&[], "sonarr");
    assert_eq!(output, "No root folders found in sonarr.");
}

#[tokio::test]
async fn test_missing_action() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(serde_json::json!({}), &ExecutionContext::default())
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("action"));
}

#[tokio::test]
async fn test_unknown_action() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(
            serde_json::json!({"action": "bogus"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
}

#[tokio::test]
async fn test_search_movie_missing_query() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(
            serde_json::json!({"action": "search_movie"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("query"));
}

#[tokio::test]
async fn test_add_movie_missing_tmdb_id() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(
            serde_json::json!({"action": "add_movie"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("tmdb_id"));
}

#[tokio::test]
async fn test_add_series_missing_tvdb_id() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(
            serde_json::json!({"action": "add_series"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("tvdb_id"));
}

#[tokio::test]
async fn test_profiles_missing_service() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let result = tool
        .execute(
            serde_json::json!({"action": "profiles"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("service"));
}

#[tokio::test]
async fn test_movie_search_truncates_long_overview() {
    let long_overview = "A".repeat(600);
    let results = vec![serde_json::json!({
        "title": "Test",
        "year": 2020,
        "tmdbId": 1,
        "overview": long_overview,
        "id": 0
    })];
    let output = format_movie_search_results(&results);
    assert!(output.contains("..."));
    // The overview should be truncated, not full 600 chars
    assert!(!output.contains(&long_overview));
}

// --- Capabilities tests ---

#[test]
fn test_media_capabilities() {
    use crate::agent::tools::base::SubagentAccess;
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let caps = tool.capabilities();
    assert!(caps.built_in);
    assert!(caps.network_outbound);
    assert_eq!(caps.subagent_access, SubagentAccess::ReadOnly);
    let read_only: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.name)
        .collect();
    let mutating: Vec<&str> = caps
        .actions
        .iter()
        .filter(|a| !a.read_only)
        .map(|a| a.name)
        .collect();
    assert!(read_only.contains(&"search_movie"));
    assert!(read_only.contains(&"get_movie"));
    assert!(read_only.contains(&"list_movies"));
    assert!(read_only.contains(&"search_series"));
    assert!(read_only.contains(&"get_series"));
    assert!(read_only.contains(&"list_series"));
    assert!(read_only.contains(&"profiles"));
    assert!(read_only.contains(&"root_folders"));
    assert!(mutating.contains(&"add_movie"));
    assert!(mutating.contains(&"add_series"));
}

#[test]
fn test_media_actions_match_schema() {
    let config = MediaConfig::default();
    let tool = MediaTool::new(&config);
    let caps = tool.capabilities();
    let params = tool.parameters();
    let schema_actions: Vec<String> = params["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let cap_actions: Vec<String> = caps.actions.iter().map(|a| a.name.to_string()).collect();
    for action in &schema_actions {
        assert!(
            cap_actions.contains(action),
            "action '{}' in schema but not in capabilities()",
            action
        );
    }
    for action in &cap_actions {
        assert!(
            schema_actions.contains(action),
            "action '{}' in capabilities() but not in schema",
            action
        );
    }
}
