//! Loading and saving the history and conflict lists.

use crate::config;
use crate::model::{ConflictRecord, HistoryEntry};

pub fn load_history() -> Vec<HistoryEntry> {
    config::read_json(&config::history_path()).unwrap_or_default()
}

pub fn save_history(entries: &[HistoryEntry]) -> Result<(), String> {
    config::write_json(&config::history_path(), &entries)
}

pub fn load_conflicts() -> Vec<ConflictRecord> {
    config::read_json(&config::conflicts_path()).unwrap_or_default()
}

pub fn save_conflicts(entries: &[ConflictRecord]) -> Result<(), String> {
    config::write_json(&config::conflicts_path(), &entries)
}
