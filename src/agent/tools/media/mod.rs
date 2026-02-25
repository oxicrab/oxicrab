use crate::agent::tools::base::{
    ActionDescriptor, ExecutionContext, SubagentAccess, ToolCapabilities,
};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use crate::config::MediaConfig;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

pub struct MediaTool {
    radarr_url: String,
    radarr_api_key: String,
    sonarr_url: String,
    sonarr_api_key: String,
    client: Client,
}

impl MediaTool {
    pub fn new(config: &MediaConfig) -> Self {
        Self {
            radarr_url: config.radarr.url.trim_end_matches('/').to_string(),
            radarr_api_key: config.radarr.api_key.clone(),
            sonarr_url: config.sonarr.url.trim_end_matches('/').to_string(),
            sonarr_api_key: config.sonarr.api_key.clone(),
            client: crate::utils::http::default_http_client(),
        }
    }

    // --- API helpers ---

    async fn radarr_get(&self, path: &str) -> Result<Value> {
        let resp = self
            .client
            .get(format!("{}{}", self.radarr_url, path))
            .header("X-Api-Key", &self.radarr_api_key)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"].as_str().unwrap_or("Unknown Radarr error");
            anyhow::bail!("Radarr API error ({}): {}", status, msg);
        }
        Ok(json)
    }

    async fn radarr_post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .client
            .post(format!("{}{}", self.radarr_url, path))
            .header("X-Api-Key", &self.radarr_api_key)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .or_else(|| json[0]["errorMessage"].as_str())
                .unwrap_or("Unknown Radarr error");
            anyhow::bail!("Radarr API error ({}): {}", status, msg);
        }
        Ok(json)
    }

    async fn sonarr_get(&self, path: &str) -> Result<Value> {
        let resp = self
            .client
            .get(format!("{}{}", self.sonarr_url, path))
            .header("X-Api-Key", &self.sonarr_api_key)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"].as_str().unwrap_or("Unknown Sonarr error");
            anyhow::bail!("Sonarr API error ({}): {}", status, msg);
        }
        Ok(json)
    }

    async fn sonarr_post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .client
            .post(format!("{}{}", self.sonarr_url, path))
            .header("X-Api-Key", &self.sonarr_api_key)
            .json(&body)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"]
                .as_str()
                .or_else(|| json[0]["errorMessage"].as_str())
                .unwrap_or("Unknown Sonarr error");
            anyhow::bail!("Sonarr API error ({}): {}", status, msg);
        }
        Ok(json)
    }

    // --- Action implementations ---

    async fn search_movie(&self, query: &str) -> Result<String> {
        let encoded = urlencoding::encode(query);
        let results = self
            .radarr_get(&format!("/api/v3/movie/lookup?term={}", encoded))
            .await?;
        let arr = results.as_array().map_or(&[][..], Vec::as_slice);
        Ok(format_movie_search_results(arr))
    }

    async fn add_movie(
        &self,
        tmdb_id: i64,
        quality_profile_id: Option<i64>,
        root_folder: Option<&str>,
    ) -> Result<String> {
        if tmdb_id <= 0 {
            anyhow::bail!("tmdb_id must be a positive number");
        }
        // Look up movie details first
        let lookup = self
            .radarr_get(&format!("/api/v3/movie/lookup/tmdb?tmdbId={}", tmdb_id))
            .await?;

        let title = lookup["title"].as_str().unwrap_or("Unknown");
        let year = lookup["year"].as_i64().unwrap_or(0);

        // Auto-fetch quality profile if not specified
        let profile_id = if let Some(id) = quality_profile_id {
            id
        } else {
            let profiles = self.radarr_get("/api/v3/qualityprofile").await?;
            profiles
                .as_array()
                .and_then(|a| a.first())
                .and_then(|p| p["id"].as_i64())
                .ok_or_else(|| anyhow::anyhow!("No quality profiles found in Radarr"))?
        };

        // Auto-fetch root folder if not specified
        let folder = if let Some(f) = root_folder {
            f.to_string()
        } else {
            let folders = self.radarr_get("/api/v3/rootfolder").await?;
            folders
                .as_array()
                .and_then(|a| a.first())
                .and_then(|f| f["path"].as_str())
                .ok_or_else(|| anyhow::anyhow!("No root folders found in Radarr"))?
                .to_string()
        };

        let body = serde_json::json!({
            "title": lookup["title"],
            "tmdbId": tmdb_id,
            "year": lookup["year"],
            "qualityProfileId": profile_id,
            "rootFolderPath": folder,
            "monitored": true,
            "addOptions": {
                "searchForMovie": true
            },
            "images": lookup["images"],
        });

        let result = self.radarr_post("/api/v3/movie", body).await?;
        let id = result["id"].as_i64().unwrap_or(0);
        Ok(format!(
            "Added: {} ({}) — Radarr ID: {}\nSearching for downloads...",
            title, year, id
        ))
    }

    async fn get_movie(&self, id: i64) -> Result<String> {
        let movie = self.radarr_get(&format!("/api/v3/movie/{}", id)).await?;
        Ok(format_movie_detail(&movie))
    }

    async fn list_movies(&self, filter: Option<&str>) -> Result<String> {
        let movies = self.radarr_get("/api/v3/movie").await?;
        let arr = movies.as_array().map_or(&[][..], Vec::as_slice);

        let filtered: Vec<&Value> = if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            arr.iter()
                .filter(|m| {
                    m["title"]
                        .as_str()
                        .is_some_and(|t| t.to_lowercase().contains(&f_lower))
                })
                .collect()
        } else {
            arr.iter().collect()
        };

        if filtered.is_empty() {
            return Ok("No movies found.".to_string());
        }

        let count = filtered.len();
        let display: Vec<String> = filtered
            .iter()
            .take(25)
            .enumerate()
            .map(|(i, m)| {
                let title = m["title"].as_str().unwrap_or("?");
                let year = m["year"].as_i64().unwrap_or(0);
                let has_file = m["hasFile"].as_bool().unwrap_or(false);
                let status = if has_file { "Downloaded" } else { "Missing" };
                format!("{}. {} ({}) — {}", i + 1, title, year, status)
            })
            .collect();

        let suffix = if count > 25 {
            format!("\n\n...and {} more", count - 25)
        } else {
            String::new()
        };

        Ok(format!(
            "Movies in library ({} total):\n\n{}{}",
            count,
            display.join("\n"),
            suffix
        ))
    }

    async fn search_series(&self, query: &str) -> Result<String> {
        let encoded = urlencoding::encode(query);
        let results = self
            .sonarr_get(&format!("/api/v3/series/lookup?term={}", encoded))
            .await?;
        let arr = results.as_array().map_or(&[][..], Vec::as_slice);
        Ok(format_series_search_results(arr))
    }

    async fn add_series(
        &self,
        tvdb_id: i64,
        quality_profile_id: Option<i64>,
        root_folder: Option<&str>,
    ) -> Result<String> {
        if tvdb_id <= 0 {
            anyhow::bail!("tvdb_id must be a positive number");
        }
        // Look up series details first
        let tvdb_str = tvdb_id.to_string();
        let encoded = urlencoding::encode(&tvdb_str);
        let lookup_results = self
            .sonarr_get(&format!("/api/v3/series/lookup?term=tvdb:{}", encoded))
            .await?;
        let lookup = lookup_results
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| anyhow::anyhow!("Series not found for TVDB ID {}", tvdb_id))?;

        let title = lookup["title"].as_str().unwrap_or("Unknown");
        let year = lookup["year"].as_i64().unwrap_or(0);

        // Auto-fetch quality profile if not specified
        let profile_id = if let Some(id) = quality_profile_id {
            id
        } else {
            let profiles = self.sonarr_get("/api/v3/qualityprofile").await?;
            profiles
                .as_array()
                .and_then(|a| a.first())
                .and_then(|p| p["id"].as_i64())
                .ok_or_else(|| anyhow::anyhow!("No quality profiles found in Sonarr"))?
        };

        // Auto-fetch root folder if not specified
        let folder = if let Some(f) = root_folder {
            f.to_string()
        } else {
            let folders = self.sonarr_get("/api/v3/rootfolder").await?;
            folders
                .as_array()
                .and_then(|a| a.first())
                .and_then(|f| f["path"].as_str())
                .ok_or_else(|| anyhow::anyhow!("No root folders found in Sonarr"))?
                .to_string()
        };

        let body = serde_json::json!({
            "title": lookup["title"],
            "tvdbId": tvdb_id,
            "year": lookup["year"],
            "qualityProfileId": profile_id,
            "rootFolderPath": folder,
            "monitored": true,
            "seasonFolder": true,
            "addOptions": {
                "searchForMissingEpisodes": true
            },
            "images": lookup["images"],
            "seasons": lookup["seasons"],
        });

        let result = self.sonarr_post("/api/v3/series", body).await?;
        let id = result["id"].as_i64().unwrap_or(0);
        Ok(format!(
            "Added: {} ({}) — Sonarr ID: {}\nSearching for missing episodes...",
            title, year, id
        ))
    }

    async fn get_series(&self, id: i64) -> Result<String> {
        let series = self.sonarr_get(&format!("/api/v3/series/{}", id)).await?;
        Ok(format_series_detail(&series))
    }

    async fn list_series(&self, filter: Option<&str>) -> Result<String> {
        let series = self.sonarr_get("/api/v3/series").await?;
        let arr = series.as_array().map_or(&[][..], Vec::as_slice);

        let filtered: Vec<&Value> = if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            arr.iter()
                .filter(|s| {
                    s["title"]
                        .as_str()
                        .is_some_and(|t| t.to_lowercase().contains(&f_lower))
                })
                .collect()
        } else {
            arr.iter().collect()
        };

        if filtered.is_empty() {
            return Ok("No series found.".to_string());
        }

        let count = filtered.len();
        let display: Vec<String> = filtered
            .iter()
            .take(25)
            .enumerate()
            .map(|(i, s)| {
                let title = s["title"].as_str().unwrap_or("?");
                let year = s["year"].as_i64().unwrap_or(0);
                let ep_count = s["statistics"]["episodeFileCount"].as_i64().unwrap_or(0);
                let total = s["statistics"]["episodeCount"].as_i64().unwrap_or(0);
                format!(
                    "{}. {} ({}) — {}/{} episodes",
                    i + 1,
                    title,
                    year,
                    ep_count,
                    total
                )
            })
            .collect();

        let suffix = if count > 25 {
            format!("\n\n...and {} more", count - 25)
        } else {
            String::new()
        };

        Ok(format!(
            "Series in library ({} total):\n\n{}{}",
            count,
            display.join("\n"),
            suffix
        ))
    }

    async fn profiles(&self, service: &str) -> Result<String> {
        let profiles = match service {
            "radarr" => self.radarr_get("/api/v3/qualityprofile").await?,
            "sonarr" => self.sonarr_get("/api/v3/qualityprofile").await?,
            _ => anyhow::bail!("Unknown service '{}'. Use 'radarr' or 'sonarr'.", service),
        };
        let arr = profiles.as_array().map_or(&[][..], Vec::as_slice);
        Ok(format_quality_profiles(arr, service))
    }

    async fn root_folders(&self, service: &str) -> Result<String> {
        let folders = match service {
            "radarr" => self.radarr_get("/api/v3/rootfolder").await?,
            "sonarr" => self.sonarr_get("/api/v3/rootfolder").await?,
            _ => anyhow::bail!("Unknown service '{}'. Use 'radarr' or 'sonarr'.", service),
        };
        let arr = folders.as_array().map_or(&[][..], Vec::as_slice);
        Ok(format_root_folders(arr, service))
    }
}

#[async_trait]
impl Tool for MediaTool {
    fn name(&self) -> &'static str {
        "media"
    }

    fn description(&self) -> &'static str {
        "Manage movies and TV series via Radarr (movies) and Sonarr (TV). \
         Actions: search_movie, add_movie (by TMDB ID), get_movie, list_movies, \
         search_series, add_series (by TVDB ID), get_series, list_series, \
         profiles (list quality profiles), root_folders (list root folders). \
         For add_movie/add_series, quality profile and root folder are auto-selected if omitted."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            actions: vec![
                ActionDescriptor {
                    name: "search_movie",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "add_movie",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "get_movie",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "list_movies",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "search_series",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "add_series",
                    read_only: false,
                },
                ActionDescriptor {
                    name: "get_series",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "list_series",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "profiles",
                    read_only: true,
                },
                ActionDescriptor {
                    name: "root_folders",
                    read_only: true,
                },
            ],
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search_movie", "add_movie", "get_movie", "list_movies",
                             "search_series", "add_series", "get_series", "list_series",
                             "profiles", "root_folders"],
                    "description": "Action to perform. Radarr manages movies, Sonarr manages TV series."
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search_movie, search_series)"
                },
                "tmdb_id": {
                    "type": "integer",
                    "description": "TMDB ID from search results (for add_movie)"
                },
                "tvdb_id": {
                    "type": "integer",
                    "description": "TVDB ID from search results (for add_series)"
                },
                "id": {
                    "type": "integer",
                    "description": "Radarr/Sonarr internal ID (for get_movie, get_series)"
                },
                "quality_profile_id": {
                    "type": "integer",
                    "description": "Quality profile ID (for add_movie, add_series). Auto-selected if omitted."
                },
                "root_folder": {
                    "type": "string",
                    "description": "Root folder path (for add_movie, add_series). Auto-selected if omitted."
                },
                "service": {
                    "type": "string",
                    "enum": ["radarr", "sonarr"],
                    "description": "Which service (for profiles, root_folders)"
                },
                "filter": {
                    "type": "string",
                    "description": "Filter text for list_movies/list_series (matches title)"
                }
            },
            "required": ["action"]
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let Some(action) = params["action"].as_str() else {
            return Ok(ToolResult::error("missing 'action' parameter".to_string()));
        };

        let result = match action {
            "search_movie" => {
                let Some(query) = params["query"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'query' parameter for search_movie".to_string(),
                    ));
                };
                self.search_movie(query).await
            }
            "add_movie" => {
                let Some(tmdb_id) = params["tmdb_id"].as_i64() else {
                    return Ok(ToolResult::error(
                        "missing 'tmdb_id' parameter for add_movie".to_string(),
                    ));
                };
                let qp = params["quality_profile_id"].as_i64();
                let rf = params["root_folder"].as_str();
                self.add_movie(tmdb_id, qp, rf).await
            }
            "get_movie" => {
                let Some(id) = params["id"].as_i64() else {
                    return Ok(ToolResult::error(
                        "missing 'id' parameter for get_movie".to_string(),
                    ));
                };
                self.get_movie(id).await
            }
            "list_movies" => {
                let filter = params["filter"].as_str();
                self.list_movies(filter).await
            }
            "search_series" => {
                let Some(query) = params["query"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'query' parameter for search_series".to_string(),
                    ));
                };
                self.search_series(query).await
            }
            "add_series" => {
                let Some(tvdb_id) = params["tvdb_id"].as_i64() else {
                    return Ok(ToolResult::error(
                        "missing 'tvdb_id' parameter for add_series".to_string(),
                    ));
                };
                let qp = params["quality_profile_id"].as_i64();
                let rf = params["root_folder"].as_str();
                self.add_series(tvdb_id, qp, rf).await
            }
            "get_series" => {
                let Some(id) = params["id"].as_i64() else {
                    return Ok(ToolResult::error(
                        "missing 'id' parameter for get_series".to_string(),
                    ));
                };
                self.get_series(id).await
            }
            "list_series" => {
                let filter = params["filter"].as_str();
                self.list_series(filter).await
            }
            "profiles" => {
                let Some(service) = params["service"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'service' parameter for profiles".to_string(),
                    ));
                };
                self.profiles(service).await
            }
            "root_folders" => {
                let Some(service) = params["service"].as_str() else {
                    return Ok(ToolResult::error(
                        "missing 'service' parameter for root_folders".to_string(),
                    ));
                };
                self.root_folders(service).await
            }
            _ => return Ok(ToolResult::error(format!("unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("media error: {}", e))),
        }
    }
}

// --- Formatting functions (extracted for testability) ---

fn format_movie_search_results(results: &[Value]) -> String {
    if results.is_empty() {
        return "No movies found.".to_string();
    }

    let entries: Vec<String> = results
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, m)| {
            let title = m["title"].as_str().unwrap_or("?");
            let year = m["year"].as_i64().unwrap_or(0);
            let tmdb_id = m["tmdbId"].as_i64().unwrap_or(0);
            let overview = m["overview"].as_str().unwrap_or("No overview available.");
            let overview_short = if overview.len() > 500 {
                // Find a safe char boundary
                let end = overview
                    .char_indices()
                    .take_while(|(idx, _)| *idx <= 500)
                    .last()
                    .map_or(500, |(idx, _)| idx);
                format!("{}...", &overview[..end])
            } else {
                overview.to_string()
            };
            let in_library = m["id"].as_i64().unwrap_or(0) > 0;
            let status = if in_library {
                "Already in library"
            } else {
                "Not in library"
            };

            format!(
                "{}. {} ({}) — TMDB: {}\n   {}\n   Status: {}",
                i + 1,
                title,
                year,
                tmdb_id,
                overview_short,
                status
            )
        })
        .collect();

    format!(
        "Found {} movie{}:\n\n{}",
        results.len().min(5),
        if results.len() == 1 { "" } else { "s" },
        entries.join("\n\n")
    )
}

fn format_series_search_results(results: &[Value]) -> String {
    if results.is_empty() {
        return "No series found.".to_string();
    }

    let entries: Vec<String> = results
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, s)| {
            let title = s["title"].as_str().unwrap_or("?");
            let year = s["year"].as_i64().unwrap_or(0);
            let tvdb_id = s["tvdbId"].as_i64().unwrap_or(0);
            let overview = s["overview"].as_str().unwrap_or("No overview available.");
            let overview_short = if overview.len() > 500 {
                let end = overview
                    .char_indices()
                    .take_while(|(idx, _)| *idx <= 500)
                    .last()
                    .map_or(500, |(idx, _)| idx);
                format!("{}...", &overview[..end])
            } else {
                overview.to_string()
            };
            let season_count = s["statistics"]["seasonCount"]
                .as_i64()
                .or_else(|| s["seasons"].as_array().map(|a| a.len() as i64))
                .unwrap_or(0);
            let in_library = s["id"].as_i64().unwrap_or(0) > 0;
            let status = if in_library {
                "Already in library"
            } else {
                "Not in library"
            };

            format!(
                "{}. {} ({}) — TVDB: {} — {} season{}\n   {}\n   Status: {}",
                i + 1,
                title,
                year,
                tvdb_id,
                season_count,
                if season_count == 1 { "" } else { "s" },
                overview_short,
                status
            )
        })
        .collect();

    format!(
        "Found {} series:\n\n{}",
        results.len().min(5),
        entries.join("\n\n")
    )
}

fn format_movie_detail(movie: &Value) -> String {
    let title = movie["title"].as_str().unwrap_or("?");
    let year = movie["year"].as_i64().unwrap_or(0);
    let status = movie["status"].as_str().unwrap_or("unknown");
    let has_file = movie["hasFile"].as_bool().unwrap_or(false);
    let path = movie["path"].as_str().unwrap_or("?");
    let size = movie["sizeOnDisk"].as_i64().unwrap_or(0);
    let size_gb = size as f64 / 1_073_741_824.0;
    let overview = movie["overview"].as_str().unwrap_or("");
    let radarr_id = movie["id"].as_i64().unwrap_or(0);
    let tmdb_id = movie["tmdbId"].as_i64().unwrap_or(0);

    let file_status = if has_file {
        format!("Downloaded ({:.1} GB)", size_gb)
    } else {
        "Missing".to_string()
    };

    format!(
        "{} ({})\nRadarr ID: {} | TMDB: {}\nStatus: {} | File: {}\nPath: {}\n\n{}",
        title, year, radarr_id, tmdb_id, status, file_status, path, overview
    )
}

fn format_series_detail(series: &Value) -> String {
    let title = series["title"].as_str().unwrap_or("?");
    let year = series["year"].as_i64().unwrap_or(0);
    let status = series["status"].as_str().unwrap_or("unknown");
    let path = series["path"].as_str().unwrap_or("?");
    let ep_count = series["statistics"]["episodeFileCount"]
        .as_i64()
        .unwrap_or(0);
    let total = series["statistics"]["episodeCount"].as_i64().unwrap_or(0);
    let size = series["statistics"]["sizeOnDisk"].as_i64().unwrap_or(0);
    let size_gb = size as f64 / 1_073_741_824.0;
    let overview = series["overview"].as_str().unwrap_or("");
    let sonarr_id = series["id"].as_i64().unwrap_or(0);
    let tvdb_id = series["tvdbId"].as_i64().unwrap_or(0);
    let season_count = series["statistics"]["seasonCount"].as_i64().unwrap_or(0);

    format!(
        "{} ({})\nSonarr ID: {} | TVDB: {}\nStatus: {} | {} season{}\n\
         Episodes: {}/{} downloaded ({:.1} GB)\nPath: {}\n\n{}",
        title,
        year,
        sonarr_id,
        tvdb_id,
        status,
        season_count,
        if season_count == 1 { "" } else { "s" },
        ep_count,
        total,
        size_gb,
        path,
        overview
    )
}

fn format_quality_profiles(profiles: &[Value], service: &str) -> String {
    if profiles.is_empty() {
        return format!("No quality profiles found in {}.", service);
    }

    let entries: Vec<String> = profiles
        .iter()
        .map(|p| {
            let id = p["id"].as_i64().unwrap_or(0);
            let name = p["name"].as_str().unwrap_or("?");
            format!("  ID: {} — {}", id, name)
        })
        .collect();

    format!("Quality profiles ({}):\n{}", service, entries.join("\n"))
}

fn format_root_folders(folders: &[Value], service: &str) -> String {
    if folders.is_empty() {
        return format!("No root folders found in {}.", service);
    }

    let entries: Vec<String> = folders
        .iter()
        .map(|f| {
            let id = f["id"].as_i64().unwrap_or(0);
            let path = f["path"].as_str().unwrap_or("?");
            let free = f["freeSpace"].as_i64().unwrap_or(0);
            let free_gb = free as f64 / 1_073_741_824.0;
            format!("  ID: {} — {} ({:.1} GB free)", id, path, free_gb)
        })
        .collect();

    format!("Root folders ({}):\n{}", service, entries.join("\n"))
}

#[cfg(test)]
mod tests;
