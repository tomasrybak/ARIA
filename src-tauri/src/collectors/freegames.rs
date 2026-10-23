use super::http::{self, get_json, FetchError};
use chrono::{DateTime, NaiveDateTime};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

// Giveaways change a few times a week; four polls a day catch a new one the
// same day without asking two free APIs more than they need.
const POLL: Duration = Duration::from_secs(6 * 3600);
const EPIC_URL: &str =
    "https://store-site-backend-static.ak.epicgames.com/freeGamesPromotions?locale=en-US&country=CZ&allowCountries=CZ";
// `/giveaways` rejects a combined platform list; `/filter` is the endpoint for it.
const GAMERPOWER_URL: &str = "https://www.gamerpower.com/api/filter?platform=steam.gog&type=game";
const EPIC_STORE: &str = "https://store.epicgames.com/p/";
const EPIC_FREE_PAGE: &str = "https://store.epicgames.com/free-games";
// Epic's CDN resizes on request; the originals are 2560 px wide.
const EPIC_THUMB_PARAMS: &str = "?w=480&h=270&resize=1";
const THUMB_MAX: usize = 400_000;

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Game {
    title: String,
    store: &'static str,
    url: String,
    /// Epoch ms; None for a giveaway that is already live.
    starts_at: Option<i64>,
    /// Epoch ms; None when the source names no end.
    ends_at: Option<i64>,
    worth: Option<String>,
    /// `data:` URI, filled in after parsing.
    thumb: Option<String>,
    #[serde(skip)]
    thumb_url: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum FreeGamesState {
    Disconnected { reason: String },
    /// `failed` names the sources that did not answer this cycle; their games
    /// are missing rather than the whole list being withheld.
    Connected { games: Vec<Game>, failed: Vec<&'static str> },
}

static REFRESH: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tauri::command]
pub fn freegames_refresh() {
    REFRESH.notify_one();
}

pub fn spawn(app: AppHandle, mut ready: tokio::sync::watch::Receiver<bool>) {
    tauri::async_runtime::spawn(async move {
        let _ = ready.wait_for(|r| *r).await;
        let client = match http::client() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("freegames: failed to build http client: {e}");
                return;
            }
        };
        // Covers by source URL, for the life of the process: the same games
        // come back every poll for days.
        let mut thumbs: HashMap<String, String> = HashMap::new();
        loop {
            let state = poll(&client, &mut thumbs).await;
            if let Err(e) = app.emit("freegames", state) {
                eprintln!("freegames emit failed: {e}");
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = REFRESH.notified() => {}
            }
        }
    });
}

async fn poll(client: &reqwest::Client, thumbs: &mut HashMap<String, String>) -> FreeGamesState {
    let (epic, gp) = tokio::join!(get_json(client, EPIC_URL), get_json(client, GAMERPOWER_URL));
    let mut failed = Vec::new();
    let mut errors = Vec::new();
    let mut take = |name: &'static str, r: Result<Value, FetchError>| match r {
        Ok(v) => Some(v),
        Err(e) => {
            failed.push(name);
            errors.push(match e {
                FetchError::RateLimited(_) => format!("{name}: rate limited"),
                FetchError::Other(e) => format!("{name}: {e}"),
            });
            None
        }
    };
    let epic = take("epic", epic).map(|v| parse_epic(&v)).unwrap_or_default();
    let gp = take("gamerpower", gp).map(|v| parse_gamerpower(&v)).unwrap_or_default();
    if failed.len() == 2 {
        return FreeGamesState::Disconnected { reason: errors.join("; ") };
    }

    let mut games = merge(epic, gp);
    thumbs.retain(|url, _| games.iter().any(|g| g.thumb_url.as_deref() == Some(url.as_str())));
    for g in &mut games {
        let Some(url) = g.thumb_url.clone() else { continue };
        if !thumbs.contains_key(&url) {
            match http::get_image_data_uri(client, &url, THUMB_MAX).await {
                Ok(uri) => {
                    thumbs.insert(url.clone(), uri);
                }
                Err(e) => eprintln!("freegames: cover for {}: {e}", g.title),
            }
        }
        g.thumb = thumbs.get(&url).cloned();
    }
    FreeGamesState::Connected { games, failed }
}

fn epoch_ms(iso: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.timestamp_millis())
}

/// Free offers from Epic's promotions feed. The feed also carries ordinary
/// sales; a free one is an offer whose `discountPercentage` (the share of the
/// price left to pay) is 0.
fn parse_epic(v: &Value) -> Vec<Game> {
    let Some(elements) = v["data"]["Catalog"]["searchStore"]["elements"].as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in elements {
        let Some(title) = e["title"].as_str() else { continue };
        let promos = &e["promotions"];
        for (key, upcoming) in [("promotionalOffers", false), ("upcomingPromotionalOffers", true)] {
            let offers = promos[key].as_array().into_iter().flatten();
            let free = offers
                .flat_map(|g| g["promotionalOffers"].as_array().into_iter().flatten())
                .find(|o| o["discountSetting"]["discountPercentage"].as_i64() == Some(0));
            let Some(offer) = free else { continue };
            out.push(Game {
                title: title.to_string(),
                store: "epic",
                url: epic_url(e),
                starts_at: if upcoming { offer["startDate"].as_str().and_then(epoch_ms) } else { None },
                ends_at: offer["endDate"].as_str().and_then(epoch_ms),
                worth: e["price"]["totalPrice"]["fmtPrice"]["originalPrice"]
                    .as_str()
                    .filter(|p| *p != "0")
                    .map(String::from),
                thumb: None,
                thumb_url: epic_thumb(e),
            });
        }
    }
    out
}

fn epic_url(e: &Value) -> String {
    let mapped = |k: &str| {
        e[k].as_array()
            .into_iter()
            .flatten()
            .find_map(|m| m["pageSlug"].as_str().filter(|s| !s.is_empty()))
            .map(String::from)
    };
    let slug = e["catalogNs"]["mappings"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|m| m["pageSlug"].as_str().filter(|s| !s.is_empty()).map(String::from))
        .or_else(|| mapped("offerMappings"))
        .or_else(|| e["productSlug"].as_str().filter(|s| !s.is_empty()).map(String::from));
    match slug {
        Some(s) => format!("{EPIC_STORE}{s}"),
        None => EPIC_FREE_PAGE.to_string(),
    }
}

fn epic_thumb(e: &Value) -> Option<String> {
    let images = e["keyImages"].as_array()?;
    ["OfferImageWide", "Thumbnail", "OfferImageTall"].iter().find_map(|kind| {
        images
            .iter()
            .find(|i| i["type"].as_str() == Some(kind))
            .and_then(|i| i["url"].as_str())
            .map(|u| format!("{u}{EPIC_THUMB_PARAMS}"))
    })
}

/// Steam and GOG giveaways from GamerPower. An empty result is not an empty
/// array but `{"status": 0, ...}`, which reads as no games, not as an error.
fn parse_gamerpower(v: &Value) -> Vec<Game> {
    let Some(items) = v.as_array() else { return Vec::new() };
    items
        .iter()
        .filter_map(|g| {
            let platforms = g["platforms"].as_str().unwrap_or("");
            // Epic giveaways come from Epic's own feed, with exact times.
            let store = if platforms.contains("Steam") {
                "steam"
            } else if platforms.contains("GOG") {
                "gog"
            } else {
                return None;
            };
            Some(Game {
                title: clean_title(g["title"].as_str()?),
                store,
                url: g["open_giveaway_url"].as_str()?.to_string(),
                starts_at: None,
                // "2026-10-30 23:59:00" with no zone; GamerPower runs on UTC.
                ends_at: g["end_date"]
                    .as_str()
                    .and_then(|d| NaiveDateTime::parse_from_str(d, "%Y-%m-%d %H:%M:%S").ok())
                    .map(|d| d.and_utc().timestamp_millis()),
                worth: g["worth"].as_str().filter(|w| *w != "N/A").map(String::from),
                thumb: None,
                thumb_url: g["thumbnail"].as_str().map(String::from),
            })
        })
        .collect()
}

/// "Dwarven Realms (Steam) Key Giveaway" -> "Dwarven Realms".
fn clean_title(raw: &str) -> String {
    let mut t = raw.trim();
    for suffix in [" Key Giveaway", " Giveaway"] {
        t = t.strip_suffix(suffix).unwrap_or(t);
    }
    if let Some(open) = t.rfind(" (") {
        if t.ends_with(')') {
            t = &t[..open];
        }
    }
    t.trim().to_string()
}

fn title_key(title: &str) -> String {
    title.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Epic first (exact times, the store's own art), then the giveaways for a
/// game not already listed.
fn merge(epic: Vec<Game>, giveaways: Vec<Game>) -> Vec<Game> {
    let mut out = epic;
    for g in giveaways {
        let key = title_key(&g.title);
        if !out.iter().any(|o| title_key(&o.title) == key) {
            out.push(g);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epic_fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/epic-free-games.json")).unwrap()
    }

    #[test]
    fn epic_keeps_only_free_offers() {
        let games = parse_epic(&epic_fixture());
        let titles: Vec<_> = games.iter().map(|g| g.title.as_str()).collect();
        assert!(titles.contains(&"Sker Ritual"));
        assert!(titles.contains(&"Home - A Unique Horror Adventure"));
        // Discounted, not free.
        assert!(!titles.contains(&"Castlevania Anniversary Collection"));
        assert!(!titles.contains(&"Monument Valley"));
    }

    #[test]
    fn epic_live_and_upcoming_are_told_apart() {
        let games = parse_epic(&epic_fixture());
        let sker = games.iter().find(|g| g.title == "Sker Ritual").unwrap();
        assert_eq!(sker.starts_at, None);
        assert_eq!(sker.ends_at, epoch_ms("2026-10-29T15:00:00.000Z"));
        // On sale now, and free from the 29th: listed once, as upcoming.
        let closed: Vec<_> = games.iter().filter(|g| g.title == "Sorry We're Closed").collect();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].starts_at.is_some());
    }

    #[test]
    fn epic_links_the_store_page_and_asks_for_a_small_cover() {
        let games = parse_epic(&epic_fixture());
        let sker = games.iter().find(|g| g.title == "Sker Ritual").unwrap();
        assert_eq!(sker.url, "https://store.epicgames.com/p/sker-ritual-aac702");
        assert!(sker.thumb_url.as_deref().unwrap().ends_with(EPIC_THUMB_PARAMS));
    }

    #[test]
    fn gamerpower_giveaways_parse_and_titles_are_cleaned() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/gamerpower-giveaways.json")).unwrap();
        let games = parse_gamerpower(&v);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].title, "Dwarven Realms");
        assert_eq!(games[0].store, "steam");
        assert_eq!(games[0].worth.as_deref(), Some("$9.99"));
        // "N/A" end date.
        assert_eq!(games[0].ends_at, None);
    }

    #[test]
    fn gamerpower_empty_answer_is_no_games_not_an_error() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/gamerpower-empty.json")).unwrap();
        assert!(v.is_object());
        assert!(parse_gamerpower(&v).is_empty());
    }

    #[test]
    fn gamerpower_dates_and_epic_listings() {
        let v = serde_json::json!([
            { "title": "Foo (GOG) Giveaway", "platforms": "PC, GOG", "open_giveaway_url": "https://x/foo",
              "end_date": "2026-10-30 23:59:00", "worth": "N/A" },
            { "title": "Bar (Epic Games) Giveaway", "platforms": "PC, Epic Games Store", "open_giveaway_url": "https://x/bar" }
        ]);
        let games = parse_gamerpower(&v);
        assert_eq!(games.len(), 1, "Epic giveaways come from Epic's own feed");
        assert_eq!(games[0].store, "gog");
        assert_eq!(games[0].ends_at, epoch_ms("2026-10-30T23:59:00Z"));
        assert_eq!(games[0].worth, None);
    }

    #[test]
    fn merge_drops_a_giveaway_epic_already_lists() {
        let game = |title: &str, store| Game {
            title: title.into(),
            store,
            url: String::new(),
            starts_at: None,
            ends_at: None,
            worth: None,
            thumb: None,
            thumb_url: None,
        };
        let merged = merge(vec![game("Sker Ritual", "epic")], vec![game("Sker  ritual!", "steam"), game("Other", "gog")]);
        let stores: Vec<_> = merged.iter().map(|g| g.store).collect();
        assert_eq!(stores, ["epic", "gog"]);
    }

    #[test]
    fn titles_lose_store_and_giveaway_suffixes() {
        assert_eq!(clean_title("Dwarven Realms (Steam) Key Giveaway"), "Dwarven Realms");
        assert_eq!(clean_title("Foo (GOG) Giveaway"), "Foo");
        assert_eq!(clean_title("Plain Title"), "Plain Title");
    }
}
