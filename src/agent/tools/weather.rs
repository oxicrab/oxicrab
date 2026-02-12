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
            client: Client::new(),
        }
    }

    #[cfg(test)]
    fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            base_url,
            client: Client::new(),
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
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    // --- Wiremock tests ---

    #[tokio::test]
    async fn test_current_weather_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .and(query_param("q", "London"))
            .and(query_param("appid", "test_key"))
            .and(query_param("units", "metric"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "main": {"temp": 15.0, "feels_like": 13.5, "humidity": 72},
                "weather": [{"description": "light rain"}],
                "wind": {"speed": 5.2},
                "name": "London",
                "sys": {"country": "GB"}
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"location": "London", "units": "metric"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("London"));
        assert!(result.content.contains("GB"));
        assert!(result.content.contains("light rain"));
        assert!(result.content.contains("15°C"));
        assert!(result.content.contains("72%"));
    }

    #[tokio::test]
    async fn test_current_weather_imperial_units() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .and(query_param("units", "imperial"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "main": {"temp": 72.0, "feels_like": 70.0, "humidity": 50},
                "weather": [{"description": "clear sky"}],
                "wind": {"speed": 8.0},
                "name": "New York",
                "sys": {"country": "US"}
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"location": "New York", "units": "imperial"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("72°F"));
        assert!(result.content.contains("mph"));
    }

    #[tokio::test]
    async fn test_forecast_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/forecast"))
            .and(query_param("q", "Paris"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "city": {"name": "Paris", "country": "FR"},
                "list": [
                    {
                        "dt_txt": "2026-02-11 12:00:00",
                        "main": {"temp": 8.0},
                        "weather": [{"description": "overcast clouds"}],
                        "pop": 0.3
                    },
                    {
                        "dt_txt": "2026-02-11 15:00:00",
                        "main": {"temp": 7.0},
                        "weather": [{"description": "light rain"}],
                        "pop": 0.8
                    }
                ]
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        let result = tool
            .execute(
                serde_json::json!({"action": "forecast", "location": "Paris", "units": "metric"}),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Paris"));
        assert!(result.content.contains("FR"));
        assert!(result.content.contains("overcast clouds"));
        assert!(result.content.contains("light rain"));
        assert!(result.content.contains("rain: 80%"));
    }

    #[tokio::test]
    async fn test_api_error_city_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
                "cod": "404",
                "message": "city not found"
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"location": "Nonexistentville"}))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("city not found"));
    }

    #[tokio::test]
    async fn test_api_error_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "cod": 401,
                "message": "Invalid API key"
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("bad_key".to_string(), server.uri());
        let result = tool
            .execute(serde_json::json!({"location": "London"}))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("Invalid API key"));
    }

    #[tokio::test]
    async fn test_default_action_is_current() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "main": {"temp": 20.0, "feels_like": 19.0, "humidity": 60},
                "weather": [{"description": "sunny"}],
                "wind": {"speed": 3.0},
                "name": "Tokyo",
                "sys": {"country": "JP"}
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        // No action specified — should default to "current"
        let result = tool
            .execute(serde_json::json!({"location": "Tokyo"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("Tokyo"));
    }

    #[tokio::test]
    async fn test_default_units_is_imperial() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/weather"))
            .and(query_param("units", "imperial"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "main": {"temp": 68.0, "feels_like": 66.0, "humidity": 55},
                "weather": [{"description": "clear"}],
                "wind": {"speed": 4.0},
                "name": "SF",
                "sys": {"country": "US"}
            })))
            .mount(&server)
            .await;

        let tool = WeatherTool::with_base_url("test_key".to_string(), server.uri());
        // No units specified — should default to "imperial"
        let result = tool
            .execute(serde_json::json!({"location": "SF"}))
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.content.contains("°F"));
    }
}
