use crate::agent::tools::{Tool, ToolResult, ToolVersion};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const OWM_API: &str = "https://api.openweathermap.org/data/2.5";

pub struct WeatherTool {
    api_key: String,
    client: Client,
}

impl WeatherTool {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
        }
    }

    async fn current(&self, location: &str, units: &str) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/weather", OWM_API))
            .query(&[("q", location), ("appid", &self.api_key), ("units", units)])
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;
        if !status.is_success() {
            let msg = json["message"].as_str().unwrap_or("Unknown error");
            anyhow::bail!("OpenWeatherMap: {}", msg);
        }

        let temp = json["main"]["temp"].as_f64().unwrap_or(0.0);
        let feels_like = json["main"]["feels_like"].as_f64().unwrap_or(0.0);
        let humidity = json["main"]["humidity"].as_u64().unwrap_or(0);
        let description = json["weather"][0]["description"]
            .as_str()
            .unwrap_or("unknown");
        let wind_speed = json["wind"]["speed"].as_f64().unwrap_or(0.0);
        let city = json["name"].as_str().unwrap_or(location);
        let country = json["sys"]["country"].as_str().unwrap_or("");

        let unit_label = match units {
            "imperial" => "°F",
            "metric" => "°C",
            _ => "K",
        };
        let wind_unit = if units == "imperial" { "mph" } else { "m/s" };

        Ok(format!(
            "Weather in {}, {}:\n{} | {:.0}{} (feels like {:.0}{})\nHumidity: {}% | Wind: {:.1} {}",
            city,
            country,
            description,
            temp,
            unit_label,
            feels_like,
            unit_label,
            humidity,
            wind_speed,
            wind_unit
        ))
    }

    async fn forecast(&self, location: &str, units: &str) -> Result<String> {
        let resp = self
            .client
            .get(format!("{}/forecast", OWM_API))
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
            anyhow::bail!("OpenWeatherMap: {}", msg);
        }

        let city = json["city"]["name"].as_str().unwrap_or(location);
        let country = json["city"]["country"].as_str().unwrap_or("");
        let list = json["list"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);

        let unit_label = match units {
            "imperial" => "°F",
            "metric" => "°C",
            _ => "K",
        };

        let lines: Vec<String> = list
            .iter()
            .map(|entry| {
                let dt_txt = entry["dt_txt"].as_str().unwrap_or("?");
                let temp = entry["main"]["temp"].as_f64().unwrap_or(0.0);
                let desc = entry["weather"][0]["description"].as_str().unwrap_or("?");
                let pop = entry["pop"].as_f64().unwrap_or(0.0) * 100.0;
                format!(
                    "{}: {:.0}{} {} (rain: {:.0}%)",
                    dt_txt, temp, unit_label, desc, pop
                )
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
    fn name(&self) -> &str {
        "weather"
    }

    fn description(&self) -> &str {
        "Get current weather or forecast for a location. Uses OpenWeatherMap."
    }

    fn version(&self) -> ToolVersion {
        ToolVersion::new(1, 0, 0)
    }

    fn cacheable(&self) -> bool {
        true
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

    async fn execute(&self, params: Value) -> Result<ToolResult> {
        let location = match params["location"].as_str() {
            Some(l) => l,
            None => {
                return Ok(ToolResult::error(
                    "Missing 'location' parameter".to_string(),
                ))
            }
        };

        let action = params["action"].as_str().unwrap_or("current");
        let units = params["units"].as_str().unwrap_or("imperial");

        let result = match action {
            "current" => self.current(location, units).await,
            "forecast" => self.forecast(location, units).await,
            _ => return Ok(ToolResult::error(format!("Unknown action: {}", action))),
        };

        match result {
            Ok(content) => Ok(ToolResult::new(content)),
            Err(e) => Ok(ToolResult::error(format!("Weather error: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> WeatherTool {
        WeatherTool::new("fake_key".to_string())
    }

    #[tokio::test]
    async fn test_missing_location() {
        let result = tool()
            .execute(serde_json::json!({"action": "current"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("location"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let result = tool()
            .execute(serde_json::json!({"action": "bogus", "location": "NYC"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Unknown action"));
    }
}
