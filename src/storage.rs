//! Loading and saving settings and daily totals as a small JSON file.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::engine::{DayStats, Settings};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedData {
    pub settings: Settings,
    pub days: BTreeMap<NaiveDate, DayStats>,
}

/// `~/Library/Application Support/com.timershift.TimerShift/data.json` on macOS,
/// the XDG data dir on Linux, `%APPDATA%` on Windows. `TIMERSHIFT_DATA` overrides it.
pub fn data_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TIMERSHIFT_DATA") {
        return Some(PathBuf::from(path));
    }
    directories::ProjectDirs::from("com", "timershift", "TimerShift")
        .map(|dirs| dirs.data_dir().join("data.json"))
}

pub fn load(path: &Path) -> SavedData {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return SavedData::default(),
        Err(err) => {
            eprintln!("timershift: could not read {}: {err}", path.display());
            return SavedData::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(data) => data,
        Err(err) => {
            // Keep the unreadable file around instead of overwriting the user's history.
            let backup = path.with_extension("json.corrupt");
            eprintln!(
                "timershift: {} is not valid ({err}); moved it to {}",
                path.display(),
                backup.display()
            );
            let _ = fs::rename(path, backup);
            SavedData::default()
        }
    }
}

/// Writes atomically: a crash mid-write never leaves a truncated file behind.
pub fn save(path: &Path, data: &SavedData) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(data)?)?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let dir = std::env::temp_dir().join(format!("timershift-test-{}", std::process::id()));
        let path = dir.join("data.json");

        let mut data = SavedData::default();
        data.settings.work_minutes = 50;
        data.days.insert(
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            DayStats {
                work_ms: 5_400_000,
                sessions: 3,
            },
        );
        save(&path, &data).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.settings, data.settings);
        assert_eq!(loaded.days, data.days);

        fs::write(&path, r#"{"settings":{"break_minutes":10}}"#).unwrap();
        let partial = load(&path);
        assert_eq!(partial.settings.break_minutes, 10);
        assert_eq!(partial.settings.work_minutes, 25);
        assert!(partial.days.is_empty());

        fs::remove_dir_all(dir).unwrap();
    }
}
