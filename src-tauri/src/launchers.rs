//! User-defined launchers for the command bar ("open claude code in ARIA").
//!
//! The frontend only names a launcher and a folder. The program and arguments
//! come from `launchers.json` and the folder must be in the file index, so the
//! command bar can't be made to run anything else. Programs run as argv with
//! no shell: `{dir}` is replaced inside an argument (it stays one argument),
//! and the folder is also the working directory. Launchers are machine paths
//! and never enter the settings export.

use crate::files::{self, Files};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Manager, State};

const CONFIG_FILE: &str = "launchers.json";
const DIR: &str = "{dir}";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Launcher {
    name: String,
    program: String,
    args: Vec<String>,
}

#[tauri::command]
pub fn launchers_config(app: AppHandle) -> Result<Vec<Launcher>, String> {
    load(&app)
}

#[tauri::command]
pub fn launchers_set_config(app: AppHandle, launchers: Vec<Launcher>) -> Result<Vec<Launcher>, String> {
    let launchers = tidy(launchers);
    let json = serde_json::to_string_pretty(&launchers).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app)?, json).map_err(|e| e.to_string())?;
    Ok(launchers)
}

/// Where `program` would be run from, so a typo shows when it is saved.
#[tauri::command]
pub fn launchers_check(program: String) -> Result<String, String> {
    resolve(&program, &search_path()).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn launchers_run(app: AppHandle, files: State<'_, Files>, name: String, dir: String) -> Result<(), String> {
    let launcher = load(&app)?
        .into_iter()
        .find(|l| l.name.eq_ignore_ascii_case(&name))
        .ok_or("That launcher isn't saved")?;
    if !files::is_indexed_dir(&files, Path::new(&dir)) {
        return Err("That folder isn't in the search folders".into());
    }
    let path = search_path();
    let mut cmd = Command::new(resolve(&launcher.program, &path)?);
    cmd.args(fill(&launcher.args, &dir))
        .current_dir(&dir)
        .env("PATH", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Own process group: Ctrl-C on `tauri dev` or quitting ARIA leaves the terminal open.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", launcher.program))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn fill(args: &[String], dir: &str) -> Vec<String> {
    args.iter().map(|a| a.replace(DIR, dir)).collect()
}

/// PATH plus the usual user install folders. An app started from the Dock or
/// a login item gets a minimal PATH without Homebrew or `~/.local/bin`; the
/// child gets the same list, so `kitty … claude` finds `claude` too.
fn search_path() -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    #[cfg(unix)]
    {
        let mut extra = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
        if let Some(home) = std::env::var_os("HOME") {
            extra.push(Path::new(&home).join(".local/bin"));
        }
        for d in extra {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// A path is used as given; a bare name is looked up in `path` (with `.exe`
/// on Windows). Batch files are refused: Windows runs them through cmd.exe.
fn resolve(program: &str, path: &OsString) -> Result<PathBuf, String> {
    let program = program.trim();
    let given = Path::new(program);
    let found = if given.components().count() > 1 {
        Some(given.to_path_buf()).filter(|p| is_executable(p))
    } else {
        std::env::split_paths(path).find_map(|dir| {
            let mut names = vec![dir.join(program)];
            if cfg!(windows) && given.extension().is_none() {
                names.push(dir.join(format!("{program}.exe")));
            }
            names.into_iter().find(|p| is_executable(p))
        })
    };
    let found = found.ok_or_else(|| format!("{program} isn't installed or isn't a program"))?;
    let ext = found.extension().map(|e| e.to_string_lossy().to_lowercase());
    if matches!(ext.as_deref(), Some("bat" | "cmd")) {
        return Err(format!("{program} is a batch file; use the program it starts instead"));
    }
    Ok(found)
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    meta.is_file()
}

/// Trimmed, named, one launcher per name (the command bar matches names).
fn tidy(launchers: Vec<Launcher>) -> Vec<Launcher> {
    let mut out: Vec<Launcher> = Vec::new();
    for l in launchers {
        let l = Launcher { name: l.name.trim().to_string(), program: l.program.trim().to_string(), args: l.args };
        if l.name.is_empty() || l.program.is_empty() || out.iter().any(|o| o.name.eq_ignore_ascii_case(&l.name)) {
            continue;
        }
        out.push(l);
    }
    out
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(CONFIG_FILE))
}

fn load(app: &AppHandle) -> Result<Vec<Launcher>, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| format!("{CONFIG_FILE} is invalid: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(name: &str, program: &str) -> Launcher {
        Launcher { name: name.into(), program: program.into(), args: Vec::new() }
    }

    fn bin_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aria-launchers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fill_keeps_a_hostile_folder_one_argument() {
        let args = vec!["--directory".to_string(), "{dir}".to_string(), "--title=ARIA {dir}".to_string()];
        let dir = "/home/t/x; rm -rf ~";
        assert_eq!(fill(&args, dir), ["--directory", dir, "--title=ARIA /home/t/x; rm -rf ~"]);
    }

    #[test]
    fn tidy_trims_and_keeps_one_launcher_per_name() {
        let got = tidy(vec![
            launcher(" Claude Code ", " kitty "),
            launcher("claude code", "wezterm"),
            launcher("", "kitty"),
            launcher("empty", " "),
        ]);
        assert_eq!(got, [launcher("Claude Code", "kitty")]);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_finds_executables_on_the_path_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = bin_dir("resolve");
        for (name, mode) in [("tool", 0o755), ("data", 0o644)] {
            let p = dir.join(name);
            std::fs::write(&p, "").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(resolve("tool", &path).unwrap(), dir.join("tool"));
        assert_eq!(resolve(dir.join("tool").to_str().unwrap(), &OsString::new()).unwrap(), dir.join("tool"));
        assert!(resolve("data", &path).is_err());
        assert!(resolve("missing", &path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_refuses_batch_files() {
        let dir = bin_dir("batch");
        let p = dir.join("code.cmd");
        std::fs::write(&p, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let err = resolve(p.to_str().unwrap(), &OsString::new()).unwrap_err();
        assert!(err.contains("batch file"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
