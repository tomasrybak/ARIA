use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

// CoinGecko's keyless tier allows only a few calls per minute per IP. Each
// cycle sends one markets request plus at most CHART_BUDGET chart requests,
// spaced by CHART_GAP. A 429 aborts the cycle and pauses all requests (see
// Throttle); the last good payload is re-emitted meanwhile.
const POLL: Duration = Duration::from_secs(120);
const CHART_BUDGET: usize = 2;
const CHART_GAP: Duration = Duration::from_secs(3);
const BACKOFF_MIN: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(900);
const COINS: [&str; 2] = ["bitcoin", "ethereum"];
const MARKETS_URL: &str = "https://api.coingecko.com/api/v3/coins/markets\
                           ?vs_currency=usd&ids=bitcoin,ethereum\
                           &price_change_percentage=24h,30d,1y";
// Points kept per chart; enough for a mini sparkline.
const SPARK_POINTS: usize = 42;
// Trailing daily points of the year series that make up the month chart.
const MONTH_DAYS: usize = 31;

/// One market_chart request per coin and range. The year series (daily
/// granularity) also feeds the month chart, so a coin needs two requests,
/// not three. Order is refresh priority: the widget defaults to the month view.
#[derive(Clone, Copy)]
enum Range {
    Year,
    Day,
}

impl Range {
    const ALL: [Range; 2] = [Range::Year, Range::Day];

    fn days(self) -> &'static str {
        match self {
            Range::Year => "365",
            Range::Day => "1",
        }
    }

    fn ttl(self) -> Duration {
        match self {
            Range::Year => Duration::from_secs(3600),
            Range::Day => Duration::from_secs(600),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CryptoState {
    Disconnected { reason: String },
    Connected { coins: Vec<Coin> },
}

#[derive(Serialize, Clone, Default)]
struct Coin {
    symbol: String,
    price: f64,
    change_1d: f64,
    change_1m: f64,
    change_1y: f64,
    spark_1d: Vec<f64>,
    spark_1m: Vec<f64>,
    spark_1y: Vec<f64>,
    market_cap: f64,
    volume_24h: f64,
    high_24h: f64,
    low_24h: f64,
}

/// Raw price series of one coin, indexed by Range.
#[derive(Default)]
struct Series {
    prices: Vec<f64>,
    fetched: Option<Instant>,
}

enum FetchError {
    RateLimited(Option<Duration>),
    Other(String),
}

/// Global pause after a 429. Honors Retry-After when sent, otherwise
/// doubles the pause on each consecutive 429 up to BACKOFF_MAX.
struct Throttle {
    until: Option<Instant>,
    backoff: Duration,
}

impl Throttle {
    fn active(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    fn hit(&mut self, retry_after: Option<Duration>) {
        let wait = retry_after.unwrap_or(self.backoff).clamp(BACKOFF_MIN, BACKOFF_MAX);
        self.until = Some(Instant::now() + wait);
        self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
        eprintln!("crypto: rate limited by CoinGecko, pausing {}s", wait.as_secs());
    }

    fn ok(&mut self) {
        self.backoff = BACKOFF_MIN;
    }
}

pub fn spawn(app: AppHandle, mut ready: tokio::sync::watch::Receiver<bool>) {
    tauri::async_runtime::spawn(async move {
        let _ = ready.wait_for(|r| *r).await;
        let client = match reqwest::Client::builder().user_agent("aria-hud").build() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("crypto: failed to build http client: {e}");
                return;
            }
        };
        let mut series: Vec<[Series; 2]> = COINS.iter().map(|_| Default::default()).collect();
        let mut throttle = Throttle { until: None, backoff: BACKOFF_MIN };
        let mut last_ok: Option<Vec<Coin>> = None;
        loop {
            let state = match poll(&client, &mut series, &mut throttle).await {
                Ok(coins) => {
                    last_ok = Some(coins.clone());
                    CryptoState::Connected { coins }
                }
                // Stale prices beat an error box; fall back to the last
                // good payload when the API throttles us.
                Err(reason) => match &last_ok {
                    Some(coins) => CryptoState::Connected { coins: coins.clone() },
                    None => CryptoState::Disconnected { reason },
                },
            };
            if let Err(e) = app.emit("crypto", state) {
                eprintln!("crypto emit failed: {e}");
            }
            // A refresh wake only skips the rest of the sleep; an active
            // throttle still blocks the network inside poll().
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = REFRESH.notified() => {}
            }
        }
    });
}

/// Wakes the poll loop early (widget refresh button).
static REFRESH: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tauri::command]
pub fn crypto_refresh() {
    REFRESH.notify_one();
}

async fn poll(
    client: &reqwest::Client,
    series: &mut [[Series; 2]],
    throttle: &mut Throttle,
) -> Result<Vec<Coin>, String> {
    if throttle.active() {
        return Err("rate limited, retrying shortly".into());
    }
    let markets = match get(client, MARKETS_URL).await {
        Ok(v) => v,
        Err(FetchError::RateLimited(after)) => {
            throttle.hit(after);
            return Err("rate limited, retrying shortly".into());
        }
        Err(FetchError::Other(e)) => return Err(e),
    };
    let markets = markets
        .as_array()
        .ok_or_else(|| "unexpected response shape".to_string())?;
    throttle.ok();

    refresh_charts(client, series, throttle).await;

    let mut coins = Vec::new();
    for (ci, id) in COINS.iter().enumerate() {
        let Some(m) = markets.iter().find(|m| m["id"] == *id) else {
            continue;
        };
        let year = &series[ci][Range::Year as usize].prices;
        let month = &year[year.len().saturating_sub(MONTH_DAYS)..];
        coins.push(Coin {
            symbol: m["symbol"].as_str().unwrap_or(id).to_uppercase(),
            price: m["current_price"].as_f64().unwrap_or(0.0),
            change_1d: pct(m, "price_change_percentage_24h_in_currency"),
            change_1m: pct(m, "price_change_percentage_30d_in_currency"),
            change_1y: pct(m, "price_change_percentage_1y_in_currency"),
            spark_1d: downsample(&series[ci][Range::Day as usize].prices),
            spark_1m: downsample(month),
            spark_1y: downsample(year),
            market_cap: m["market_cap"].as_f64().unwrap_or(0.0),
            volume_24h: m["total_volume"].as_f64().unwrap_or(0.0),
            high_24h: m["high_24h"].as_f64().unwrap_or(0.0),
            low_24h: m["low_24h"].as_f64().unwrap_or(0.0),
        });
    }

    if coins.is_empty() {
        return Err("no market data returned".into());
    }
    Ok(coins)
}

/// Refresh up to CHART_BUDGET stale series in Range priority order. A 429
/// stops the cycle immediately; other failures still consume budget.
async fn refresh_charts(
    client: &reqwest::Client,
    series: &mut [[Series; 2]],
    throttle: &mut Throttle,
) {
    let mut budget = CHART_BUDGET;
    for range in Range::ALL {
        for (ci, id) in COINS.iter().enumerate() {
            if budget == 0 {
                return;
            }
            let s = &mut series[ci][range as usize];
            if s.fetched.is_some_and(|t| t.elapsed() < range.ttl()) {
                continue;
            }
            tokio::time::sleep(CHART_GAP).await;
            let url = format!(
                "https://api.coingecko.com/api/v3/coins/{id}/market_chart\
                 ?vs_currency=usd&days={}",
                range.days()
            );
            match get(client, &url).await {
                Ok(chart) => {
                    s.prices = chart["prices"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|p| p[1].as_f64()).collect())
                        .unwrap_or_default();
                    s.fetched = Some(Instant::now());
                }
                Err(FetchError::RateLimited(after)) => {
                    throttle.hit(after);
                    return;
                }
                Err(FetchError::Other(e)) => {
                    eprintln!("crypto chart {id}/{}d: {e}", range.days())
                }
            }
            budget -= 1;
        }
    }
}

/// Only the 24h key has a legitimate fallback (the non-currency variant);
/// falling back across periods would mislabel a day change as month/year.
fn pct(m: &serde_json::Value, key: &str) -> f64 {
    let v = m[key].as_f64();
    if key == "price_change_percentage_24h_in_currency" {
        return v
            .or_else(|| m["price_change_percentage_24h"].as_f64())
            .unwrap_or(0.0);
    }
    v.unwrap_or(0.0)
}

/// Thin a price series to SPARK_POINTS evenly spaced samples, always
/// keeping the latest price.
fn downsample(vals: &[f64]) -> Vec<f64> {
    if vals.len() <= SPARK_POINTS {
        return vals.to_vec();
    }
    let last = vals.len() - 1;
    (0..SPARK_POINTS)
        .map(|i| vals[i * last / (SPARK_POINTS - 1)])
        .collect()
}

async fn get(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, FetchError> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Other(e.to_string()))?;
    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        return Err(FetchError::RateLimited(after));
    }
    resp.error_for_status()
        .map_err(|e| FetchError::Other(e.to_string()))?
        .json()
        .await
        .map_err(|e| FetchError::Other(e.to_string()))
}
