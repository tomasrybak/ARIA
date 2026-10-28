//! Command bar: a small always-on-top window over every app. The HUD lives on
//! the desktop layer and never has keyboard focus, so typed commands need a
//! window of their own.
//!
//! The palette is only a view. It sends what the user types to the main
//! window, which owns the action registry and every collector's latest data,
//! and shows the answer that comes back.
//!
//! It is created on first use and then only hidden, so users who never open it
//! don't pay for a second webview, and later opens are instant.

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

const LABEL: &str = "palette";
/// Cmd+Option+K on macOS. Plain Ctrl+K would take Discord's quick switcher,
/// Firefox's search box and VS Code's chords away while ARIA runs.
const HOTKEY: &str = "CommandOrControl+Alt+K";
/// Opens the palette in a running ARIA (or starts one). Wayland has no global
/// hotkeys for apps, so there the user binds this as a custom shortcut.
const COMMAND_ARG: &str = "--command";
const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 64.0;

pub fn init(app: &AppHandle) {
    if crate::window::display_server() != "wayland" {
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
        let registered = app.global_shortcut().on_shortcut(HOTKEY, |app, _, event| {
            if event.state == ShortcutState::Pressed {
                toggle(app);
            }
        });
        if let Err(e) = registered {
            eprintln!("command bar hotkey {HOTKEY} not registered: {e}");
        }
    }
    handle_args(app, &std::env::args().collect::<Vec<_>>());
}

/// Arguments of this launch or, through single-instance, of a later one.
pub fn handle_args(app: &AppHandle, args: &[String]) {
    if wants_palette(args) {
        show(app);
    }
}

/// The HUD's own command bar button: the only way in on a Wayland session
/// before the user has bound `aria --command` to a shortcut.
#[tauri::command]
pub fn palette_open(app: AppHandle) {
    show(&app);
}

fn wants_palette(args: &[String]) -> bool {
    args.iter().skip(1).any(|a| a == COMMAND_ARG)
}

fn toggle(app: &AppHandle) {
    match app.get_webview_window(LABEL) {
        Some(w) if w.is_visible().unwrap_or(false) => {
            let _ = w.hide();
        }
        _ => show(app),
    }
}

fn show(app: &AppHandle) {
    let window = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => match build(app) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("command bar window failed: {e}");
                return;
            }
        },
    };
    if crate::window::display_server() != "wayland" {
        center_on_hud(app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
    use tauri::Emitter;
    let _ = window.emit_to(LABEL, "palette-open", ());
}

/// Centre on the monitor the HUD is on, a third of the way down (the list
/// grows downwards). Wayland ignores positions; see `build` for that path.
fn center_on_hud(app: &AppHandle, window: &WebviewWindow) {
    let monitor = app.get_webview_window("main").and_then(|m| m.current_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        let _ = window.center();
        return;
    };
    let area = monitor.work_area();
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + (area.size.height as i32 - size.height as i32) / 3;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    // Opaque on purpose: transparent WebKitGTK windows on NVIDIA paint damaged
    // regions black (see the Linux workaround notes in window.rs).
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("palette.html".into()));
    // Wayland lets no app place its windows. A dialog of the HUD is placed by
    // the compositor over its parent, and the HUD fills its monitor. Only
    // there: on macOS a child window would share the HUD's desktop level.
    #[cfg(target_os = "linux")]
    let builder = match app.get_webview_window("main") {
        Some(main) if crate::window::display_server() == "wayland" => builder.transient_for(&main)?,
        _ => builder,
    };
    let window = builder
        .title("ARIA")
        .inner_size(WIDTH, HEIGHT)
        // Borderless, so nothing to drag; and a non-resizable GTK window
        // sticks at its natural 200 px and ignores fit-to-content resizes.
        .resizable(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;
    #[cfg(target_os = "linux")]
    if crate::window::display_server() == "wayland" {
        use gtk::prelude::GtkWindowExt;
        if let Ok(gtk_window) = window.gtk_window() {
            gtk_window.set_type_hint(gtk::gdk::WindowTypeHint::Dialog);
        }
    }
    let w = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = w.hide();
        }
    });
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn command_arg_opens_the_palette() {
        assert!(wants_palette(&args(&["aria", "--command"])));
        assert!(wants_palette(&args(&["/usr/bin/aria", "--minimized", "--command"])));
    }

    #[test]
    fn other_launches_do_not() {
        assert!(!wants_palette(&args(&["aria"])));
        assert!(!wants_palette(&args(&["aria", "--minimized"])));
        // argv[0] is the program, never an option.
        assert!(!wants_palette(&args(&["--command"])));
    }
}
