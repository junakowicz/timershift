//! Desktop notifications and the end-of-period chime.
//!
//! On macOS the app plays the chime itself: notification sounds can't use the
//! system sounds, and macOS mutes notifications while you're using the app.
//! Elsewhere the notification carries the sound.

use std::sync::OnceLock;

#[cfg(target_os = "macos")]
const CHIME: &str = "/System/Library/Sounds/Glass.aiff";
#[cfg(target_os = "windows")]
const SOUND: &str = "Default";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SOUND: &str = "complete";

/// Whether the sound plays even with notifications turned off.
pub const CHIME_WITHOUT_NOTIFICATIONS: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy)]
enum Backend {
    /// notify-rust: D-Bus on Linux, toast notifications on Windows, and on macOS
    /// the UserNotifications framework under TimerShift.app's own identity.
    Native,
    /// macOS refuses notifications from a bare binary (e.g. `cargo run`),
    /// so ask AppleScript to post them instead.
    #[cfg(target_os = "macos")]
    AppleScript,
}

static BACKEND: OnceLock<Backend> = OnceLock::new();

/// Call once at startup, before the first notification is sent.
pub fn init() {
    BACKEND.get_or_init(choose_backend);
}

/// Shows a notification and/or plays the chime, off the calling thread
/// (delivery can block for a moment: D-Bus, osascript, waiting for the sound).
pub fn announce(title: &str, body: &str, notification: bool, sound: bool) {
    let backend = *BACKEND.get_or_init(choose_backend);
    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::spawn(move || {
        if notification {
            let result = match backend {
                Backend::Native => send_native(&title, &body, sound),
                #[cfg(target_os = "macos")]
                Backend::AppleScript => send_applescript(&title, &body),
            };
            if let Err(err) = result {
                eprintln!("timershift: could not show notification: {err}");
            }
        }
        #[cfg(target_os = "macos")]
        if sound {
            play_chime();
        }
    });
}

#[cfg(target_os = "macos")]
fn choose_backend() -> Backend {
    // UNUserNotificationCenter only works (and only doesn't crash) inside an app bundle.
    if notify_rust::check_bundle().is_err() {
        return Backend::AppleScript;
    }
    // Shows the permission prompt the first time; it waits for the user, so not on the UI thread.
    std::thread::spawn(|| match notify_rust::request_auth_blocking() {
        Ok(true) => {}
        Ok(false) => eprintln!("timershift: notifications are turned off in System Settings"),
        Err(err) => eprintln!("timershift: could not ask for notification permission: {err}"),
    });
    Backend::Native
}

#[cfg(not(target_os = "macos"))]
fn choose_backend() -> Backend {
    Backend::Native
}

#[cfg_attr(target_os = "macos", allow(unused_variables))]
fn send_native(title: &str, body: &str, sound: bool) -> Result<(), String> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(title).body(body).appname("TimerShift");
    #[cfg(not(target_os = "macos"))]
    if sound {
        notification.sound_name(SOUND);
    }
    notification.show().map(drop).map_err(|err| err.to_string())
}

#[cfg(target_os = "macos")]
fn send_applescript(title: &str, body: &str) -> Result<(), String> {
    // The text goes in as arguments, never spliced into the script source.
    let status = std::process::Command::new("osascript")
        .args(["-e", "on run argv"])
        .args([
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
        ])
        .args(["-e", "end run", title, body])
        .status()
        .map_err(|err| err.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("osascript exited with {status}"))
    }
}

#[cfg(target_os = "macos")]
fn play_chime() {
    match std::process::Command::new("afplay").arg(CHIME).status() {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("timershift: afplay exited with {status}"),
        Err(err) => eprintln!("timershift: could not play the chime: {err}"),
    }
}
