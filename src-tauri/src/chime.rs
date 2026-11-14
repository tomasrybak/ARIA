//! The timer chime. Played here rather than in the webview: WebKit only lets a
//! page make sound after a click in it, and timers also start from the command
//! bar or survive a restart.
//!
//! The output stream is opened per chime and closed right after, so no stream
//! stays open between chimes: screen time reads an open one as media playing
//! (Presence::Media in screentime.rs).

use std::thread;
use std::time::Duration;

use rodio::source::{SineWave, Source};

const TONES_HZ: [f32; 2] = [880.0, 1320.0];
const TONE: Duration = Duration::from_millis(200);
const GAP: Duration = Duration::from_millis(220);
const ATTACK: Duration = Duration::from_millis(20);
const VOLUME: f32 = 0.25;
// Last tone ends at GAP + TONE; the rest lets the device drain before the
// stream closes.
const PLAY_FOR: Duration = Duration::from_millis(600);

/// Returns at once; the chime plays on its own thread. A machine without an
/// audio device just stays silent: the toast already says the timer ended.
#[tauri::command]
pub fn chime() {
    thread::spawn(|| {
        if let Err(e) = play() {
            eprintln!("chime: {e}");
        }
    });
}

fn play() -> Result<(), rodio::DeviceSinkError> {
    let mut sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    sink.log_on_drop(false);
    for (i, freq) in TONES_HZ.into_iter().enumerate() {
        let tone = SineWave::new(freq)
            .take_duration(TONE)
            .amplify(VOLUME)
            .fade_in(ATTACK)
            .fade_out(TONE)
            .delay(GAP * i as u32);
        sink.mixer().add(tone);
    }
    thread::sleep(PLAY_FOR);
    Ok(())
}
