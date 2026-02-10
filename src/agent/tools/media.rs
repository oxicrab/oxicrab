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
            client: Client::new(),
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
        let arr = results.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        Ok(format_movie_search_results(arr))
    }

    async fn add_movie(
        &self,
        tmdb_id: i64,
        quality_profile_id: Option<i64>,
        root_folder: Option<&str>,
    ) -> Result<String> {
        // Look up movie details first
        let lookup = self
            .radarr_get(&format!("/api/v3/movie/lookup/tmdb?tmdbId={}", tmdb_id))
            .await?;

        let title = lookup["title"].as_str().unwrap_or("Unknown");
        let year = lookup["year"].as_i64().unwrap_or(0);

        // Auto-fetch quality profile if not specified
        let profile_id = match quality_profile_id {
            Some(id) => id,
            None => {
                let profiles = self.radarr_get("/api/v3/qualityprofile").await?;
                profiles
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|p| p["id"].as_i64())
                    .ok_or_else(|| anyhow::anyhow!("No quality profiles found in Radarr"))?
            }
        };

        // Auto-fetch root folder if not specified
        let folder = match root_folder {
            Some(f) => f.to_string(),
            None => {
                let folders = self.radarr_get("/api/v3/rootfolder").await?;
                folders
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|f| f["path"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("No root folders found in Radarr"))?
                    .to_string()
            }
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
        let arr = movies.as_array().map(|a| a.as_slice()).unwrap_or(&[]);

        let filtered: Vec<&Value> = if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            arr.iter()
                .filter(|m| {
                    m["title"]
                        .as_str()
                        .map(|t| t.to_lowercase().contains(&f_lower))
                        .unwrap_or(false)
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
        let arr = results.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        Ok(format_series_search_results(arr))
    }

    async fn add_series(
        &self,
        tvdb_id: i64,
        quality_profile_id: Option<i64>,
        root_folder: Option<&str>,
    ) -> Result<String> {
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
        let profile_id = match quality_profile_id {
            Some(id) => id,
            None => {
                let profiles = self.sonarr_get("/api/v3/qualityprofile").await?;
                profiles
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|p| p["id"].as_i64())
                    .ok_or_else(|| anyhow::anyhow!("No quality profiles found in Sonarr"))?
            }
        };

        // Auto-fetch root folder if not specified
        let folder = match root_folder {
            Some(f) => f.to_string(),
            None => {
                let folders = self.sonarr_get("/api/v3/rootfolder").await?;
                folders
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|f| f["path"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("No root folders found in Sonarr"))?
                    .to_string()
            }
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
        let arr = series.as_array().map(|a| a.as_slice()).unwrap_or(&[]);

        let filtered: Vec<&Value> = if let Some(f) = filter {
            let f_lower = f.to_lowercase();
            arr.iter()
                .filter(|s| {
                    s["title"]
                        .as_str()
                        .map(|t| t.to_lowercase().contains(&f_lower))
                        .unwrap_or(false)
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
        let arr = profiles.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        Ok(format_quality_profiles(arr, service))
    }

    async fn root_folders(&self, service: &str) -> Result<String> {
        let folders = match service {
            "radarr" => self.radarr_get("/api/v3/rootfolder").await?,
            "sonarr" => self.sonarr_get("/api/v3/rootfolder").await?,
            _ => anyhow::bail!("Unknown service '{}'. Use 'radarr' or 'sonarr'.", service),
        };
        let arr = folders.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
        Ok(format_root_folders(arr, service))
    }
}

#[async_trait]
impl Tool for MediaTool {
    fn name(&self) -> &str {
        "media"
    }

    fn description(&self) -> &str {
        "Manage movies and TV series via Radarr (movies) and Sonarr (TV). \
         Actions: search_movie, add_movie (by TMDB ID), get_movie, list_movies, \
         search_series, add_series (by TVDB ID), get_series, list_series, \
         profiles (list quality profiles), root_folders (list root folders). \
         For add_movie/add_series, quality profile and root folder are auto-selected if omitted."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let action = match params["action"].as_str() {
            Some(a) => a,
            None => return Ok(ToolResult::error("Missing 'action' parameter".to_string())),
        };

        let result = match action {
            "search_movie" => {
                let query = match params["query"].as_str() {
                    Some(q) => q,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'query' parameter for search_movie".to_string(),
                        ))
                    }
                };
                self.search_movie(query).await
            }
            "add_movie" => {
                let tmdb_id = match params["tmdb_id"].as_i64() {
                    Some(id) => id,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'tmdb_id' parameter for add_movie".to_string(),
                        ))
                    }
                };
                let qp = params["quality_profile_id"].as_i64();
                let rf = params["root_folder"].as_str();
                self.add_movie(tmdb_id, qp, rf).await
            }
            "get_movie" => {
                let id = match params["id"].as_i64() {
                    Some(id) => id,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'id' parameter for get_movie".to_string(),
                        ))
                    }
                };
                self.get_movie(id).await
            }
            "list_movies" => {
                let filter = params["filter"].as_str();
                self.list_movies(filter).await
            }
            "search_series" => {
                let query = match params["query"].as_str() {
                    Some(q) => q,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'query' parameter for search_series".to_string(),
                        ))
                    }
                };
                self.search_series(query).await
            }
            "add_series" => {
                let tvdb_id = match params["tvdb_id"].as_i64() {
                    Some(id) => id,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'tvdb_id' parameter for add_series".to_string(),
                        ))
                    }
                };
                let qp = params["quality_profile_id"].as_i64();
                let rf = params["root_folder"].as_str();
                self.add_series(tvdb_id, qp, rf).await
            }
            "get_series" => {
                let id = match params["id"].as_i64() {
                    Some(id) => id,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'id' parameter for get_series".to_string(),
                        ))
                    }
                };
                self.get_series(id).await
            }
            "list_series" => {
                let filter = params["filter"].as_str();
                self.list_series(filter).await
            }
            "profiles" => {
                let service = match params["service"].as_str() {
                    Some(s) => s,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'service' parameter for profiles".to_string(),
                        ))
                    }
                };
                self.profiles(service).await
            }
            "root_folders" => {
                let service = match params["service"].as_str() {
                    Some(s) => s,
                    None => {
                        return Ok(ToolResult::error(
                            "Missing 'service' parameter for root_folders".to_string(),
                        ))
                    }
                };
                self.root_folders(service).await
            }
            _ => return Ok(ToolResult::error(format!("Unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Media error: {}", e))),
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
            let overview_short = if overview.len() > 150 {
                // Find a safe char boundary
                let end = overview
                    .char_indices()
                    .take_while(|(idx, _)| *idx <= 150)
                    .last()
                    .map(|(idx, _)| idx)
                    .unwrap_or(150);
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
            let overview_short = if overview.len() > 150 {
                let end = overview
                    .char_indices()
                    .take_while(|(idx, _)| *idx <= 150)
                    .last()
                    .map(|(idx, _)| idx)
                    .unwrap_or(150);
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
mod tests {
    use super::*;

    #[test]
    fn test_format_movie_search_results() {
        let results = vec![
            serde_json::json!({
                "title": "Interstellar",
                "year": 2014,
                "tmdbId": 157336,
                "overview": "A team of explorers travel through a wormhole in space.",
                "id": 0
            }),
            serde_json::json!({
                "title": "Interstellar Wars",
                "year": 2016,
                "tmdbId": 390876,
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
            "tmdbId": 335984,
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
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let config = MediaConfig::default();
        let tool = MediaTool::new(&config);
        let result = tool
            .execute(serde_json::json!({"action": "bogus"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_search_movie_missing_query() {
        let config = MediaConfig::default();
        let tool = MediaTool::new(&config);
        let result = tool
            .execute(serde_json::json!({"action": "search_movie"}))
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
            .execute(serde_json::json!({"action": "add_movie"}))
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
            .execute(serde_json::json!({"action": "add_series"}))
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
            .execute(serde_json::json!({"action": "profiles"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("service"));
    }

    #[tokio::test]
    async fn test_movie_search_truncates_long_overview() {
        let long_overview = "A".repeat(300);
        let results = vec![serde_json::json!({
            "title": "Test",
            "year": 2020,
            "tmdbId": 1,
            "overview": long_overview,
            "id": 0
        })];
        let output = format_movie_search_results(&results);
        assert!(output.contains("..."));
        // The overview should be truncated, not full 300 chars
        assert!(!output.contains(&long_overview));
    }
}
