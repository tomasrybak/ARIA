//! Installed apps for the command bar and voice ("open Firefox").
//!
//! The frontend only ever gets names and opaque ids, and `apps_launch` starts
//! an id only if it is still in the current list, so a command can't be made
//! to run an arbitrary path. Linux asks GIO, which already applies the
//! desktop-entry rules (NoDisplay, OnlyShowIn, Flatpak exports, Exec field
//! codes); macOS lists `.app` bundles and Windows the Start Menu shortcuts,
//! both opened like a double-click through the opener plugin.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct App {
    name: String,
    /// Linux: desktop file id ("firefox.desktop"); macOS/Windows: the path.
    id: String,
}

#[tauri::command]
pub async fn apps_list() -> Result<Vec<App>, String> {
    tauri::async_runtime::spawn_blocking(|| tidy(platform::list()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apps_launch(app: AppHandle, id: String) -> Result<(), String> {
    let known = tauri::async_runtime::spawn_blocking(platform::list).await.map_err(|e| e.to_string())?;
    if !known.iter().any(|a| a.id == id) {
        return Err("That app isn't installed".into());
    }
    platform::launch(&app, &id)
}

/// Sorted by name, one entry per name: a system and a user copy of the same
/// app (or a Start Menu shortcut in both the user and the all-users folder)
/// would otherwise make every "open X" ambiguous.
fn tidy(mut apps: Vec<App>) -> Vec<App> {
    apps.retain(|a| !a.name.trim().is_empty());
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by_key(|a| a.name.to_lowercase());
    apps
}

/// Start Menu entries that are not apps you'd ask to open.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_shortcut_noise(name: &str) -> bool {
    let n = name.to_lowercase();
    ["uninstall", "readme", "release notes", "documentation", "help", "website"]
        .iter()
        .any(|w| n.contains(w))
}

/// Every file under `dir` (recursively, `depth` levels) with extension `ext`,
/// named after its file stem.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn files_named(dir: &std::path::Path, ext: &str, depth: usize, out: &mut Vec<App>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
            if let Some(stem) = path.file_stem() {
                out.push(App { name: stem.to_string_lossy().into_owned(), id: path.to_string_lossy().into_owned() });
            }
        } else if depth > 0 && path.is_dir() {
            files_named(&path, ext, depth - 1, out);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::App;
    use gtk::gio::{self, prelude::*};
    use tauri::AppHandle;

    pub fn list() -> Vec<App> {
        gio::AppInfo::all()
            .into_iter()
            .filter(|a| a.should_show())
            .filter_map(|a| Some(App { name: a.display_name().to_string(), id: a.id()?.to_string() }))
            .collect()
    }

    pub fn launch(_app: &AppHandle, id: &str) -> Result<(), String> {
        let info = gio::DesktopAppInfo::new(id).ok_or("That app isn't installed")?;
        info.launch(&[], gio::AppLaunchContext::NONE).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{files_named, App};
    use std::path::PathBuf;
    use tauri::AppHandle;
    use tauri_plugin_opener::OpenerExt;

    pub fn list() -> Vec<App> {
        let mut dirs = vec![PathBuf::from("/Applications"), PathBuf::from("/System/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
        let mut out = Vec::new();
        // One level down covers Utilities and vendor folders; an .app is a
        // directory too, but files_named stops at a match, not inside it.
        for dir in dirs {
            files_named(&dir, "app", 1, &mut out);
        }
        out
    }

    pub fn launch(app: &AppHandle, id: &str) -> Result<(), String> {
        app.opener().open_path(id, None::<&str>).map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{files_named, is_shortcut_noise, App};
    use std::path::PathBuf;
    use tauri::AppHandle;
    use tauri_plugin_opener::OpenerExt;

    const START_MENU: &str = r"Microsoft\Windows\Start Menu\Programs";

    pub fn list() -> Vec<App> {
        let mut out = Vec::new();
        for var in ["APPDATA", "PROGRAMDATA"] {
            if let Some(base) = std::env::var_os(var) {
                files_named(&PathBuf::from(base).join(START_MENU), "lnk", 3, &mut out);
            }
        }
        out.retain(|a| !is_shortcut_noise(&a.name));
        out
    }

    pub fn launch(app: &AppHandle, id: &str) -> Result<(), String> {
        app.opener().open_path(id, None::<&str>).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> App {
        App { name: name.into(), id: id.into() }
    }

    #[test]
    fn tidy_sorts_and_keeps_one_per_name() {
        let apps = tidy(vec![
            app("firefox", "/usr/share/applications/firefox.desktop"),
            app("Discord", "discord.desktop"),
            app("Firefox", "org.mozilla.firefox.desktop"),
            app(" ", "blank.desktop"),
        ]);
        let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Discord", "firefox"]);
    }

    #[test]
    fn shortcut_noise() {
        assert!(is_shortcut_noise("Uninstall Discord"));
        assert!(is_shortcut_noise("Steam Help"));
        assert!(!is_shortcut_noise("Discord"));
    }

    #[test]
    fn files_named_walks_to_the_given_depth() {
        let root = std::env::temp_dir().join(format!("aria-apps-test-{}", std::process::id()));
        let deep = root.join("Vendor").join("Sub");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join("Top.lnk"), "").unwrap();
        std::fs::write(root.join("Vendor").join("Mid.LNK"), "").unwrap();
        std::fs::write(deep.join("Deep.lnk"), "").unwrap();
        std::fs::write(root.join("notes.txt"), "").unwrap();

        let mut found = Vec::new();
        files_named(&root, "lnk", 1, &mut found);
        let mut names: Vec<_> = found.iter().map(|a| a.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["Mid", "Top"]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
