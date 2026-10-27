//! The local Steam install: which games are installed and what they are
//! called, read from the appmanifests Steam keeps next to every game. No
//! network and no API key; the Web API part is the STEAM widget's.

use std::path::PathBuf;

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
    use std::sync::Mutex;
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
        .find_map(|dir| read_app_manifest(&dir.join(format!("appmanifest_{id}.acf"))));
    cache.insert(id.to_string(), found.clone());
    found
}

/// Every `steamapps` directory Steam might have put a game in: the usual
/// install roots, plus whatever `libraryfolders.vdf` adds for games kept on
/// another drive.
#[cfg(target_os = "linux")]
fn libraries() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let roots = [
        home.join(".steam/steam/steamapps"),
        home.join(".steam/debian-installation/steamapps"),
        home.join(".local/share/Steam/steamapps"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam/steamapps"), // flatpak
    ];
    expand_libraries(&roots)
}

/// Every one of `roots` that exists, plus every library their
/// `libraryfolders.vdf` files point at — games kept on a second drive.
///
/// A library on an unmounted drive is simply absent rather than an error: the
/// id still matches the game trigger (that is a window class, not a file), the
/// name just falls back to the class until the drive is there.
#[cfg(not(target_os = "linux"))]
fn libraries() -> Vec<PathBuf> {
    Vec::new()
}

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

/// The `"name"` field of an appmanifest, trimmed of the trademark noise Steam
/// keeps in its titles ("Overwatch®" is not what anyone calls it).
fn read_app_manifest(path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = text.lines().find_map(|l| vdf_value(l, "name"))?;
    let name = name.trim_matches(|c: char| c == '®' || c == '™' || c.is_whitespace());
    (!name.is_empty()).then(|| name.to_string())
}

/// One line of Valve's key/value format: `\t"key"\t\t"value"`. Good enough for
/// the two flat fields read here, and never asked to parse the nested blocks.
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
            read_app_manifest(&ssd.join("appmanifest_292030.acf")),
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
        assert_eq!(read_app_manifest(&path), Some("Overwatch".to_string()));

        std::fs::write(&path, "\"AppState\"\n{\n\t\"appid\"\t\t\"1\"\n}\n").unwrap();
        assert_eq!(read_app_manifest(&path), None, "a manifest with no name is not a name");
        assert_eq!(read_app_manifest(&dir.join("missing.acf")), None);
        std::fs::remove_dir_all(&dir).ok();
    }}
