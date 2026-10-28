//! System output volume for the command bar and voice: up, down, mute, or a
//! level. Each OS has its own path: WirePlumber's `wpctl` on Linux (`pactl`
//! where PipeWire's tools are missing), AppleScript on macOS, and the Core
//! Audio endpoint API on Windows.

use serde::{Deserialize, Serialize};

/// One "volume up" / "louder", in percent.
const STEP: i32 = 10;

#[derive(Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "op", content = "level", rename_all = "snake_case")]
pub enum VolumeOp {
    Up,
    Down,
    Mute,
    Unmute,
    Set(u8),
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    percent: u8,
    muted: bool,
}

/// The volume after `op`. Raising the volume also unmutes, like a keyboard's
/// volume keys; lowering it leaves mute alone.
fn apply(current: Volume, op: VolumeOp) -> Volume {
    let level = |v: i32| v.clamp(0, 100) as u8;
    match op {
        VolumeOp::Up => Volume { percent: level(current.percent as i32 + STEP), muted: false },
        VolumeOp::Down => Volume { percent: level(current.percent as i32 - STEP), muted: current.muted },
        VolumeOp::Mute => Volume { muted: true, ..current },
        VolumeOp::Unmute => Volume { muted: false, ..current },
        VolumeOp::Set(p) => Volume { percent: level(p as i32), muted: false },
    }
}

#[tauri::command]
pub async fn volume(op: VolumeOp) -> Result<Volume, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let next = apply(platform::get()?, op);
        platform::set(next)?;
        Ok(next)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Volume: 0.67" or "Volume: 0.67 [MUTED]" from `wpctl get-volume`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_wpctl(out: &str) -> Option<Volume> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let level: f32 = rest.split_whitespace().next()?.parse().ok()?;
    Some(Volume { percent: (level * 100.0).round().clamp(0.0, 100.0) as u8, muted: rest.contains("[MUTED]") })
}

/// The first "NN%" of `pactl get-sink-volume` (front-left; ARIA sets all
/// channels alike) and "Mute: yes" from `pactl get-sink-mute`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_pactl(volume: &str, mute: &str) -> Option<Volume> {
    let percent = volume
        .split_whitespace()
        .find_map(|w| w.strip_suffix('%')?.parse::<u16>().ok())?;
    Some(Volume { percent: percent.min(100) as u8, muted: mute.trim() == "Mute: yes" })
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{parse_pactl, parse_wpctl, Volume};
    use std::process::Command;

    const WP_SINK: &str = "@DEFAULT_AUDIO_SINK@";
    const PA_SINK: &str = "@DEFAULT_SINK@";

    fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
        if !out.status.success() {
            return Err(format!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn has_wpctl() -> bool {
        Command::new("wpctl").arg("--version").output().is_ok()
    }

    pub fn get() -> Result<Volume, String> {
        if has_wpctl() {
            let out = run("wpctl", &["get-volume", WP_SINK])?;
            return parse_wpctl(&out).ok_or_else(|| format!("unexpected wpctl output: {out}"));
        }
        let volume = run("pactl", &["get-sink-volume", PA_SINK])?;
        let mute = run("pactl", &["get-sink-mute", PA_SINK])?;
        parse_pactl(&volume, &mute).ok_or_else(|| format!("unexpected pactl output: {volume}"))
    }

    pub fn set(v: Volume) -> Result<(), String> {
        let percent = format!("{}%", v.percent);
        let mute = if v.muted { "1" } else { "0" };
        if has_wpctl() {
            run("wpctl", &["set-volume", WP_SINK, &percent])?;
            run("wpctl", &["set-mute", WP_SINK, mute])?;
        } else {
            run("pactl", &["set-sink-volume", PA_SINK, &percent])?;
            run("pactl", &["set-sink-mute", PA_SINK, mute])?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Volume;
    use std::process::Command;

    fn osascript(script: &str) -> Result<String, String> {
        let out = Command::new("osascript").args(["-e", script]).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn get() -> Result<Volume, String> {
        // "67, false"
        let out = osascript("set s to get volume settings\nreturn {output volume of s, output muted of s}")?;
        let (level, muted) = out.split_once(", ").ok_or_else(|| format!("unexpected osascript output: {out}"))?;
        // Output volume is "missing value" on devices without a volume control.
        let percent = level.parse::<u8>().map_err(|_| "this output device has no volume control".to_string())?;
        Ok(Volume { percent, muted: muted == "true" })
    }

    pub fn set(v: Volume) -> Result<(), String> {
        osascript(&format!("set volume output volume {}\nset volume output muted {}", v.percent, v.muted))?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::Volume;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

    fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
        unsafe {
            // Already initialised on this thread is fine; the call is per use
            // because blocking-pool threads come and go.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            devices.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)
        }
    }

    pub fn get() -> Result<Volume, String> {
        let ep = endpoint().map_err(|e| e.to_string())?;
        unsafe {
            let level = ep.GetMasterVolumeLevelScalar().map_err(|e| e.to_string())?;
            let muted = ep.GetMute().map_err(|e| e.to_string())?.as_bool();
            Ok(Volume { percent: (level * 100.0).round() as u8, muted })
        }
    }

    pub fn set(v: Volume) -> Result<(), String> {
        let ep = endpoint().map_err(|e| e.to_string())?;
        unsafe {
            ep.SetMasterVolumeLevelScalar(v.percent as f32 / 100.0, std::ptr::null())
                .map_err(|e| e.to_string())?;
            ep.SetMute(v.muted, std::ptr::null()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: fn(u8, bool) -> Volume = |percent, muted| Volume { percent, muted };

    #[test]
    fn steps_clamp_and_raising_unmutes() {
        assert_eq!(apply(V(67, false), VolumeOp::Up), V(77, false));
        assert_eq!(apply(V(95, true), VolumeOp::Up), V(100, false));
        assert_eq!(apply(V(5, true), VolumeOp::Down), V(0, true));
        assert_eq!(apply(V(40, false), VolumeOp::Mute), V(40, true));
        assert_eq!(apply(V(40, true), VolumeOp::Unmute), V(40, false));
        assert_eq!(apply(V(40, true), VolumeOp::Set(150)), V(100, false));
    }

    #[test]
    fn ops_arrive_from_the_frontend_as_tagged_json() {
        let op: VolumeOp = serde_json::from_str(r#"{"op":"set","level":30}"#).unwrap();
        assert_eq!(op, VolumeOp::Set(30));
        let op: VolumeOp = serde_json::from_str(r#"{"op":"up"}"#).unwrap();
        assert_eq!(op, VolumeOp::Up);
    }

    #[test]
    fn wpctl_output() {
        assert_eq!(parse_wpctl("Volume: 0.67\n"), Some(V(67, false)));
        assert_eq!(parse_wpctl("Volume: 0.40 [MUTED]\n"), Some(V(40, true)));
        // Above 100 % is possible in wpctl; ARIA never sets it, but reads it as 100.
        assert_eq!(parse_wpctl("Volume: 1.25"), Some(V(100, false)));
        assert_eq!(parse_wpctl("no sink"), None);
    }

    #[test]
    fn pactl_output() {
        let volume = "Volume: front-left: 43909 /  67% / -10.44 dB,   front-right: 43909 /  67% / -10.44 dB\n";
        assert_eq!(parse_pactl(volume, "Mute: no\n"), Some(V(67, false)));
        assert_eq!(parse_pactl(volume, "Mute: yes\n"), Some(V(67, true)));
        assert_eq!(parse_pactl("garbage", "Mute: no"), None);
    }
}
