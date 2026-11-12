//! Persistent configuration: app settings and sync tasks.
//!
//! Everything is stored as human-readable JSON in `%APPDATA%\Syncora`.
//! Writes are atomic (temp file + rename) so a crash cannot corrupt the config.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::model::TaskResult;

pub const APP_NAME: &str = "Syncora";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CONFIG_VERSION: u32 = 1;

/// Public home of the project, shown as a link on the About page.
pub const APP_REPO: &str = "https://github.com/ChathamHung/Syncora";
/// The same address without the scheme, used as the link text.
pub const APP_REPO_LABEL: &str = "github.com/ChathamHung/Syncora";

/// Directories that are never worth copying by default.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "$RECYCLE.BIN/**",
    "System Volume Information/**",
    ".syncora-tmp-*",
    "**/.syncora-tmp-*",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Source -> target only. Target extras are left alone.
    OneWay,
    /// Both directions, deletions included.
    TwoWay,
    /// Target is forced to be an exact copy of source.
    Mirror,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Record the conflict and let the user decide.
    Ask,
    /// Keep both files; the older one is renamed.
    KeepBoth,
    /// Source version always wins.
    PreferSource,
    /// Target version always wins.
    PreferTarget,
    /// Leave both files untouched until resolved manually.
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageSetting {
    Auto,
    English,
    Chinese,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeSetting {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncTask {
    pub id: String,
    pub name: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub mode: SyncMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub watch: bool,
    /// 0 means manual only.
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default)]
    pub excludes: Vec<String>,
    /// `None` inherits the global policy.
    #[serde(default)]
    pub conflict_policy: Option<ConflictPolicy>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub last_sync: Option<i64>,
    #[serde(default)]
    pub last_result: Option<TaskResult>,
    #[serde(default)]
    pub last_message: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_interval() -> u64 {
    300
}

impl SyncTask {
    pub fn new(name: String, source: PathBuf, target: PathBuf, mode: SyncMode) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            source,
            target,
            mode,
            enabled: true,
            watch: true,
            interval_secs: 300,
            excludes: Vec::new(),
            conflict_policy: None,
            created_at: now_secs(),
            last_sync: None,
            last_result: None,
            last_message: None,
        }
    }

    /// Resolve the effective conflict policy for this task.
    pub fn effective_policy(&self, global: ConflictPolicy) -> ConflictPolicy {
        self.conflict_policy.unwrap_or(global)
    }

    /// All exclude patterns including built-in safety defaults.
    pub fn all_excludes(&self) -> Vec<String> {
        let mut out: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
        for pattern in &self.excludes {
            let trimmed = pattern.trim();
            if !trimmed.is_empty() && !out.iter().any(|p| p == trimmed) {
                out.push(trimmed.to_string());
            }
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: LanguageSetting,
    pub theme: ThemeSetting,
    /// Master switch for all tasks.
    pub global_enabled: bool,
    pub start_with_windows: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub notify_on_success: bool,
    pub conflict_policy: ConflictPolicy,
    pub default_interval_secs: u64,
    pub verify_copies: bool,
    pub skip_hidden_system: bool,
    pub max_history: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: LanguageSetting::Auto,
            theme: ThemeSetting::System,
            global_enabled: true,
            start_with_windows: false,
            start_minimized: false,
            close_to_tray: true,
            notifications: true,
            notify_on_success: false,
            conflict_policy: ConflictPolicy::Ask,
            default_interval_secs: 300,
            verify_copies: false,
            skip_hidden_system: false,
            max_history: 500,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub settings: Settings,
    #[serde(default)]
    pub tasks: Vec<SyncTask>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            settings: Settings::default(),
            tasks: Vec::new(),
        }
    }
}

impl Config {
    /// Load the config, repairing obvious problems instead of failing.
    pub fn load() -> Config {
        let path = config_path();
        let Ok(text) = fs::read_to_string(&path) else {
            return Config::default();
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut config) => {
                config.normalize();
                config
            }
            Err(error) => {
                crate::util::log(&format!("config parse failed: {error}"));
                // Keep the broken file for inspection, but never block startup.
                let backup = path.with_extension("json.broken");
                let _ = fs::copy(&path, backup);
                Config::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        write_json(&config_path(), self)
    }

    /// Clamp values that could otherwise cause trouble.
    pub fn normalize(&mut self) {
        self.version = CONFIG_VERSION;
        self.settings.max_history = self.settings.max_history.clamp(100, 5000);
        if self.settings.default_interval_secs != 0 {
            self.settings.default_interval_secs =
                self.settings.default_interval_secs.clamp(10, 86_400);
        }

        let mut seen = std::collections::HashSet::new();
        self.tasks.retain(|task| !task.id.is_empty());
        for task in &mut self.tasks {
            if !seen.insert(task.id.clone()) {
                task.id = uuid::Uuid::new_v4().to_string();
                seen.insert(task.id.clone());
            }
            task.name = task.name.trim().to_string();
            if task.name.is_empty() {
                task.name = task
                    .source
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Sync task".to_string());
            }
            if task.interval_secs != 0 {
                task.interval_secs = task.interval_secs.clamp(10, 86_400);
            }
            task.excludes.retain(|p| !p.trim().is_empty());
        }
    }
}

/// A path problem that the UI can translate into the current language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathIssue {
    SourceMissing,
    SourceIsFile,
    TargetIsFile,
    Same,
    Nested,
}

/// Basic path validation shared by the editor and the engine.
pub fn check_paths(source: &Path, target: &Path) -> Result<(), PathIssue> {
    if !source.exists() {
        return Err(PathIssue::SourceMissing);
    }
    if source.is_file() {
        return Err(PathIssue::SourceIsFile);
    }
    if target.exists() && target.is_file() {
        return Err(PathIssue::TargetIsFile);
    }

    let source_abs = absolute_path(source);
    let target_abs = absolute_path(target);
    let s = path_key(&source_abs);
    let t = path_key(&target_abs);
    if s == t {
        return Err(PathIssue::Same);
    }
    if is_inside(&t, &s) || is_inside(&s, &t) {
        return Err(PathIssue::Nested);
    }
    Ok(())
}

/// Make a path absolute without requiring it to exist.
pub fn absolute_path(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_key(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    while text.len() > 3 && text.ends_with('\\') {
        text.pop();
    }
    text
}

fn is_inside(child: &str, parent: &str) -> bool {
    child == parent || child.starts_with(&format!("{parent}\\"))
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

pub fn data_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        PathBuf::from(appdata).join(APP_NAME)
    } else {
        std::env::temp_dir().join(APP_NAME)
    }
}

pub fn ensure_data_dir() -> Result<PathBuf, String> {
    let dir = data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let snapshots = dir.join("snapshots");
    fs::create_dir_all(&snapshots).map_err(|e| format!("{}: {e}", snapshots.display()))?;
    Ok(dir)
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

pub fn snapshot_path(task_id: &str) -> PathBuf {
    data_dir().join("snapshots").join(format!("{task_id}.json"))
}

pub fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

pub fn conflicts_path() -> PathBuf {
    data_dir().join("conflicts.json")
}

pub fn log_path() -> PathBuf {
    data_dir().join("syncora.log")
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
    // `rename` replaces an existing file on Windows as well.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Export writes `Config` as JSON and import parses it back, so a round trip
    /// must preserve tasks and settings.
    #[test]
    fn config_round_trips_through_json() {
        let mut config = Config::default();
        config.settings.language = LanguageSetting::Chinese;
        config.settings.theme = ThemeSetting::Dark;
        config.settings.conflict_policy = ConflictPolicy::KeepBoth;
        config.tasks.push(SyncTask::new(
            "Round trip".to_string(),
            PathBuf::from(r"C:/Source"),
            PathBuf::from(r"D:/Target"),
            SyncMode::TwoWay,
        ));

        let text = serde_json::to_string_pretty(&config).expect("serialize");
        let parsed: Config = serde_json::from_str(&text).expect("deserialize");

        assert_eq!(parsed.version, CONFIG_VERSION);
        assert_eq!(parsed.settings.language, LanguageSetting::Chinese);
        assert_eq!(parsed.settings.theme, ThemeSetting::Dark);
        assert_eq!(parsed.settings.conflict_policy, ConflictPolicy::KeepBoth);
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].mode, SyncMode::TwoWay);
        assert_eq!(parsed.tasks[0].name, "Round trip");
    }

    #[test]
    fn normalize_clamps_out_of_range_values() {
        let mut config = Config::default();
        config.settings.max_history = 1;
        config.settings.default_interval_secs = 5;
        config.tasks.push(SyncTask::new(
            "   ".to_string(),
            PathBuf::from(r"C:/A"),
            PathBuf::from(r"C:/B"),
            SyncMode::OneWay,
        ));
        config.normalize();

        assert_eq!(config.settings.max_history, 100);
        assert_eq!(config.settings.default_interval_secs, 10);
        // A blank name is replaced by the source folder name so the task list
        // never shows an empty card.
        assert_eq!(config.tasks[0].name, "A");
    }

    /// When the source path has no file name (a drive root) there is nothing to
    /// borrow, so the generic placeholder must be used instead.
    #[test]
    fn normalize_falls_back_to_placeholder_name() {
        let mut config = Config::default();
        config.tasks.push(SyncTask::new(
            "".to_string(),
            PathBuf::from(r"C:/"),
            PathBuf::from(r"D:/B"),
            SyncMode::OneWay,
        ));
        config.normalize();

        assert_eq!(config.tasks[0].name, "Sync task");
    }
}
