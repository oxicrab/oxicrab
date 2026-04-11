use anyhow::Result;
use async_trait::async_trait;
use oxicrab_core::actions;
use oxicrab_core::tools::base::{
    ExecutionContext, SubagentAccess, ToolCapabilities, ToolCategory, ToolConcurrency,
};
use oxicrab_core::tools::base::{Tool, ToolResult};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const OWM_API: &str = "https://api.openweathermap.org/data/2.5";

struct DaySummary {
    high: f64,
    low: f64,
    max_pop: f64,
    conditions: Vec<String>,
}

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
                ("cnt", "40"), // 5 days (3h intervals, max for free tier)
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

        // Aggregate 3-hour intervals into daily summaries
        let mut days: std::collections::BTreeMap<String, DaySummary> =
            std::collections::BTreeMap::new();

        for entry in list {
            let dt_txt = entry["dt_txt"].as_str().unwrap_or("");
            let date = dt_txt.split(' ').next().unwrap_or("?").to_string();
            let temp = entry["main"]["temp"].as_f64().unwrap_or_default();
            let desc = entry["weather"][0]["description"]
                .as_str()
                .unwrap_or("?")
                .to_string();
            let pop = entry["pop"].as_f64().unwrap_or_default();

            let day = days.entry(date).or_insert_with(|| DaySummary {
                high: f64::NEG_INFINITY,
                low: f64::INFINITY,
                max_pop: 0.0,
                conditions: Vec::new(),
            });
            if temp > day.high {
                day.high = temp;
            }
            if temp < day.low {
                day.low = temp;
            }
            if pop > day.max_pop {
                day.max_pop = pop;
            }
            if !day.conditions.contains(&desc) {
                day.conditions.push(desc);
            }
        }

        let lines: Vec<String> = days
            .iter()
            .map(|(date, day)| {
                let conditions = day.conditions.join(", ");
                let rain = day.max_pop * 100.0;
                format!(
                    "{date}: High {:.0}{unit_label} / Low {:.0}{unit_label} — {conditions} (rain: {rain:.0}%)",
                    day.high, day.low
                )
            })
            .collect();

        Ok(format!(
            "5-day forecast for {city}, {country}:\n{}",
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
        "Get current weather or 5-day daily forecast for a location. Uses OpenWeatherMap."
    }

    fn cacheable(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities {
            built_in: true,
            network_outbound: true,
            subagent_access: SubagentAccess::ReadOnly,
            category: ToolCategory::Web,
            actions: actions![current: ro, forecast: ro],
            concurrency: ToolConcurrency::ReadOnly,
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
                    "description": "current = now, forecast = 5-day daily with high/low temps and conditions"
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
