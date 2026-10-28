//! The STEAM widget's Web API side: who of your friends is online and what
//! they play, your recently played games and the wishlist games on sale.
//! Needs a Web API key and a public profile; the game running on this machine
//! comes from `steam_local` and needs neither, so it rides along in every
//! state.

use super::http::{self, get_json, FetchError, Throttle};
use super::steam_local;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const POLL: Duration = Duration::from_secs(60);
// Playtime moves in minutes and the list changes a few times a day.
const RECENT_EVERY: Duration = Duration::from_secs(900);
// Steam sales start and end at fixed times a day; the store API is the
// tightly rate-limited one, so it is asked rarely and retried slowly.
const SALES_EVERY: Duration = Duration::from_secs(6 * 3600);
const SALES_RETRY: Duration = Duration::from_secs(900);
const BACKOFF_MIN: Duration = Duration::from_secs(60);
const BACKOFF_MAX: Duration = Duration::from_secs(1800);
const API: &str = "https://api.steampowered.com";
const STORE: &str = "https://store.steampowered.com/api";
const ICON_BASE: &str = "https://media.steampowered.com/steamcommunity/public/images/apps";
/// GetPlayerSummaries takes at most this many ids per call.
const SUMMARIES_BATCH: usize = 100;
/// Two columns' worth at L; smaller sizes fade-clip the same list.
const FRIENDS_SHOWN: usize = 24;
const RECENT_SHOWN: usize = 5;
const SALES_SHOWN: usize = 6;
/// Ids per store request; a wishlist can hold hundreds.
const STORE_BATCH: usize = 100;
const IMAGE_MAX: usize = 64 * 1024;

/// steam.json. The id stays a string end to end: a SteamID64 is above 2^53,
/// so it would lose digits as a JavaScript number.
#[derive(Deserialize, Serialize)]
struct SteamConfig {
    steamid: String,
    /// The store country that prices the wishlist; `None` prices by IP. The
    /// Web API does not expose the account's store country, and the profile's
    /// location is free text people fill in for fun.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    /// SteamIDs pinned in the widget. Steam keeps its own favorites in the
    /// client, out of the Web API's reach.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    favorites: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SteamState {
    Disconnected {
        reason: String,
    },
    Connected {
        me: Player,
        /// Favorites (offline too) and online friends, at most `FRIENDS_SHOWN`.
        friends: Vec<Player>,
        online: usize,
        playing: usize,
        recent: Vec<RecentGame>,
        /// Wishlist games on sale, the biggest discount first.
        sales: Vec<Sale>,
        /// What the privacy settings keep from us. Each one empties its part
        /// of the widget, which then names the setting to change.
        hidden: Vec<Hidden>,
    },
}

/// What is emitted: the web state plus the game running here.
#[derive(Serialize, Clone)]
struct Emitted {
    local: Option<steam_local::Game>,
    #[serde(flatten)]
    state: SteamState,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Hidden {
    /// "My profile" is not Public: nothing but the name and avatar.
    Profile,
    FriendList,
    /// "Game details" is not Public: no recent games.
    GameDetails,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Presence {
    Online,
    Busy,
    Away,
    Offline,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Player {
    steamid: String,
    name: String,
    presence: Presence,
    /// Only reported when that player's game details are public.
    game: Option<String>,
    avatar: Option<String>,
    #[serde(skip)]
    avatar_url: Option<String>,
    #[serde(skip)]
    public: bool,
    favorite: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct RecentGame {
    appid: u32,
    name: String,
    minutes_2weeks: u32,
    minutes_total: u32,
    icon: Option<String>,
    #[serde(skip)]
    icon_url: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Sale {
    appid: u32,
    name: String,
    /// Sorts the list; shown only when `show_discount`.
    discount: u32,
    /// False where the store itself hides the percentage (EU price rules,
    /// when the price was lower in recent weeks).
    show_discount: bool,
    /// Formatted by the store in the store's currency, e.g. "9,75€".
    price: String,
    original: String,
    /// The lowest price of recent weeks, which tells a real sale from a
    /// discount off a raised price. Only `GetItems` reports it.
    lowest_recent: Option<String>,
}

#[derive(Debug)]
enum ApiError {
    Status(u16),
    Other(String),
}

/// Per account: switching to another profile starts from nothing.
#[derive(Default)]
struct Cache {
    steamid: String,
    country: Option<String>,
    /// `None` until the first successful fetch, `Some(None)` while private.
    recent: Option<Option<Vec<RecentGame>>>,
    recent_at: Option<Instant>,
    sales: Vec<Sale>,
    sales_next: Option<Instant>,
    images: HashMap<String, String>,
}

/// Wakes the poll loop early: refresh button, setup or logout.
static REFRESH: tokio::sync::Notify = tokio::sync::Notify::const_new();

#[tauri::command]
pub fn steam_refresh() {
    REFRESH.notify_one();
}

pub fn spawn(app: AppHandle, mut ready: tokio::sync::watch::Receiver<bool>) {
    tauri::async_runtime::spawn(async move {
        let _ = ready.wait_for(|r| *r).await;
        let (client, images) = match (api_client(), http::client()) {
            (Ok(c), Ok(i)) => (c, i),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("steam: failed to build http client: {e}");
                return;
            }
        };
        let mut throttle = Throttle::new("steam", BACKOFF_MIN, BACKOFF_MAX);
        let mut cache = Cache::default();
        let mut last_ok: Option<SteamState> = None;
        loop {
            let state = match credentials(&app) {
                Err(reason) => {
                    last_ok = None;
                    SteamState::Disconnected { reason }
                }
                Ok(_) if throttle.active() => last_or_disconnected(&last_ok, "rate limited"),
                Ok((key, SteamConfig { steamid, country, favorites })) => {
                    if cache.steamid != steamid {
                        cache = Cache { steamid: steamid.clone(), ..Cache::default() };
                        last_ok = None;
                    }
                    if cache.country != country {
                        cache.country = country;
                        cache.sales.clear();
                        cache.sales_next = None;
                    }
                    match poll(&client, &images, &key, &steamid, &favorites, &mut cache).await {
                        Ok(s) => {
                            throttle.ok();
                            last_ok = Some(s.clone());
                            s
                        }
                        // A revoked or mistyped key: retrying cannot help.
                        Err(ApiError::Status(401 | 403)) => {
                            last_ok = None;
                            SteamState::Disconnected { reason: "Steam rejected the API key".into() }
                        }
                        Err(ApiError::Status(429)) => {
                            throttle.hit(None);
                            last_or_disconnected(&last_ok, "rate limited")
                        }
                        Err(e) => last_or_disconnected(&last_ok, &describe(&e)),
                    }
                }
            };
            let out = Emitted { local: steam_local::running_game().map(|r| r.game), state };
            if let Err(e) = app.emit("steam", out) {
                eprintln!("steam emit failed: {e}");
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                _ = REFRESH.notified() => {}
            }
        }
    });
}

fn last_or_disconnected(last_ok: &Option<SteamState>, reason: &str) -> SteamState {
    last_ok.clone().unwrap_or_else(|| SteamState::Disconnected { reason: reason.into() })
}

fn credentials(app: &AppHandle) -> Result<(String, SteamConfig), String> {
    let key = super::keychain_secret("steam").map_err(|_| "not connected".to_string())?;
    Ok((key, load_config(app)?))
}

async fn poll(
    client: &reqwest::Client,
    images: &reqwest::Client,
    key: &str,
    steamid: &str,
    favorites: &[String],
    cache: &mut Cache,
) -> Result<SteamState, ApiError> {
    let mut hidden = Vec::new();
    let friend_ids = match call(client, "ISteamUser/GetFriendList/v1", key, &[("steamid", steamid), ("relationship", "friend")]).await {
        Ok(v) => parse_friend_ids(&v),
        // The answer for a friend list that is not public.
        Err(ApiError::Status(401)) => {
            hidden.push(Hidden::FriendList);
            Vec::new()
        }
        Err(e) => return Err(e),
    };

    let mut ids = vec![steamid.to_string()];
    ids.extend(friend_ids);
    let mut players = Vec::new();
    for chunk in ids.chunks(SUMMARIES_BATCH) {
        let v = call(client, "ISteamUser/GetPlayerSummaries/v2", key, &[("steamids", &chunk.join(","))]).await?;
        players.extend(parse_players(&v));
    }
    let me_at = players
        .iter()
        .position(|p| p.steamid == steamid)
        .ok_or_else(|| ApiError::Other("Steam does not know this profile".into()))?;
    let mut me = players.swap_remove(me_at);
    if !me.public {
        hidden.insert(0, Hidden::Profile);
    }
    let (mut friends, online, playing) = rank_friends(players, favorites);

    let due = cache.recent_at.is_none_or(|t| t.elapsed() >= RECENT_EVERY);
    if due {
        let count = RECENT_SHOWN.to_string();
        // A failure here keeps the previous list rather than failing the
        // friends, and is retried on the next tick.
        match call(client, "IPlayerService/GetRecentlyPlayedGames/v1", key, &[("steamid", steamid), ("count", &count)]).await {
            Ok(v) => {
                cache.recent = Some(parse_recent(&v));
                cache.recent_at = Some(Instant::now());
            }
            Err(e) => eprintln!("steam: recent games: {}", describe(&e)),
        }
    }
    if cache.sales_next.is_none_or(|t| Instant::now() >= t) {
        let wait = match wishlist_sales(images, steamid, cache.country.as_deref()).await {
            Ok(sales) => {
                cache.sales = sales;
                SALES_EVERY
            }
            Err(FetchError::RateLimited(after)) => after.unwrap_or(SALES_RETRY).max(SALES_RETRY),
            Err(FetchError::Other(e)) => {
                eprintln!("steam: wishlist: {e}");
                SALES_RETRY
            }
        };
        cache.sales_next = Some(Instant::now() + wait);
    }
    let sales = cache.sales.clone();

    let mut recent = match &cache.recent {
        Some(Some(games)) => games.clone(),
        Some(None) => {
            if !hidden.contains(&Hidden::Profile) {
                hidden.push(Hidden::GameDetails);
            }
            Vec::new()
        }
        None => Vec::new(),
    };

    let wanted: Vec<String> = std::iter::once(&me)
        .chain(&friends)
        .filter_map(|p| p.avatar_url.clone())
        .chain(recent.iter().filter_map(|g| g.icon_url.clone()))
        .collect();
    cache.images.retain(|url, _| wanted.contains(url));
    for url in &wanted {
        if !cache.images.contains_key(url) {
            match http::get_image_data_uri(images, url, IMAGE_MAX).await {
                Ok(uri) => {
                    cache.images.insert(url.clone(), uri);
                }
                Err(e) => eprintln!("steam: image: {e}"),
            }
        }
    }
    let image = |url: &Option<String>| url.as_ref().and_then(|u| cache.images.get(u).cloned());
    me.avatar = image(&me.avatar_url);
    for f in &mut friends {
        f.avatar = image(&f.avatar_url);
    }
    for g in &mut recent {
        g.icon = image(&g.icon_url);
    }

    Ok(SteamState::Connected { me, friends, online, playing, recent, sales, hidden })
}

/// For Web API calls only. Redirects are not followed: the key is in the
/// URL, and a redirect must not carry it anywhere but Steam's API host.
fn api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("aria-hud")
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// One Web API call. The key travels in the query string, as Steam requires,
/// so errors are stripped of the URL before anything can log or show them.
async fn call(client: &reqwest::Client, method: &str, key: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
    call_at(client, API, method, key, params).await
}

async fn call_at(client: &reqwest::Client, base: &str, method: &str, key: &str, params: &[(&str, &str)]) -> Result<Value, ApiError> {
    let mut url = reqwest::Url::parse(&format!("{base}/{method}/")).map_err(|e| ApiError::Other(e.to_string()))?;
    url.query_pairs_mut().append_pair("key", key).extend_pairs(params);
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ApiError::Other(e.without_url().to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(ApiError::Status(status.as_u16()));
    }
    resp.json().await.map_err(|e| ApiError::Other(e.without_url().to_string()))
}

fn describe(e: &ApiError) -> String {
    match e {
        ApiError::Status(401 | 403) => "Steam rejected the API key".into(),
        ApiError::Status(429) => "Steam is rate limiting, try again in a minute".into(),
        ApiError::Status(s) => format!("Steam answered HTTP {s}"),
        ApiError::Other(e) => e.clone(),
    }
}

fn parse_friend_ids(v: &Value) -> Vec<String> {
    v["friendslist"]["friends"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| Some(f["steamid"].as_str()?.to_string()))
        .collect()
}

fn parse_players(v: &Value) -> Vec<Player> {
    v["response"]["players"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| {
            Some(Player {
                steamid: p["steamid"].as_str()?.to_string(),
                name: p["personaname"].as_str().unwrap_or("?").to_string(),
                presence: match p["personastate"].as_u64() {
                    Some(1 | 5 | 6) => Presence::Online,
                    Some(2) => Presence::Busy,
                    Some(3 | 4) => Presence::Away,
                    _ => Presence::Offline,
                },
                game: p["gameextrainfo"].as_str().map(str::to_string),
                avatar: None,
                avatar_url: p["avatarmedium"].as_str().map(str::to_string),
                // 3 is "public"; 1 is private or friends-only.
                public: p["communityvisibilitystate"].as_u64() == Some(3),
                favorite: false,
            })
        })
        .collect()
}

/// Favorites first, offline ones included, then online friends; within each,
/// the ones in a game first, then by presence and name. Plus how many are
/// online and how many are playing.
fn rank_friends(players: Vec<Player>, favorites: &[String]) -> (Vec<Player>, usize, usize) {
    let mut shown: Vec<Player> = players
        .into_iter()
        .map(|p| Player { favorite: favorites.contains(&p.steamid), ..p })
        .filter(|p| p.favorite || p.presence != Presence::Offline)
        .collect();
    shown.sort_by_key(|p| (!p.favorite, p.game.is_none(), p.presence, p.name.to_lowercase()));
    let online = shown.iter().filter(|p| p.presence != Presence::Offline);
    let (count, playing) = (online.clone().count(), online.filter(|p| p.game.is_some()).count());
    shown.truncate(FRIENDS_SHOWN);
    (shown, count, playing)
}

/// `None` when game details are private: Steam then answers with an empty
/// `response`, while a public profile that played nothing has `total_count: 0`.
fn parse_recent(v: &Value) -> Option<Vec<RecentGame>> {
    let r = &v["response"];
    r.get("total_count")?;
    Some(
        r["games"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|g| {
                let appid = u32::try_from(g["appid"].as_u64()?).ok()?;
                Some(RecentGame {
                    appid,
                    name: g["name"].as_str().unwrap_or("?").to_string(),
                    minutes_2weeks: g["playtime_2weeks"].as_u64().unwrap_or(0) as u32,
                    minutes_total: g["playtime_forever"].as_u64().unwrap_or(0) as u32,
                    icon: None,
                    icon_url: g["img_icon_url"]
                        .as_str()
                        .filter(|h| !h.is_empty())
                        .map(|h| format!("{ICON_BASE}/{appid}/{h}.jpg")),
                })
            })
            .take(RECENT_SHOWN)
            .collect(),
    )
}

/// The wishlist needs no key, and the store calls neither, so these go
/// through the shared keyless fetch. With a store country one
/// `GetItems` call has names and prices; without it `appdetails` prices by
/// the caller's IP and `GetItems` is asked only for the names.
async fn wishlist_sales(client: &reqwest::Client, steamid: &str, country: Option<&str>) -> Result<Vec<Sale>, FetchError> {
    let url = reqwest::Url::parse_with_params(&format!("{API}/IWishlistService/GetWishlist/v1/"), &[("steamid", steamid)])
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let ids = parse_wishlist(&get_json(client, url.as_str()).await?);
    let mut sales = Vec::new();
    for chunk in ids.chunks(STORE_BATCH) {
        if let Some(cc) = country {
            sales.extend(parse_items(&get_json(client, items_url(chunk, cc)?.as_str()).await?));
            continue;
        }
        let ids: Vec<String> = chunk.iter().map(u32::to_string).collect();
        let url = format!("{STORE}/appdetails?appids={}&filters=price_overview", ids.join(","));
        let priced = parse_price_overview(&get_json(client, &url).await?);
        if priced.is_empty() {
            continue;
        }
        let on_sale: Vec<u32> = priced.iter().map(|s| s.appid).collect();
        let names = parse_names(&get_json(client, items_url(&on_sale, "US")?.as_str()).await?);
        sales.extend(priced.into_iter().filter_map(|s| Some(Sale { name: names.get(&s.appid)?.clone(), ..s })));
    }
    sales.sort_by(|a, b| b.discount.cmp(&a.discount).then_with(|| a.name.cmp(&b.name)));
    sales.truncate(SALES_SHOWN);
    Ok(sales)
}

fn items_url(appids: &[u32], country: &str) -> Result<reqwest::Url, FetchError> {
    let input = serde_json::json!({
        "ids": appids.iter().map(|id| serde_json::json!({ "appid": id })).collect::<Vec<_>>(),
        "context": { "language": "english", "country_code": country },
    });
    reqwest::Url::parse_with_params(
        &format!("{API}/IStoreBrowseService/GetItems/v1/"),
        &[("input_json", input.to_string())],
    )
    .map_err(|e| FetchError::Other(e.to_string()))
}

/// A private wishlist and an empty one answer the same: no `items`.
fn parse_wishlist(v: &Value) -> Vec<u32> {
    v["response"]["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| u32::try_from(i["appid"].as_u64()?).ok())
        .collect()
}

/// Discounted items only; a game with no price (unreleased, delisted) has no
/// `best_purchase_option` at all.
fn parse_items(v: &Value) -> Vec<Sale> {
    store_items(v)
        .filter_map(|it| {
            let o = &it["best_purchase_option"];
            let discount = o["discount_pct"].as_u64().filter(|d| *d > 0)? as u32;
            Some(Sale {
                appid: u32::try_from(it["appid"].as_u64()?).ok()?,
                name: it["name"].as_str()?.to_string(),
                discount,
                show_discount: !o["hide_discount_pct_for_compliance"].as_bool().unwrap_or(false),
                price: o["formatted_final_price"].as_str()?.to_string(),
                original: o["formatted_original_price"].as_str().unwrap_or("").to_string(),
                lowest_recent: o["formatted_lowest_recent_price"].as_str().map(str::to_string),
            })
        })
        .collect()
}

fn parse_names(v: &Value) -> HashMap<u32, String> {
    store_items(v)
        .filter_map(|it| Some((u32::try_from(it["appid"].as_u64()?).ok()?, it["name"].as_str()?.to_string())))
        .collect()
}

fn store_items(v: &Value) -> impl Iterator<Item = &Value> {
    v["response"]["store_items"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter()
}

/// `appdetails` keys by appid; a free game's `data` is an empty array rather
/// than an object, so it simply has no `price_overview`. Names are left empty
/// for the caller to fill.
fn parse_price_overview(v: &Value) -> Vec<Sale> {
    let Some(map) = v.as_object() else { return Vec::new() };
    map.iter()
        .filter_map(|(id, entry)| {
            let p = &entry["data"]["price_overview"];
            let discount = p["discount_percent"].as_u64().filter(|d| *d > 0)? as u32;
            Some(Sale {
                appid: id.parse().ok()?,
                name: String::new(),
                discount,
                // appdetails does not say; the store page is the authority.
                show_discount: true,
                price: p["final_formatted"].as_str()?.to_string(),
                original: p["initial_formatted"].as_str().unwrap_or("").to_string(),
                lowest_recent: None,
            })
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum ProfileRef {
    Id(String),
    Vanity(String),
}

/// Accepts what people paste: a profile URL (`/profiles/<id>` or
/// `/id/<name>`), a bare SteamID64 or a bare custom URL name.
fn parse_profile(input: &str) -> Result<ProfileRef, String> {
    let s = input.trim();
    let segment = |marker: &str| {
        s.find(marker)
            .map(|i| s[i + marker.len()..].split(['/', '?', '#']).next().unwrap_or(""))
    };
    if let Some(id) = segment("/profiles/") {
        return if is_id64(id) { Ok(ProfileRef::Id(id.into())) } else { Err("that profile link has no valid Steam ID".into()) };
    }
    if let Some(name) = segment("/id/") {
        return if name.is_empty() { Err("that profile link has no name".into()) } else { Ok(ProfileRef::Vanity(name.into())) };
    }
    if is_id64(s) {
        return Ok(ProfileRef::Id(s.into()));
    }
    if s.is_empty() || s.contains(['/', ' ', '.']) {
        return Err("paste your profile link or SteamID64".into());
    }
    Ok(ProfileRef::Vanity(s.into()))
}

fn is_id64(s: &str) -> bool {
    s.len() == 17 && s.starts_with("7656119") && s.bytes().all(|b| b.is_ascii_digit())
}

/// Web API keys are 32 hex characters; checking the shape first turns a
/// pasted wrong string into a clear message instead of a 403.
fn valid_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_resolved(v: &Value) -> Result<String, String> {
    let r = &v["response"];
    match (r["success"].as_u64(), r["steamid"].as_str()) {
        (Some(1), Some(id)) if is_id64(id) => Ok(id.to_string()),
        _ => Err("no Steam profile has that custom URL".into()),
    }
}

/// Check the key and the profile against Steam before saving either, so a
/// typo is reported in the form rather than as a broken widget.
#[tauri::command]
pub async fn steam_setup(app: AppHandle, key: String, profile: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if !valid_key(&key) {
        return Err("a Steam Web API key is 32 letters and digits".into());
    }
    let client = api_client()?;
    let steamid = match parse_profile(&profile)? {
        ProfileRef::Vanity(name) => {
            let v = call(&client, "ISteamUser/ResolveVanityURL/v1", &key, &[("vanityurl", &name)])
                .await
                .map_err(|e| describe(&e))?;
            parse_resolved(&v)?
        }
        ProfileRef::Id(id) => {
            let v = call(&client, "ISteamUser/GetPlayerSummaries/v2", &key, &[("steamids", &id)])
                .await
                .map_err(|e| describe(&e))?;
            if parse_players(&v).is_empty() {
                return Err("Steam does not know this profile".into());
            }
            id
        }
    };
    super::store_secret("steam", &key)?;
    // A new key or profile keeps the store country and favorites.
    let cfg = match load_config(&app) {
        Ok(old) => SteamConfig { steamid, ..old },
        Err(_) => SteamConfig { steamid, country: None, favorites: Vec::new() },
    };
    save_config(&app, &cfg)?;
    REFRESH.notify_one();
    Ok(())
}

/// Log out: forget the key and the profile. Best-effort on the key, so an
/// already-missing secret does not block it.
#[tauri::command]
pub fn steam_logout(app: AppHandle) -> Result<(), String> {
    if let Err(e) = super::delete_secret("steam") {
        eprintln!("steam: secret delete: {e}");
    }
    let path = config_path(&app)?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    REFRESH.notify_one();
    Ok(())
}

/// The chosen store country, for the gear panel.
#[tauri::command]
pub fn steam_store_country(app: AppHandle) -> Option<String> {
    load_config(&app).ok()?.country
}

/// An empty field goes back to pricing by IP.
#[tauri::command]
pub fn steam_set_store_country(app: AppHandle, country: String) -> Result<(), String> {
    let mut cfg = load_config(&app)?;
    cfg.country = parse_country(&country)?;
    save_config(&app, &cfg)?;
    REFRESH.notify_one();
    Ok(())
}

#[tauri::command]
pub fn steam_set_favorite(app: AppHandle, steamid: String, favorite: bool) -> Result<(), String> {
    let mut cfg = load_config(&app)?;
    cfg.favorites.retain(|id| *id != steamid);
    if favorite {
        cfg.favorites.push(steamid);
    }
    save_config(&app, &cfg)?;
    REFRESH.notify_one();
    Ok(())
}

fn parse_country(input: &str) -> Result<Option<String>, String> {
    let cc = input.trim().to_ascii_uppercase();
    if cc.is_empty() {
        return Ok(None);
    }
    if cc.len() != 2 || !cc.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("a store country is a two-letter code, e.g. CZ or DE".into());
    }
    Ok(Some(cc))
}

fn config_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("steam.json"))
}

fn load_config(app: &AppHandle) -> Result<SteamConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Err("not connected".into());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("steam.json is invalid: {e}"))
}

fn save_config(app: &AppHandle, cfg: &SteamConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(app)?, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let raw = match name {
            "friends" => include_str!("../../tests/fixtures/steam-friendlist.json"),
            "players" => include_str!("../../tests/fixtures/steam-player-summaries.json"),
            "recent" => include_str!("../../tests/fixtures/steam-recent-games.json"),
            "wishlist" => include_str!("../../tests/fixtures/steam-wishlist.json"),
            "items" => include_str!("../../tests/fixtures/steam-store-items.json"),
            "appdetails" => include_str!("../../tests/fixtures/steam-appdetails.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(raw).unwrap()
    }

    const ME: &str = "76561197960287930";

    #[test]
    fn friend_ids_parse() {
        assert_eq!(parse_friend_ids(&fixture("friends")).len(), 4);
        assert!(parse_friend_ids(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn players_map_presence_game_and_visibility() {
        let players = parse_players(&fixture("players"));
        let by = |id: &str| players.iter().find(|p| p.steamid == id).unwrap();
        let me = by(ME);
        assert!(me.public);
        assert_eq!(me.presence, Presence::Online);
        assert_eq!(by("76561198000000002").game.as_deref(), Some("Counter-Strike 2"));
        assert_eq!(by("76561198000000003").presence, Presence::Away);
        assert_eq!(by("76561198000000004").presence, Presence::Offline);
        assert!(!by("76561198000000005").public);
    }

    #[test]
    fn friends_in_a_game_come_first_and_offline_ones_are_counted_out() {
        let mut players = parse_players(&fixture("players"));
        players.retain(|p| p.steamid != ME);
        let (friends, online, playing) = rank_friends(players, &[]);
        assert_eq!((online, playing), (3, 1));
        assert_eq!(friends[0].game.as_deref(), Some("Counter-Strike 2"));
        assert!(friends.iter().all(|f| f.presence != Presence::Offline));
        // Online before away among the rest.
        assert_eq!(friends[1].presence, Presence::Online);
        assert_eq!(friends[2].presence, Presence::Away);
    }

    #[test]
    fn the_list_is_capped_but_the_counts_are_not() {
        let p = |i: usize| Player {
            steamid: i.to_string(),
            name: format!("p{i}"),
            presence: Presence::Online,
            game: None,
            avatar: None,
            avatar_url: None,
            public: true,
            favorite: false,
        };
        let (friends, online, _) = rank_friends((0..30).map(p).collect(), &[]);
        assert_eq!(friends.len(), FRIENDS_SHOWN);
        assert_eq!(online, 30);
    }

    #[test]
    fn favorites_come_first_even_offline_and_do_not_count_as_online() {
        let mut players = parse_players(&fixture("players"));
        players.retain(|p| p.steamid != ME);
        let (friends, online, playing) = rank_friends(players, &["76561198000000004".into()]);
        assert_eq!((online, playing), (3, 1));
        assert_eq!(friends[0].steamid, "76561198000000004");
        assert!(friends[0].favorite);
        assert_eq!(friends[0].presence, Presence::Offline);
        assert_eq!(friends[1].game.as_deref(), Some("Counter-Strike 2"));
    }

    #[test]
    fn recent_games_carry_playtime_and_an_icon_url() {
        let games = parse_recent(&fixture("recent")).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "Overwatch");
        assert_eq!(games[0].minutes_2weeks, 754);
        assert_eq!(
            games[0].icon_url.as_deref(),
            Some("https://media.steampowered.com/steamcommunity/public/images/apps/2357570/abc123.jpg")
        );
        assert_eq!(games[1].icon_url, None);
    }

    #[test]
    fn private_game_details_differ_from_nothing_played() {
        assert_eq!(parse_recent(&serde_json::json!({ "response": {} })), None);
        assert_eq!(parse_recent(&serde_json::json!({ "response": { "total_count": 0 } })), Some(vec![]));
    }

    #[test]
    fn wishlist_ids_parse_and_private_is_empty() {
        assert_eq!(parse_wishlist(&fixture("wishlist")), vec![10, 20, 30]);
        assert!(parse_wishlist(&serde_json::json!({ "response": {} })).is_empty());
    }

    #[test]
    fn store_items_keep_only_discounted_games() {
        let sales = parse_items(&fixture("items"));
        assert_eq!(sales.len(), 1);
        let s = &sales[0];
        assert_eq!((s.appid, s.discount), (20, 50));
        assert_eq!(s.price, "9,75€");
        assert_eq!(s.original, "19,50€");
        assert!(s.lowest_recent.is_some());
        assert!(!s.show_discount, "the fixture's sale hides its percentage, as Steam does in the EU");
    }

    #[test]
    fn names_cover_every_item_with_one() {
        let names = parse_names(&fixture("items"));
        assert_eq!(names.len(), 3);
        assert_eq!(names[&20], "Sale Game");
    }

    #[test]
    fn appdetails_skips_full_price_and_free_games() {
        let sales = parse_price_overview(&fixture("appdetails"));
        assert_eq!(sales.len(), 1);
        assert_eq!((sales[0].appid, sales[0].discount), (20, 50));
        assert_eq!(sales[0].original, "19,50€");
        assert!(sales[0].name.is_empty());
    }

    #[test]
    fn store_country_is_two_letters_or_nothing() {
        assert_eq!(parse_country(" cz ").unwrap().as_deref(), Some("CZ"));
        assert_eq!(parse_country("").unwrap(), None);
        assert!(parse_country("CZE").is_err());
        assert!(parse_country("C1").is_err());
    }

    #[test]
    fn config_without_a_country_still_loads() {
        let cfg: SteamConfig = serde_json::from_str(r#"{"steamid":"1"}"#).unwrap();
        assert_eq!(cfg.country, None);
    }

    #[test]
    fn items_url_carries_ids_and_country() {
        let url = items_url(&[10, 20], "CZ").ok().expect("a valid URL");
        let input = url.query_pairs().find(|(k, _)| k == "input_json").unwrap().1;
        let v: Value = serde_json::from_str(&input).unwrap();
        assert_eq!(v["ids"][1]["appid"], 20);
        assert_eq!(v["context"]["country_code"], "CZ");
    }

    #[test]
    fn profile_input_forms() {
        let id = ProfileRef::Id(ME.into());
        assert_eq!(parse_profile(&format!("https://steamcommunity.com/profiles/{ME}/")), Ok(id));
        assert_eq!(parse_profile(ME), Ok(ProfileRef::Id(ME.into())));
        assert_eq!(
            parse_profile(" https://steamcommunity.com/id/gaben/?l=czech "),
            Ok(ProfileRef::Vanity("gaben".into()))
        );
        assert_eq!(parse_profile("gaben"), Ok(ProfileRef::Vanity("gaben".into())));
        assert!(parse_profile("https://steamcommunity.com/profiles/123/").is_err());
        assert!(parse_profile("store.steampowered.com").is_err());
        assert!(parse_profile("").is_err());
    }

    #[test]
    fn key_shape() {
        assert!(valid_key("0123456789ABCDEF0123456789abcdef"));
        assert!(!valid_key("0123456789ABCDEF0123456789abcde"));
        assert!(!valid_key("0123456789ABCDEF0123456789abcdeg"));
    }

    #[test]
    fn vanity_resolution() {
        let ok = serde_json::json!({ "response": { "steamid": ME, "success": 1 } });
        assert_eq!(parse_resolved(&ok), Ok(ME.to_string()));
        let miss = serde_json::json!({ "response": { "success": 42, "message": "No match" } });
        assert!(parse_resolved(&miss).is_err());
    }

    #[tokio::test]
    async fn a_failed_call_does_not_reveal_the_key() {
        const KEY: &str = "0123456789ABCDEF0123456789ABCDEF";
        // Port 9 (discard) is closed, so this fails in transport, the error
        // reqwest would otherwise print with the full URL.
        let err = call_at(&api_client().unwrap(), "http://127.0.0.1:9", "ISteamUser/GetFriendList/v1", KEY, &[])
            .await
            .unwrap_err();
        let text = format!("{} {err:?}", describe(&err));
        assert!(!text.contains(KEY), "{text}");
    }

    #[test]
    fn emitted_state_is_flat_with_the_local_game_beside_it() {
        let state = SteamState::Disconnected { reason: "not connected".into() };
        let v = serde_json::to_value(Emitted { local: None, state }).unwrap();
        assert_eq!(v, serde_json::json!({ "local": null, "status": "disconnected", "reason": "not connected" }));
    }
}
