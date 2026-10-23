use std::time::{Duration, Instant};

/// Shared by the keyless JSON collectors (crypto, weather, free games): one GET, with a
/// 429 reported separately so the caller can back off instead of hammering
/// a free tier.
pub enum FetchError {
    RateLimited(Option<Duration>),
    Other(String),
}

/// Global pause after a 429. Honors Retry-After when sent, otherwise
/// doubles the pause on each consecutive 429 up to `max`.
pub struct Throttle {
    label: &'static str,
    min: Duration,
    max: Duration,
    until: Option<Instant>,
    backoff: Duration,
}

impl Throttle {
    pub fn new(label: &'static str, min: Duration, max: Duration) -> Self {
        Self { label, min, max, until: None, backoff: min }
    }

    pub fn active(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    pub fn hit(&mut self, retry_after: Option<Duration>) {
        let wait = retry_after.unwrap_or(self.backoff).clamp(self.min, self.max);
        self.until = Some(Instant::now() + wait);
        self.backoff = (self.backoff * 2).min(self.max);
        eprintln!("{}: rate limited, pausing {}s", self.label, wait.as_secs());
    }

    pub fn ok(&mut self) {
        self.backoff = self.min;
    }
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("aria-hud")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, FetchError> {
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

/// An image as a `data:` URI, for widgets that show remote art: the CSP only
/// lets the webview load images from `data:` and the app itself.
pub async fn get_image_data_uri(client: &reqwest::Client, url: &str, max: usize) -> Result<String, String> {
    use base64::Engine;
    let resp = client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mime = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_string())
        .filter(|v| v.starts_with("image/"))
        .ok_or("not an image")?;
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > max {
        return Err(format!("{} bytes is not a usable image", bytes.len()));
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_resets() {
        let mut t = Throttle::new("test", Duration::from_secs(60), Duration::from_secs(900));
        t.hit(None);
        assert!(t.active());
        assert_eq!(t.backoff, Duration::from_secs(120));
        t.hit(None);
        t.hit(None);
        t.hit(None);
        assert_eq!(t.backoff, Duration::from_secs(900));
        t.ok();
        assert_eq!(t.backoff, Duration::from_secs(60));
    }

    #[test]
    fn retry_after_is_clamped() {
        let mut t = Throttle::new("test", Duration::from_secs(60), Duration::from_secs(900));
        t.hit(Some(Duration::from_secs(5)));
        let left = t.until.unwrap() - Instant::now();
        assert!(left > Duration::from_secs(55));
    }
}
