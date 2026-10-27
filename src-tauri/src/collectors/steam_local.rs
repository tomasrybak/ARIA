//! The local Steam install: which games are installed, what they are called,
//! and which one is running, read from the appmanifests Steam keeps next to
//! every game and from the process table. No network and no API key; the Web
//! API part is the STEAM widget's.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// An installed game, as its appmanifest describes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Game {
    pub id: u32,
    pub name: String,
    /// The folder under `steamapps/common` it is installed in.
    #[serde(skip)]
    installdir: String,
}

/// A game that is running right now, and the processes that make it up.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub game: Game,
    pub pids: Vec<u32>,
}

/// Proton, the Steam Linux Runtimes and the shared redistributables are
/// installed exactly like games, and no manifest field says otherwise, so they
/// are known by the folder they install into.
const TOOL_DIRS: &[&str] = &["Proton", "SteamLinuxRuntime", "Steamworks Shared"];
/// Launcher services that install as their own app.
const TOOL_IDS: &[u32] = &[
    4099820, // Ubisoft Connect Services
];

fn is_tool(game: &Game) -> bool {
    TOOL_IDS.contains(&game.id) || TOOL_DIRS.iter().any(|d| game.installdir.starts_with(d))
}

/// Every installed game, tools excluded, sorted by name. The voice
/// assistant's "launch game" resolves names against this list.
#[tauri::command]
pub fn steam_games() -> Vec<Game> {
    installed_games()
}

/// Re-read only when the manifest files change (a game installed, updated or
/// removed), so it is cheap enough to ask on every screen-time tick.
pub fn installed_games() -> Vec<Game> {
    static CACHE: Mutex<Option<(Vec<(PathBuf, SystemTime)>, Vec<Game>)>> = Mutex::new(None);
    let manifests = manifest_files(&libraries());
    let Ok(mut guard) = CACHE.lock() else {
        return Vec::new();
    };
    if let Some((seen, games)) = guard.as_ref() {
        if *seen == manifests {
            return games.clone();
        }
    }
    let mut games: Vec<Game> = manifests
        .iter()
        .filter_map(|(path, _)| read_app_manifest(path))
        .filter(|g| !is_tool(g))
        .collect();
    // The same game in two libraries (a copy left behind on another drive).
    games.sort_by_key(|g| g.id);
    games.dedup_by_key(|g| g.id);
    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    *guard = Some((manifests, games.clone()));
    games
}

fn manifest_files(libraries: &[PathBuf]) -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    for dir in libraries {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((e.path(), mtime));
        }
    }
    out.sort();
    out
}

/// One row of the process table, reduced to what `find_running` needs.
struct Proc {
    pid: u32,
    parent: Option<u32>,
    start: u64,
    cmd: Vec<String>,
    exe: Option<String>,
}

/// A full process scan costs ~30 ms (measured on Linux, ~500 processes): too
/// much for every 5 s screen-time tick, and a game starts and quits on a far
/// slower clock than that.
const SCAN_TTL: Duration = Duration::from_secs(15);

/// The Steam game running now, if any. Counts while it is in the background
/// too: the Gaming board follows the game, not the focus.
pub fn running_game() -> Option<Running> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    // The `System` is kept between scans so `OnlyIfNotSet` reads a process's
    // cmd and exe once, not on every scan.
    struct Scan {
        sys: System,
        at: Option<Instant>,
        last: Option<Running>,
    }
    static SCAN: Mutex<Option<Scan>> = Mutex::new(None);
    let mut guard = SCAN.lock().ok()?;
    let scan = guard.get_or_insert_with(|| Scan { sys: System::new(), at: None, last: None });
    if scan.at.is_some_and(|t| t.elapsed() < SCAN_TTL) {
        return scan.last.clone();
    }
    scan.sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::OnlyIfNotSet)
            .with_exe(UpdateKind::OnlyIfNotSet),
    );
    let procs: Vec<Proc> = scan
        .sys
        .processes()
        .values()
        .map(|p| Proc {
            pid: p.pid().as_u32(),
            parent: p.parent().map(|pp| pp.as_u32()),
            start: p.start_time(),
            cmd: p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect(),
            exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
        })
        .collect();
    scan.last = find_running(&procs, &installed_games(), &libraries());
    scan.at = Some(Instant::now());
    scan.last.clone()
}

/// Two ways to spot a game, best first:
/// - Steam starts every Linux game, Proton or native, as
///   `reaper SteamLaunch AppId=<id> -- …`, and the reaper lives as long as the
///   game. That names the id outright; the game's own path is no help there,
///   because Proton and the runtime carry it in their command lines too.
/// - Elsewhere, a process whose executable sits under
///   `<library>/common/<installdir>` belongs to that game.
///
/// Several candidates (a game launched while another closes): the newest wins.
fn find_running(procs: &[Proc], games: &[Game], libraries: &[PathBuf]) -> Option<Running> {
    let reaper = procs
        .iter()
        .filter_map(|p| reaper_app_id(&p.cmd).map(|id| (p, id)))
        .max_by_key(|(p, _)| p.start);
    if let Some((root, id)) = reaper {
        let game = games.iter().find(|g| g.id == id).cloned().unwrap_or_else(|| Game {
            id,
            name: "Steam game".to_string(),
            installdir: String::new(),
        });
        return Some(Running { game, pids: process_tree(procs, root.pid) });
    }

    let mut best: Option<(&Game, u64)> = None;
    let mut pids = Vec::new();
    for game in games.iter().filter(|g| !g.installdir.is_empty()) {
        let dirs: Vec<String> = libraries
            .iter()
            .map(|l| l.join("common").join(&game.installdir).to_string_lossy().into_owned())
            .collect();
        let mine: Vec<&Proc> = procs
            .iter()
            .filter(|p| p.exe.as_deref().is_some_and(|exe| dirs.iter().any(|d| is_under(exe, d))))
            .collect();
        let Some(start) = mine.iter().map(|p| p.start).max() else { continue };
        if best.is_none_or(|(_, s)| start > s) {
            best = Some((game, start));
            pids = mine.iter().map(|p| p.pid).collect();
        }
    }
    best.map(|(game, _)| Running { game: game.clone(), pids })
}

/// `AppId=<id>` from a reaper command line. Matched on the binary's file name,
/// not a substring: the `sh -c` wrapper around it carries the same words.
fn reaper_app_id(cmd: &[String]) -> Option<u32> {
    let exe = cmd.first()?;
    let file = exe.rsplit(['/', '\\']).next()?;
    if file != "reaper" || !cmd.iter().any(|a| a == "SteamLaunch") {
        return None;
    }
    cmd.iter()
        .take_while(|a| *a != "--")
        .find_map(|a| a.strip_prefix("AppId="))
        .and_then(|id| id.parse().ok())
}

/// `root` and every process descended from it.
fn process_tree(procs: &[Proc], root: u32) -> Vec<u32> {
    let mut tree = vec![root];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        tree.extend(procs.iter().filter(|p| p.parent == Some(parent)).map(|p| p.pid));
        i += 1;
    }
    tree
}

/// Whether `path` lies inside `dir`. Separator- and case-insensitive: Windows
/// and macOS paths compare that way, and two Steam folders differing only by
/// case don't occur on Linux either.
fn is_under(path: &str, dir: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    let (path, dir) = (norm(path), norm(dir));
    path.strip_prefix(dir.trim_end_matches('/')).is_some_and(|rest| rest.starts_with('/'))
}

/// A Proton/Steam window calls itself `steam_app_<id>`, which is the one name
/// on the board that means nothing to a human — and the one most likely to be
/// on screen for hours. Steam already knows the answer: every installed game
/// has an `appmanifest_<id>.acf` next to it naming it. So read it.
///
/// Looked up once per id and cached, including the misses: a game with no
/// manifest (uninstalled since, or a shortcut) must not re-scan the disk
/// every five seconds for the rest of the session.
pub fn steam_app_name(app_id: &str) -> Option<String> {
    use std::collections::HashMap;
    // Case-insensitive: the class arrives as `steam_app_2357570`, but a day
    // file stores what `friendly` made of it, `Steam_app_2357570`.
    const PREFIX: &str = "steam_app_";
    let id = app_id
        .as_bytes()
        .get(..PREFIX.len())
        .filter(|head| head.eq_ignore_ascii_case(PREFIX.as_bytes()))
        .map(|_| &app_id[PREFIX.len()..])?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    static CACHE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(id) {
        return hit.clone();
    }
    let found = libraries()
        .into_iter()
        .find_map(|dir| read_app_manifest(&dir.join(format!("appmanifest_{id}.acf"))))
        .map(|g| g.name);
    cache.insert(id.to_string(), found.clone());
    found
}

/// Every `steamapps` directory Steam might have put a game in: the usual
/// install roots, plus whatever `libraryfolders.vdf` adds for games kept on
/// another drive.
fn libraries() -> Vec<PathBuf> {
    expand_libraries(&roots())
}

#[cfg(target_os = "linux")]
fn roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join(".steam/steam/steamapps"),
        home.join(".steam/debian-installation/steamapps"),
        home.join(".local/share/Steam/steamapps"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/steamapps"), // flatpak
    ]
}

#[cfg(target_os = "macos")]
fn roots() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![home.join("Library/Application Support/Steam/steamapps")]
}

/// Steam records where it is installed in the registry; the default folder is
/// the fallback for an install that predates or lost the key.
#[cfg(target_os = "windows")]
fn roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = registry_steam_path().into_iter().map(|p| p.join("steamapps")).collect();
    let default = PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps");
    if !out.contains(&default) {
        out.push(default);
    }
    out
}

/// `HKCU\Software\Valve\Steam\SteamPath`, e.g. `c:/program files (x86)/steam`.
#[cfg(target_os = "windows")]
fn registry_steam_path() -> Option<PathBuf> {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 1024];
    let mut bytes = std::mem::size_of_val(&buf) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Valve\Steam"),
            w!("SteamPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    }
    .ok()
    .ok()?;
    // `bytes` counts the terminating NUL.
    let len = (bytes as usize / 2).saturating_sub(1);
    let path = String::from_utf16_lossy(&buf[..len]);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Every one of `roots` that exists, plus every library their
/// `libraryfolders.vdf` files point at — games kept on a second drive.
///
/// A library on an unmounted drive is simply absent rather than an error: the
/// id still matches the game trigger (that is a window class, not a file), the
/// name just falls back to the class until the drive is there.
fn expand_libraries(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = roots.iter().filter(|p| p.is_dir()).cloned().collect();
    // "path"  "/mnt/games/SteamLibrary"  → /mnt/games/SteamLibrary/steamapps
    for root in &out.clone() {
        let Ok(vdf) = std::fs::read_to_string(root.join("libraryfolders.vdf")) else {
            continue;
        };
        for line in vdf.lines() {
            let Some(path) = vdf_value(line, "path") else { continue };
            let dir = PathBuf::from(path).join("steamapps");
            if dir.is_dir() && !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    out
}

/// An appmanifest's id, name and install folder. The name is trimmed of the
/// trademark noise Steam keeps in its titles ("Overwatch®" is not what anyone
/// calls it).
fn read_app_manifest(path: &Path) -> Option<Game> {
    let text = std::fs::read_to_string(path).ok()?;
    let field = |key| text.lines().find_map(|l| vdf_value(l, key));
    let id = field("appid")?.parse().ok()?;
    let name = field("name")?.trim_matches(|c: char| c == '®' || c == '™' || c.is_whitespace());
    if name.is_empty() {
        return None;
    }
    Some(Game {
        id,
        name: name.to_string(),
        installdir: field("installdir").unwrap_or_default().to_string(),
    })
}

/// One line of Valve's key/value format: `\t"key"\t\t"value"`. Good enough for
/// the flat fields read here, and never asked to parse the nested blocks.
fn vdf_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let mut parts = line.split('"').skip(1);
    let found = parts.next()?;
    if found != key {
        return None;
    }
    parts.next()?; // the whitespace between the quoted key and quoted value
    parts.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vdf_line_yields_only_its_own_key() {
        assert_eq!(vdf_value("\t\"name\"\t\t\"Overwatch®\"", "name"), Some("Overwatch®"));
        assert_eq!(vdf_value("\t\"path\"\t\t\"/mnt/games/Lib\"", "path"), Some("/mnt/games/Lib"));
        assert_eq!(vdf_value("\t\"name\"\t\t\"x\"", "path"), None, "a different key is not a match");
        // Nothing here is a well-formed pair; none of it may panic.
        assert_eq!(vdf_value("", "name"), None);
        assert_eq!(vdf_value("\"name\"", "name"), None);
        assert_eq!(vdf_value("{", "name"), None);
    }

    #[test]
    fn a_steam_window_is_named_by_its_manifest() {
        // Only the well-formed ids are looked up at all; everything else must
        // fall through to the ordinary WM_CLASS handling rather than hitting
        // the disk on every activity tick.
        assert_eq!(steam_app_name("firefox"), None);
        assert_eq!(steam_app_name("steam_app_"), None);
        assert_eq!(steam_app_name("steam_app_notanumber"), None);
        // An id that is syntactically fine but not installed resolves to
        // nothing.
        assert_eq!(steam_app_name("steam_app_999999999"), None);
    }

    /// The regression this pair exists for: resolving the game name inside
    /// `friendly` renamed the string `isGameApp` matches on, so the Gaming
    /// board stopped switching itself while Overwatch was in focus.

    /// Live check against this machine's Steam install: every game it has,
    /// in every library, must resolve to a name. Skipped where Steam isn't
    /// installed so the suite stays runnable anywhere. Nothing here knows any
    /// particular game — the ids come off the disk.
    #[test]
    fn every_installed_game_resolves_to_a_name() {
        let mut seen = 0;
        let libs = libraries();
        eprintln!("steam libraries: {libs:#?}");
        for dir in libs {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let file = e.file_name();
                let name = file.to_string_lossy();
                let Some(id) = name
                    .strip_prefix("appmanifest_")
                    .and_then(|r| r.strip_suffix(".acf"))
                else {
                    continue;
                };
                let class = format!("steam_app_{id}");
                let label = steam_app_name(&class);
                eprintln!("{dir:?} {class} -> {label:?}");
                assert!(label.is_some(), "{class} in {dir:?} did not resolve");
                seen += 1;
            }
        }
        if seen == 0 {
            eprintln!("skipping: no Steam manifests on this machine");
        }
    }

    #[test]
    fn a_library_on_another_drive_is_found_through_the_vdf() {
        let base = std::env::temp_dir().join(format!("aria-libs-{}", std::process::id()));
        let main = base.join("home/steamapps");
        let ssd = base.join("media/GamesSSD/SteamLibrary/steamapps");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&ssd).unwrap();
        // Exactly the shape Steam writes, tabs and all, plus a library on a
        // drive that is not mounted right now.
        std::fs::write(
            main.join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"2\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                base.join("home").display(),
                base.join("media/GamesSSD/SteamLibrary").display(),
                base.join("media/NotMounted").display(),
            ),
        )
        .unwrap();

        let found = expand_libraries(&[main.clone()]);
        assert!(found.contains(&main), "the main library");
        assert!(found.contains(&ssd), "the library on the other drive");
        assert_eq!(found.len(), 2, "an unmounted drive is absent, not an error");

        // And a game installed there resolves by name like any other.
        std::fs::write(
            ssd.join("appmanifest_292030.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"292030\"\n\t\"name\"\t\t\"The Witcher 3\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            read_app_manifest(&ssd.join("appmanifest_292030.acf")).map(|g| g.name),
            Some("The Witcher 3".to_string())
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_manifest_is_read_and_stripped_of_its_trademark() {
        let dir = std::env::temp_dir().join(format!("aria-manifest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("appmanifest_1.acf");
        std::fs::write(&path, "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n\t\"name\"\t\t\"Overwatch®\"\n}\n").unwrap();
        assert_eq!(read_app_manifest(&path).map(|g| g.name), Some("Overwatch".to_string()));

        std::fs::write(&path, "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n}\n").unwrap();
        assert_eq!(read_app_manifest(&path), None, "a manifest with no name is not a name");
        assert_eq!(read_app_manifest(&dir.join("missing.acf")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn game(id: u32, name: &str, installdir: &str) -> Game {
        Game { id, name: name.to_string(), installdir: installdir.to_string() }
    }

    fn proc(pid: u32, parent: u32, start: u64, cmd: &str, exe: Option<&str>) -> Proc {
        Proc {
            pid,
            parent: Some(parent),
            start,
            cmd: cmd.split(' ').map(str::to_string).collect(),
            exe: exe.map(str::to_string),
        }
    }

    /// `ps -eo pid,ppid,args` taken while Overwatch ran under GE-Proton.
    fn fixture_procs() -> Vec<Proc> {
        include_str!("../../tests/fixtures/steam-ps-linux.txt")
            .lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                let pid = f.next()?.parse().ok()?;
                let parent = f.next()?.parse().ok()?;
                Some(Proc { pid, parent: Some(parent), start: 0, cmd: f.map(str::to_string).collect(), exe: None })
            })
            .collect()
    }

    #[test]
    fn a_proton_game_is_named_by_its_reaper() {
        let games = [game(2357570, "Overwatch", "Overwatch"), game(1625450, "Muck", "Muck")];
        let running = find_running(&fixture_procs(), &games, &[]).expect("a game is running");
        assert_eq!(running.game.name, "Overwatch");
        // The reaper and everything under it, but not the `sh -c` wrapper above it.
        assert_eq!(running.pids.first(), Some(&267208));
        assert!(running.pids.contains(&267376), "the Wine process is part of the game");
        assert!(!running.pids.contains(&267207));
    }

    #[test]
    fn only_the_reaper_binary_names_a_game() {
        let cmd = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(reaper_app_id(&cmd("/x/ubuntu12_32/reaper SteamLaunch AppId=730 -- /x/cs2")), Some(730));
        assert_eq!(reaper_app_id(&cmd("/bin/sh -c /x/reaper SteamLaunch AppId=730 --")), None);
        assert_eq!(reaper_app_id(&cmd("/x/reaper SteamLaunch AppId=abc --")), None);
        assert_eq!(reaper_app_id(&cmd("/x/reaper AppId=730")), None, "not a launch");
        // An `AppId=` after `--` belongs to the game's own arguments.
        assert_eq!(reaper_app_id(&cmd("/x/reaper SteamLaunch -- /x/game AppId=1")), None);
        assert_eq!(reaper_app_id(&[]), None);
    }

    #[test]
    fn an_unknown_reaper_id_still_counts_as_a_game() {
        let procs = [proc(10, 1, 0, "/x/reaper SteamLaunch AppId=3000000001 -- /opt/thing", None)];
        let running = find_running(&procs, &[], &[]).unwrap();
        assert_eq!(running.game.id, 3000000001);
        assert_eq!(running.game.name, "Steam game");
    }

    #[test]
    fn a_game_is_found_by_its_executable_on_windows_and_macos() {
        let games = [game(632360, "Risk of Rain 2", "Risk of Rain 2"), game(730, "Counter-Strike 2", "Counter-Strike Global Offensive")];
        let win = [PathBuf::from("c:/program files (x86)/steam/steamapps")];
        let procs = [
            proc(1, 0, 5, "steam.exe", Some(r"C:\Program Files (x86)\Steam\steam.exe")),
            proc(2, 1, 9, "Risk of Rain 2.exe", Some(r"C:\Program Files (x86)\Steam\steamapps\common\Risk of Rain 2\Risk of Rain 2.exe")),
        ];
        let running = find_running(&procs, &games, &win).unwrap();
        assert_eq!((running.game.id, running.pids), (632360, vec![2]));

        let mac = [PathBuf::from("/Users/u/Library/Application Support/Steam/steamapps")];
        let procs = [proc(7, 1, 3, "cs2", Some("/Users/u/Library/Application Support/Steam/steamapps/common/Counter-Strike Global Offensive/cs2.app/Contents/MacOS/cs2"))];
        assert_eq!(find_running(&procs, &games, &mac).unwrap().game.id, 730);

        // Steam itself, or a sibling folder sharing a prefix, is not a game.
        let procs = [proc(8, 1, 3, "x", Some("/Users/u/Library/Application Support/Steam/steamapps/common/Risk of Rain 2 Demo/x"))];
        assert_eq!(find_running(&procs, &games, &mac), None);
    }

    #[test]
    fn the_newest_of_two_running_games_wins() {
        let games = [game(1, "Old", "Old"), game(2, "New", "New")];
        let libs = [PathBuf::from("/s/steamapps")];
        let procs = [
            proc(1, 0, 100, "old", Some("/s/steamapps/common/Old/old")),
            proc(2, 0, 200, "new", Some("/s/steamapps/common/New/new")),
        ];
        assert_eq!(find_running(&procs, &games, &libs).unwrap().game.name, "New");
    }

    #[test]
    fn tools_are_not_games() {
        for (id, dir) in [(1493710, "Proton - Experimental"), (2805730, "Proton 9.0 (Beta)"), (1628350, "SteamLinuxRuntime_sniper"), (228980, "Steamworks Shared"), (4099820, "Ubisoft Connect Services")] {
            assert!(is_tool(&game(id, "x", dir)), "{dir}");
        }
        assert!(!is_tool(&game(2357570, "Overwatch", "Overwatch")));
    }

    /// Live: whatever this machine has installed lists without a single tool
    /// in it. Skipped where Steam isn't installed.
    #[test]
    fn installed_games_hold_no_tools() {
        let games = installed_games();
        eprintln!("{games:#?}");
        for g in &games {
            let lower = g.name.to_lowercase();
            assert!(!lower.contains("proton") && !lower.contains("runtime") && !lower.contains("redistributable"), "{g:?}");
        }
    }

    #[test]
    fn a_path_is_under_a_folder_only_at_a_separator() {
        assert!(is_under("/a/common/Game/bin/x", "/a/common/Game"));
        assert!(is_under(r"C:\A\Common\Game\x.exe", "c:/a/common/game/"));
        assert!(!is_under("/a/common/Game2/x", "/a/common/Game"));
        assert!(!is_under("/a/common/Game", "/a/common/Game"), "the folder itself is not a process");
    }
}
