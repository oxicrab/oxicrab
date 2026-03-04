use crate::agent::tools::base::{ExecutionContext, SubagentAccess, ToolCapabilities};
use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const OWM_API: &str = "https://api.openweathermap.org/data/2.5";

pub struct WeatherTool {
    api_key: String,
    base_url: String,
    client: Client,
}

impl WeatherTool {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: OWM_API.to_string(),
            client: crate::utils::http::default_http_client(),
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: crate::utils::http::default_http_client(),
        }
    }

    async fn current(&self, location: &str, units: &str) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/weather", self.base_url))
            .query(&[("q", location), ("appid", &self.api_key), ("units", units)])
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("OpenWeatherMap: {msg}");
        }

        let temp = json["main"]["temp"].as_f64().unwrap_or_default();
        let feels_like = json["main"]["feels_like"].as_f64().unwrap_or_default();
        let humidity = json["main"]["humidity"].as_u64().unwrap_or(0);
        let description = json["weather"][0]["description"]
            .as_str()
            .unwrap_or("unknown");
        let wind_speed = json["wind"]["speed"].as_f64().unwrap_or_default();
        let city = json["name"].as_str().unwrap_or(location);
        let country = json["sys"]["country"].as_str().unwrap_or_default();

        let unit_label = match units {
            "imperial" => "°F",
            "metric" => "°C",
            _ => "K",
        };
        let wind_unit = if units == "imperial" { "mph" } else { "m/s" };

        Ok(format!(
            "Weather in {city}, {country}:\n{description} | {temp:.0}{unit_label} (feels like {feels_like:.0}{unit_label})\nHumidity: {humidity}% | Wind: {wind_speed:.1} {wind_unit}"
        ))
    }

    async fn forecast(&self, location: &str, units: &str) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/forecast", self.base_url))
            .query(&[
                ("q", location),
                ("appid", &self.api_key),
                ("units", units),
                ("cnt", "8"), // 24 hours (3h intervals)
            ])
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("OpenWeatherMap: {msg}");
        }

        let city = json["city"]["name"].as_str().unwrap_or(location);
        let country = json["city"]["country"].as_str().unwrap_or_default();
        let list = json["list"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();

        let unit_label = match units {
            "imperial" => "°F",
            "metric" => "°C",
            _ => "K",
        };

        let lines: Vec<String> = list
            .iter()
            .map(|entry| {
                let dt_txt = entry["dt_txt"].as_str().unwrap_or("?");
                let temp = entry["main"]["temp"].as_f64().unwrap_or_default();
                let desc = entry["weather"][0]["description"].as_str().unwrap_or("?");
                let pop = entry["pop"].as_f64().unwrap_or_default() * 100.0;
                format!("{dt_txt}: {temp:.0}{unit_label} {desc} (rain: {pop:.0}%)")
            })
            .collect();

        Ok(format!(
            "24h forecast for {}, {}:\n{}",
            city,
            country,
            lines.join("\n")
        ))
    }
}

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &'static str {
        "weather"
    }

    fn description(&self) -> &'static str {
        "Get current weather or forecast for a location. Uses OpenWeatherMap."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::Full,
            ..Default::default()
        }
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["current", "forecast"],
                    "default": "current",
                    "description": "Get current weather or 24h forecast"
                },
                "location": {
                    "type": "string",
                    "description": "City name, optionally with country code (e.g. 'New York,US' or 'London')"
                },
                "units": {
                    "type": "string",
                    "enum": ["imperial", "metric"],
                    "default": "imperial",
                    "description": "Temperature units"
                }
            },
            "required": ["location"]
        })
    }

    async fn execute(&self, params: Value, _ctx: &ExecutionContext) -> Result<ToolResult> {
        let Some(location) = params["location"].as_str() else {
            return Ok(ToolResult::error(
                "missing 'location' parameter".to_string(),
            ));
        };

        let action = params["action"].as_str().unwrap_or("current");
        let units = params["units"].as_str().unwrap_or("imperial");

        let result = match action {
            "current" => self.current(location, units).await,
            "forecast" => self.forecast(location, units).await,
            _ => return Ok(ToolResult::error(format!("unknown action: {action}"))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("weather error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests;
