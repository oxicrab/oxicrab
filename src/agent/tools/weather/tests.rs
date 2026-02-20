use super::*;
use crate::agent::tools::base::ExecutionContext;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tool() -> WeatherTool {
    WeatherTool::new("fake_key".to_string())
}

#[tokio::test]
async fn test_missing_location() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "current"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("location"));
}

#[tokio::test]
async fn test_unknown_action() {
    let result = tool()
        .execute(
            serde_json::json!({"action": "bogus", "location": "NYC"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("unknown action"));
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
        .execute(
            serde_json::json!({"location": "London", "units": "metric"}),
            &ExecutionContext::default(),
        )
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
        .execute(
            serde_json::json!({"location": "New York", "units": "imperial"}),
            &ExecutionContext::default(),
        )
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
            &ExecutionContext::default(),
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
        .execute(
            serde_json::json!({"location": "Nonexistentville"}),
            &ExecutionContext::default(),
        )
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
        .execute(
            serde_json::json!({"location": "London"}),
            &ExecutionContext::default(),
        )
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
        .execute(
            serde_json::json!({"location": "Tokyo"}),
            &ExecutionContext::default(),
        )
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
        .execute(
            serde_json::json!({"location": "SF"}),
            &ExecutionContext::default(),
        )
        .await
        .unwrap();

    assert!(!result.is_error);
    assert!(result.content.contains("°F"));
}
