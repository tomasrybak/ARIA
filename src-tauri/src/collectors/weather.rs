use super::http::{self, get_json, FetchError, Throttle};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// Open-Meteo's free tier is generous (10k calls/day), and forecasts change
// hourly at best; 15 minutes keeps "now" honest without wasting calls.
const POLL: Duration = Duration::from_secs(900);
const BACKOFF_MIN: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(1800);
const HOURS_SHOWN: usize = 12;
const SEARCH_RESULTS: &str = "5";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
const GEOCODE_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";

/// One geocoding hit, and also what weather.json stores for the chosen place.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    name: String,
    /// Region and country, for telling two Springfields apart.
    #[serde(default)]
    detail: String,
    lat: f64,
    lon: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct WeatherConfig {
    place: Place,
    #[serde(default)]
    units: Units,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WeatherState {
    Disconnected { reason: String },
    Connected { place: Place, units: Units, forecast: Forecast },
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Forecast {
    current: Current,
    hourly: Vec<Hour>,
    daily: Vec<Day>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Current {
    temp: f64,
    feels_like: f64,
    humidity: f64,
    wind: f64,
    /// WMO weather interpretation code; the frontend maps it to label + icon.
    code: u8,
    is_day: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Hour {
    /// Local wall-clock time at the place, "HH:MM".
    time: String,
    temp: f64,
    code: u8,
    /// Per hour, so clear evening hours get a moon rather than a sun.
    is_day: bool,
    precip_prob: Option<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Day {
    /// "YYYY-MM-DD" at the place.
    date: String,
    code: u8,
    max: f64,
    min: f64,
    precip_prob: Option<f64>,
    sunrise: String,
    sunset: String,
}

/// Wakes the poll loop early: refresh button, or a new place/units.
static REFRESH: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tauri::command]
pub fn weather_refresh() {
    REFRESH.notify_one();
}

pub fn spawn(app: AppHandle, mut ready: tokio::sync::watch::Receiver<bool>) {
    tauri::async_runtime::spawn(async move {
        let _ = ready.wait_for(|r| *r).await;
        let client = match http::client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("weather: failed to build http client: {e}");
                return;
            }
        };
        let mut throttle = Throttle::new("weather", BACKOFF_MIN, BACKOFF_MAX);
        // Keyed by place and units, so a stale forecast never shows under a
        // newly picked city.
        let mut last_ok: Option<(Place, Units, Forecast)> = None;
        loop {
            let state = match load_config(&app) {
                Err(reason) => WeatherState::Disconnected { reason },
                Ok(cfg) => match poll(&client, &cfg, &mut throttle).await {
                    Ok(forecast) => {
                        last_ok = Some((cfg.place.clone(), cfg.units, forecast.clone()));
                        WeatherState::Connected { place: cfg.place, units: cfg.units, forecast }
                    }
                    Err(reason) => match &last_ok {
                        Some((p, u, f)) if *p == cfg.place && *u == cfg.units => {
                            WeatherState::Connected { place: cfg.place, units: cfg.units, forecast: f.clone() }
                        }
                        _ => WeatherState::Disconnected { reason },
                    },
                },
            };
            if let Err(e) = app.emit("weather", state) {
                eprintln!("weather emit failed: {e}");
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = REFRESH.notified() => {}
            }
        }
    });
}

async fn poll(
    client: &reqwest::Client,
    cfg: &WeatherConfig,
    throttle: &mut Throttle,
) -> Result<Forecast, String> {
    if throttle.active() {
        return Err("rate limited, retrying shortly".into());
    }
    let url = forecast_url(&cfg.place, cfg.units)?;
    match get_json(client, url.as_str()).await {
        Ok(v) => {
            throttle.ok();
            parse_forecast(&v)
        }
        Err(FetchError::RateLimited(after)) => {
            throttle.hit(after);
            Err("rate limited, retrying shortly".into())
        }
        Err(FetchError::Other(e)) => Err(e),
    }
}

fn forecast_url(place: &Place, units: Units) -> Result<reqwest::Url, String> {
    let mut params = vec![
        ("latitude", place.lat.to_string()),
        ("longitude", place.lon.to_string()),
        (
            "current",
            "temperature_2m,apparent_temperature,relative_humidity_2m,wind_speed_10m,weather_code,is_day".into(),
        ),
        ("hourly", "temperature_2m,weather_code,precipitation_probability,is_day".into()),
        (
            "daily",
            "weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset"
                .into(),
        ),
        ("timezone", "auto".into()),
        ("forecast_days", "7".into()),
    ];
    if units == Units::Imperial {
        params.push(("temperature_unit", "fahrenheit".into()));
        params.push(("wind_speed_unit", "mph".into()));
    }
    reqwest::Url::parse_with_params(FORECAST_URL, &params).map_err(|e| e.to_string())
}

/// The forecast payload, with hourly cut to the next HOURS_SHOWN hours from
/// the place's current local time (Open-Meteo starts hourly at midnight).
fn parse_forecast(v: &serde_json::Value) -> Result<Forecast, String> {
    let c = &v["current"];
    let num = |x: &serde_json::Value, k: &str| x[k].as_f64().ok_or_else(|| format!("missing {k}"));
    let current = Current {
        temp: num(c, "temperature_2m")?,
        feels_like: num(c, "apparent_temperature")?,
        humidity: num(c, "relative_humidity_2m")?,
        wind: num(c, "wind_speed_10m")?,
        code: c["weather_code"].as_u64().unwrap_or(0) as u8,
        is_day: c["is_day"].as_u64() != Some(0),
    };
    let now = c["time"].as_str().ok_or("missing current time")?;
    // "2026-10-22T14:15" → the hour slot "2026-10-22T14:00"; ISO strings
    // compare correctly as plain strings.
    let hour_slot = format!("{}:00", now.get(..13).ok_or("bad current time")?);

    let h = &v["hourly"];
    let times = h["time"].as_array().ok_or("missing hourly")?;
    let first = times
        .iter()
        .position(|t| t.as_str().is_some_and(|t| t >= hour_slot.as_str()))
        .unwrap_or(times.len());
    let hourly = (first..times.len().min(first + HOURS_SHOWN))
        .filter_map(|i| {
            Some(Hour {
                time: times[i].as_str()?.get(11..16)?.to_string(),
                temp: h["temperature_2m"][i].as_f64()?,
                code: h["weather_code"][i].as_u64()? as u8,
                is_day: h["is_day"][i].as_u64() != Some(0),
                precip_prob: h["precipitation_probability"][i].as_f64(),
            })
        })
        .collect();

    let d = &v["daily"];
    let dates = d["time"].as_array().ok_or("missing daily")?;
    let daily = (0..dates.len())
        .filter_map(|i| {
            Some(Day {
                date: dates[i].as_str()?.to_string(),
                code: d["weather_code"][i].as_u64()? as u8,
                max: d["temperature_2m_max"][i].as_f64()?,
                min: d["temperature_2m_min"][i].as_f64()?,
                precip_prob: d["precipitation_probability_max"][i].as_f64(),
                sunrise: d["sunrise"][i].as_str().and_then(|s| s.get(11..16)).unwrap_or("").into(),
                sunset: d["sunset"][i].as_str().and_then(|s| s.get(11..16)).unwrap_or("").into(),
            })
        })
        .collect();

    Ok(Forecast { current, hourly, daily })
}

fn parse_places(v: &serde_json::Value) -> Vec<Place> {
    let Some(results) = v["results"].as_array() else {
        return Vec::new(); // Open-Meteo omits `results` when nothing matched
    };
    results
        .iter()
        .filter_map(|r| {
            let detail = [r["admin1"].as_str(), r["country"].as_str()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
            Some(Place {
                name: r["name"].as_str()?.to_string(),
                detail,
                lat: r["latitude"].as_f64()?,
                lon: r["longitude"].as_f64()?,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn weather_search(query: String) -> Result<Vec<Place>, String> {
    let query = query.trim();
    if query.chars().count() < 2 {
        return Err("type at least two letters".into());
    }
    let url = reqwest::Url::parse_with_params(
        GEOCODE_URL,
        &[("name", query), ("count", SEARCH_RESULTS), ("language", "en"), ("format", "json")],
    )
    .map_err(|e| e.to_string())?;
    let client = http::client()?;
    match get_json(&client, url.as_str()).await {
        Ok(v) => Ok(parse_places(&v)),
        Err(FetchError::RateLimited(_)) => Err("search is rate limited, try again in a minute".into()),
        Err(FetchError::Other(e)) => Err(e),
    }
}

#[tauri::command]
pub fn weather_set_location(app: AppHandle, place: Place, units: Units) -> Result<(), String> {
    if !(-90.0..=90.0).contains(&place.lat) || !(-180.0..=180.0).contains(&place.lon) {
        return Err("invalid coordinates".into());
    }
    save_config(&app, &WeatherConfig { place, units })?;
    REFRESH.notify_one();
    Ok(())
}

#[tauri::command]
pub fn weather_clear(app: AppHandle) -> Result<(), String> {
    let path = config_path(&app)?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    REFRESH.notify_one();
    Ok(())
}

/// The saved place, for the gear panel's units toggle.
#[tauri::command]
pub fn weather_config(app: AppHandle) -> Option<(Place, Units)> {
    load_config(&app).ok().map(|c| (c.place, c.units))
}

fn config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("weather.json"))
}

/// No file is a clean "not configured"; the widget shows its search box.
fn load_config(app: &AppHandle) -> Result<WeatherConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Err("no location".into());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("weather.json is invalid: {e}"))
}

fn save_config(app: &AppHandle, cfg: &WeatherConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(app)?, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../tests/fixtures/open-meteo-forecast.json")).unwrap()
    }

    #[test]
    fn current_conditions_parse() {
        let f = parse_forecast(&fixture()).unwrap();
        assert_eq!(f.current.code, 3);
        assert!(f.current.is_day);
        assert_eq!(f.current.temp, 11.4);
    }

    #[test]
    fn hourly_starts_at_the_current_hour_at_the_place() {
        // current.time is 14:15 local, hourly starts at 00:00.
        let f = parse_forecast(&fixture()).unwrap();
        assert_eq!(f.hourly.first().unwrap().time, "14:00");
        assert_eq!(f.hourly.len(), HOURS_SHOWN);
    }

    #[test]
    fn hourly_near_the_end_of_the_range_is_short_not_an_error() {
        let mut v = fixture();
        v["current"]["time"] = "2026-10-28T20:00".into();
        let f = parse_forecast(&v).unwrap();
        assert_eq!(f.hourly.len(), 4); // 20..23 of the last day
    }

    #[test]
    fn hours_after_sunset_are_night() {
        let f = parse_forecast(&fixture()).unwrap();
        let at = |t: &str| f.hourly.iter().find(|h| h.time == t).unwrap().is_day;
        assert!(at("15:00"));
        assert!(!at("21:00"));
    }

    #[test]
    fn daily_has_seven_days_with_sun_times() {
        let f = parse_forecast(&fixture()).unwrap();
        assert_eq!(f.daily.len(), 7);
        assert_eq!(f.daily[0].sunrise, "07:12");
    }

    #[test]
    fn missing_current_is_an_error() {
        let mut v = fixture();
        v["current"] = serde_json::Value::Null;
        assert!(parse_forecast(&v).is_err());
    }

    #[test]
    fn places_carry_region_and_country() {
        let v = serde_json::json!({ "results": [
            { "name": "Plzeň", "admin1": "Plzeňský kraj", "country": "Czechia",
              "latitude": 49.74747, "longitude": 13.37759 },
            { "name": "Nowhere", "latitude": 1.0, "longitude": 2.0 }
        ]});
        let p = parse_places(&v);
        assert_eq!(p[0].detail, "Plzeňský kraj, Czechia");
        assert_eq!(p[1].detail, "");
    }

    #[test]
    fn no_match_is_an_empty_list() {
        assert!(parse_places(&serde_json::json!({ "generationtime_ms": 0.5 })).is_empty());
    }

    #[test]
    fn imperial_asks_for_fahrenheit_and_mph() {
        let p = Place { name: "x".into(), detail: String::new(), lat: 1.0, lon: 2.0 };
        let url = forecast_url(&p, Units::Imperial).unwrap();
        assert!(url.as_str().contains("temperature_unit=fahrenheit"));
        assert!(!forecast_url(&p, Units::Metric).unwrap().as_str().contains("fahrenheit"));
    }
}
