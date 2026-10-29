//! File and folder search for the command bar ("open thesis", "open the ARIA
//! folder").
//!
//! Names only, in memory: a walk of the user's search folders, rebuilt in the
//! background when a search finds the index older than `STALE`. This module
//! only narrows the index to a few dozen candidates; the frontend matcher
//! ranks them with the same scoring as apps and games.
//!
//! `files_open` and `files_reveal` accept only indexed paths, so a command
//! can't be made to open an arbitrary file. The folders are machine paths and
//! live in app_config_dir, outside the settings export. Search is off until
//! the user switches it on: on macOS the first walk of Documents, Downloads
//! and Desktop raises privacy prompts, which should come at that moment.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const CONFIG_FILE: &str = "search-folders.json";
const STALE: Duration = Duration::from_secs(10 * 60);
/// First build after launch waits for the collectors' startup burst.
const STARTUP_DELAY: Duration = Duration::from_secs(30);
const MAX_DEPTH: usize = 8;
const MAX_ENTRIES: usize = 100_000;
const MAX_HITS: usize = 30;
/// Query words are matched by their first letters, so a typo later in the
/// word ("thesus") still finds the file and the matcher sorts it out.
const STEM: usize = 3;
/// Build output and caches; hidden folders (.git, .venv) are skipped anyway.
const SKIP_DIRS: [&str; 3] = ["node_modules", "target", "__pycache__"];
/// Words of the command phrases. They would otherwise pull in every file
/// whose name starts with "open" or "show" ahead of the one being asked for.
const COMMAND_WORDS: [&str; 8] = ["open", "show", "reveal", "the", "folder", "latest", "last", "download"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Config {
    enabled: bool,
    folders: Vec<PathBuf>,
}

struct Entry {
    /// Folded name split into words, joined by single spaces.
    key: String,
    path: PathBuf,
    root: usize,
    dir: bool,
    /// Seconds since the Unix epoch.
    mtime: u64,
}

#[derive(Default)]
struct Index {
    roots: Vec<PathBuf>,
    entries: Vec<Entry>,
    built: Option<Instant>,
    /// The walk stopped at the entry limit, so some names are missing.
    capped: bool,
}

#[derive(Default)]
pub struct Files {
    index: RwLock<Index>,
    building: AtomicBool,
    /// A rebuild was asked for while one ran (e.g. the folders changed).
    again: AtomicBool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Hit {
    name: String,
    path: String,
    /// Root folder name plus the path below it: "Documents/school".
    folder: String,
    dir: bool,
    mtime: u64,
}

#[derive(Serialize)]
pub struct Status {
    entries: usize,
    /// Seconds since the last finished build; none before the first.
    age: Option<u64>,
    building: bool,
    capped: bool,
}

pub fn init(app: &AppHandle) {
    app.manage(Files::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        rebuild(&app);
    });
}

#[tauri::command]
pub async fn files_search(app: AppHandle, text: String) -> Result<Vec<Hit>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let files = app.state::<Files>();
        let stale = files.index.read().unwrap().built.is_none_or(|t| t.elapsed() >= STALE);
        if stale {
            rebuild(&app);
        }
        let index = files.index.read().unwrap();
        search(&index, &text)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn files_latest(app: AppHandle, files: State<'_, Files>) -> Option<Hit> {
    let downloads = app.path().download_dir().ok()?;
    let index = files.index.read().unwrap();
    let root = index.roots.iter().position(|r| *r == downloads)?;
    let latest = index.entries.iter().filter(|e| e.root == root && !e.dir).max_by_key(|e| e.mtime)?;
    Some(hit(&index, latest))
}

#[tauri::command]
pub fn files_open(app: AppHandle, files: State<'_, Files>, path: String) -> Result<(), String> {
    indexed(&files, &path)?;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn files_reveal(app: AppHandle, files: State<'_, Files>, path: String) -> Result<(), String> {
    indexed(&files, &path)?;
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn files_config(app: AppHandle) -> Result<Config, String> {
    load_config(&app)
}

#[tauri::command]
pub fn files_set_config(app: AppHandle, files: State<'_, Files>, config: Config) -> Result<Config, String> {
    let config = Config { enabled: config.enabled, folders: tidy_folders(config.folders) };
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app)?, json).map_err(|e| e.to_string())?;
    // Turning search off forgets the names at once; any change rebuilds.
    *files.index.write().unwrap() = Index::default();
    rebuild(&app);
    Ok(config)
}

#[tauri::command]
pub fn files_status(files: State<'_, Files>) -> Status {
    let index = files.index.read().unwrap();
    Status {
        entries: index.entries.len(),
        age: index.built.map(|t| t.elapsed().as_secs()),
        building: files.building.load(Ordering::Relaxed),
        capped: index.capped,
    }
}

fn indexed(files: &Files, path: &str) -> Result<(), String> {
    let index = files.index.read().unwrap();
    if index.entries.iter().any(|e| e.path == Path::new(path)) {
        Ok(())
    } else {
        Err("That file isn't in the search folders".into())
    }
}

/// Walks the folders on a blocking thread. One build at a time; a request
/// during a build runs another one after it, so a folder change isn't lost.
fn rebuild(app: &AppHandle) {
    let files = app.state::<Files>();
    if files.building.swap(true, Ordering::AcqRel) {
        files.again.store(true, Ordering::Release);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let files = app.state::<Files>();
        let config = load_config(&app).unwrap_or_else(|e| {
            eprintln!("file search config: {e}");
            Config { enabled: false, folders: Vec::new() }
        });
        // Off still counts as built, so searches don't retry every keystroke.
        let index = if config.enabled { build(config.folders, MAX_ENTRIES) } else { Index { built: Some(Instant::now()), ..Index::default() } };
        *files.index.write().unwrap() = index;
        files.building.store(false, Ordering::Release);
        if files.again.swap(false, Ordering::AcqRel) {
            rebuild(&app);
        }
    });
}

fn build(roots: Vec<PathBuf>, max: usize) -> Index {
    let mut entries = Vec::new();
    for (i, root) in roots.iter().enumerate() {
        // The root itself, so "open Dev folder" finds the folder that was added.
        if let (Some(name), Ok(meta)) = (root.file_name(), root.metadata()) {
            entries.push(Entry { key: words(&name.to_string_lossy()), path: root.clone(), root: i, dir: true, mtime: mtime(&meta) });
        }
        walk(root, i, MAX_DEPTH, max, &mut entries);
    }
    let capped = entries.len() >= max;
    Index { roots, entries, built: Some(Instant::now()), capped }
}

/// Symlinks are not followed: a link back up the tree would loop, and one out
/// of the folder would index what the user didn't choose.
fn walk(dir: &Path, root: usize, depth: usize, max: usize, out: &mut Vec<Entry>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= max {
            return;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.file_type().is_symlink() || is_hidden(&name, &meta) {
            continue;
        }
        let dir = meta.is_dir();
        if dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let mtime = mtime(&meta);
        let path = entry.path();
        // A macOS bundle is one thing to open, not a folder to search in.
        let descend = dir && depth > 1 && !name.ends_with(".app");
        if descend {
            walk(&path, root, depth - 1, max, out);
            if out.len() >= max {
                return;
            }
        }
        out.push(Entry { key: words(&name), path, root, dir, mtime });
    }
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        if meta.file_attributes() & (HIDDEN | SYSTEM) != 0 {
            return true;
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = meta;
    name.starts_with('.')
}

/// Lowercase, no diacritics, split into words at punctuation, spaces and
/// letter/digit boundaries: "Thesis_v3-final.PDF" → "thesis v 3 final pdf".
fn words(s: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase) {
        if !c.is_alphanumeric() {
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            if p.is_numeric() != c.is_numeric() {
                out.push(' ');
            }
        } else if !out.is_empty() {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// The first letters of each query word long enough to have them; command
/// words are dropped unless nothing else is left.
fn stems(text: &str) -> Vec<String> {
    let key = words(text);
    let long: Vec<&str> = key.split(' ').filter(|w| w.chars().count() >= STEM).collect();
    let named: Vec<&str> = long.iter().copied().filter(|w| !COMMAND_WORDS.contains(w)).collect();
    let mut out: Vec<String> = if named.is_empty() { long } else { named }
        .into_iter()
        .map(|w| w.chars().take(STEM).collect())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Entries with a word starting like a query word, most such words first,
/// newest first among equals.
fn search(index: &Index, text: &str) -> Vec<Hit> {
    let stems = stems(text);
    if stems.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &Entry)> = index
        .entries
        .iter()
        .filter_map(|e| {
            let n = stems.iter().filter(|s| e.key.split(' ').any(|w| w.starts_with(s.as_str()))).count();
            (n > 0).then_some((n, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.mtime.cmp(&a.1.mtime)));
    scored.into_iter().take(MAX_HITS).map(|(_, e)| hit(index, e)).collect()
}

fn hit(index: &Index, e: &Entry) -> Hit {
    let root = &index.roots[e.root];
    // A root is labelled by where it lives: "Dev — /home/t".
    if e.path == *root {
        return Hit {
            name: root.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            path: root.to_string_lossy().into_owned(),
            folder: root.parent().map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
            dir: true,
            mtime: e.mtime,
        };
    }
    let mut folder = root.file_name().map_or_else(|| root.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
    if let Some(rel) = e.path.parent().and_then(|p| p.strip_prefix(root).ok()) {
        for part in rel.components() {
            folder.push('/');
            folder.push_str(&part.as_os_str().to_string_lossy());
        }
    }
    Hit {
        name: e.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        path: e.path.to_string_lossy().into_owned(),
        folder,
        dir: e.dir,
        mtime: e.mtime,
    }
}

/// Existing absolute folders, each once, none inside another listed one
/// (it would be walked twice).
fn tidy_folders(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for f in folders {
        if !f.is_absolute() || !f.is_dir() || out.contains(&f) {
            continue;
        }
        out.retain(|o| !o.starts_with(&f));
        if !out.iter().any(|o| f.starts_with(o)) {
            out.push(f);
        }
    }
    out
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(CONFIG_FILE))
}

/// No file yet: off, with Documents, Downloads and Desktop ready to switch on.
fn load_config(app: &AppHandle) -> Result<Config, String> {
    let path = config_path(app)?;
    if !path.exists() {
        let p = app.path();
        let folders = [p.document_dir(), p.download_dir(), p.desktop_dir()].into_iter().flatten().collect();
        return Ok(Config { enabled: false, folders: tidy_folders(folders) });
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("{CONFIG_FILE} is invalid: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("aria-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["school/thesis", "node_modules/pkg", ".git", "a/b/c"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "Thesis_v3-final.pdf",
            "school/thesis/Diplomová práce.docx",
            "school/budget.xls",
            "node_modules/pkg/thesis.js",
            ".git/thesis",
            ".hidden-thesis.txt",
            "a/b/c/deep.txt",
            "Open day.txt",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        root
    }

    fn names(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|h| h.name.as_str()).collect()
    }

    #[test]
    fn words_folds_case_diacritics_and_splits_digits() {
        assert_eq!(words("Thesis_v3-final.PDF"), "thesis v 3 final pdf");
        assert_eq!(words("Diplomová práce"), "diplomova prace");
        assert_eq!(words("CS2 2024.xls"), "cs 2 2024 xls");
    }

    #[test]
    fn stems_drop_command_words_unless_alone() {
        assert_eq!(stems("open the thesis"), ["the"]);
        assert_eq!(stems("open xyz.xls"), ["xls", "xyz"]);
        assert_eq!(stems("open"), ["ope"]);
        assert!(stems("a b").is_empty());
    }

    #[test]
    fn walk_skips_hidden_and_build_folders() {
        let root = tree("walk");
        let index = build(vec![root.clone()], MAX_ENTRIES);
        let hits = search(&index, "thesis");
        let mut found = names(&hits);
        found.sort();
        assert_eq!(found, ["Thesis_v3-final.pdf", "thesis"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn walk_stops_at_the_depth_limit() {
        let root = tree("depth");
        let mut out = Vec::new();
        walk(&root, 0, 3, MAX_ENTRIES, &mut out);
        assert!(out.iter().any(|e| e.path.ends_with("a/b/c")));
        assert!(!out.iter().any(|e| e.path.ends_with("deep.txt")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_finds_a_root_folder_itself() {
        let root = tree("root");
        let index = build(vec![root.join("school")], MAX_ENTRIES);
        let hits = search(&index, "school");
        assert_eq!(hits[0].path, root.join("school").to_string_lossy());
        assert!(hits[0].dir);
        assert_eq!(hits[0].folder, root.to_string_lossy());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn build_reports_the_entry_cap() {
        let root = tree("cap");
        assert!(!build(vec![root.clone()], MAX_ENTRIES).capped);
        let index = build(vec![root.clone()], 3);
        assert!(index.capped);
        assert_eq!(index.entries.len(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_ranks_by_matched_words_and_names_the_folder() {
        let root = tree("search");
        let index = build(vec![root.clone()], MAX_ENTRIES);
        let prace = search(&index, "open prace");
        assert_eq!(names(&prace), ["Diplomová práce.docx"]);
        let leaf = root.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(prace[0].folder, format!("{leaf}/school/thesis"));

        let thesis = search(&index, "thesis final");
        assert_eq!(thesis[0].name, "Thesis_v3-final.pdf");
        // Command words only count when nothing else was said.
        assert_eq!(names(&search(&index, "open")), ["Open day.txt"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tidy_folders_drops_missing_duplicate_and_nested() {
        let root = tree("tidy");
        let school = root.join("school");
        let got = tidy_folders(vec![
            school.clone(),
            root.clone(),
            school,
            root.join("missing"),
            PathBuf::from("relative"),
        ]);
        assert_eq!(got, [root.clone()]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
