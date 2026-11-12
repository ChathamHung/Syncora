//! Serializable data model shared by the UI, the engine and the store.

use serde::{Deserialize, Serialize};

use crate::config::SyncMode;

/// Final outcome of a sync run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResult {
    Success,
    Partial,
    Failed,
    Cancelled,
}

impl TaskResult {
    pub fn is_problem(self) -> bool {
        matches!(self, TaskResult::Partial | TaskResult::Failed)
    }
}

/// One row in the sync history page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: SyncMode,
    pub started_at: i64,
    pub finished_at: i64,
    pub copied: u64,
    pub updated: u64,
    pub deleted: u64,
    pub conflicts: u64,
    pub bytes: u64,
    pub errors: u64,
    pub result: TaskResult,
    pub message: Option<String>,
}

/// A file that changed on both sides and needs a decision.
///
/// `source_*` / `target_*` are `None` when the file was deleted on that side.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub rel_path: String,
    pub source_size: Option<u64>,
    pub source_mtime: Option<i64>,
    pub target_size: Option<u64>,
    pub target_mtime: Option<i64>,
    pub detected_at: i64,
}

impl ConflictRecord {
    /// Used to avoid showing the exact same conflict twice.
    pub fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{:?}|{:?}|{:?}|{:?}",
            self.task_id,
            self.rel_path,
            self.source_size,
            self.source_mtime,
            self.target_size,
            self.target_mtime
        )
    }

    pub fn source_missing(&self) -> bool {
        self.source_size.is_none()
    }

    pub fn target_missing(&self) -> bool {
        self.target_size.is_none()
    }
}
