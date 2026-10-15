# TimerShift

A small work/break interval timer for the desktop, written in Rust with
[egui/eframe](https://github.com/emilk/egui). It runs on macOS, Windows and Linux.

- Start, pause/resume, reset or skip the current period (Space toggles start/pause)
- Adjustable work and break lengths, with 25/5, 50/10 and 90/20 presets
- Desktop notification (with sound) when a work period or break ends, plus a dock bounce
- Breaks start on their own; the next work period waits for you (both configurable)
- Counts the hours you work each day, with a 7-day history, weekly total and all-time total
- Menu bar icon (system tray on Windows) that shows the period at a glance:
  a red ring filling up during work, green with a pulsing dot during a break,
  and 30 seconds of blinking when a period ends. Click it for Start/Pause, Skip,
  Open and Quit. Optionally shows the remaining time next to the icon (macOS)

Only time spent in a *running work period* counts towards your hours. Paused time,
breaks and time while the computer is asleep do not.

## Download

Get the latest version from the [releases page](https://github.com/junakowicz/timershift/releases/latest):

- **macOS** (Apple Silicon and Intel): [TimerShift-macos-universal.zip](https://github.com/junakowicz/timershift/releases/latest/download/TimerShift-macos-universal.zip).
  Unzip it and drag TimerShift.app into Applications. The app isn't notarized,
  so macOS blocks the first launch: click **Open Anyway** in System Settings ›
  Privacy & Security, or run `xattr -dr com.apple.quarantine /Applications/TimerShift.app`.
- **Windows**: [TimerShift-windows-x86_64.zip](https://github.com/junakowicz/timershift/releases/latest/download/TimerShift-windows-x86_64.zip)
- **Linux**: [TimerShift-linux-x86_64.tar.gz](https://github.com/junakowicz/timershift/releases/latest/download/TimerShift-linux-x86_64.tar.gz)

## Releasing

Bump `version` in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

GitHub Actions builds all three platforms and publishes the release.

## Build from source

You need a Rust toolchain ([rustup](https://rustup.rs)).

```bash
cargo run --release
```

### macOS app bundle (recommended on macOS)

macOS only shows notifications from real app bundles. The script below builds
`TimerShift.app` with its own identity and icon:

```bash
./scripts/bundle-macos.sh
```

```bash
cp -R target/release/bundle/TimerShift.app /Applications/
```

On first launch macOS asks whether TimerShift may send notifications. Allow it.
In System Settings › Notifications › TimerShift you can switch the style to
*Alerts* so reminders stay on screen until you dismiss them. Notifications also
show while TimerShift is the active app.

On macOS the app plays the chime (the system "Glass" sound) itself, so you hear
it even when notifications are off or muted. Settings › Test notification and
sound checks both.

When run as a bare binary (`cargo run`), notifications go through AppleScript
instead and appear as coming from Script Editor.

### Linux

Building needs the usual windowing development packages, for example on Debian/Ubuntu:

```bash
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

Notifications use the freedesktop D-Bus notification service your desktop provides.
The tray icon is not available on Linux yet.

### Windows

`cargo build --release` produces `target\release\timershift.exe`. Notifications
are standard toast notifications.

## Data

Settings and daily totals are stored as JSON:

| OS      | Location |
|---------|----------|
| macOS   | `~/Library/Application Support/com.timershift.TimerShift/data.json` |
| Linux   | `~/.local/share/timershift/data.json` |
| Windows | `%APPDATA%\timershift\TimerShift\data\data.json` |

Set `TIMERSHIFT_DATA=/some/file.json` to use a different file.

## Code layout

- `src/engine.rs`: timer state machine and per-day stats (pure logic, unit-tested)
- `src/app.rs`: the egui UI and the background thread that ticks the timer,
  so periods end on time even while the window is hidden
- `src/notify.rs`: desktop notifications per platform
- `src/tray.rs`: the menu bar / system tray icon and its menu
- `src/storage.rs`: loading and saving the JSON data file
- `src/icon.rs`: the app icon and menu bar icon frames, drawn in code

```bash
cargo test
```
