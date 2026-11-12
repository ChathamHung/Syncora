//! The Syncora interface.
//!
//! The UI owns the configuration and the history. All disk work happens in the
//! engine; this file only sends commands and renders events.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{
    Align, Align2, Button, Color32, FontId, Frame, Id, Label, Layout, Margin, RichText, Sense,
    Stroke, StrokeKind, Vec2,
};

use crate::config::{
    self, check_paths, Config, ConflictPolicy, LanguageSetting, PathIssue, SyncMode, SyncTask,
    ThemeSetting,
};
use crate::engine::{
    Command, EngineConfig, EngineHandle, Event, FailureCode, PreviewData, SyncSummary,
};
use crate::i18n::{fill, policy_label, Key, Lang};
use crate::model::{ConflictRecord, HistoryEntry, TaskResult};
use crate::platform;
use crate::store;
use crate::sync::Resolution;
use crate::theme::{self, Palette};
use crate::tray::{TrayCommand, TrayState};
use crate::util;

const SIDEBAR_WIDTH: f32 = 212.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Tasks,
    History,
    Conflicts,
    Settings,
    About,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum TaskStatus {
    #[default]
    Idle,
    Queued,
    Scanning,
    Syncing,
    Error,
}

#[derive(Clone, Debug, Default)]
struct TaskRuntime {
    status: TaskStatus,
    scanned: u64,
    done: usize,
    total: usize,
    bytes: u64,
    current: String,
    last_error: Option<String>,
}

impl TaskRuntime {
    fn is_busy(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Queued | TaskStatus::Scanning | TaskStatus::Syncing
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    created: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickTarget {
    Source,
    Target,
}

struct TaskEditor {
    id: Option<String>,
    name: String,
    source: String,
    target: String,
    mode: SyncMode,
    enabled: bool,
    watch: bool,
    interval_secs: u64,
    excludes: String,
    inherit_policy: bool,
    policy: ConflictPolicy,
    advanced: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorAction {
    Cancel,
    Save,
    Browse(PickTarget),
}

struct PreviewState {
    task_id: String,
    task_name: String,
    data: PreviewData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileDialogKind {
    Export,
    Import,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TaskAction {
    SyncNow,
    Cancel,
    Preview,
    Edit,
    Delete,
    ToggleEnabled(bool),
    OpenSource,
    OpenTarget,
    CopyPath(String),
}

pub struct SyncoraApp {
    config: Config,
    lang: Lang,
    palette: Palette,
    dark: bool,
    system_dark: bool,
    page: Page,
    engine: EngineHandle,
    runtime: HashMap<String, TaskRuntime>,
    history: Vec<HistoryEntry>,
    conflicts: Vec<ConflictRecord>,
    resolving: HashSet<String>,
    editor: Option<TaskEditor>,
    preview: Option<PreviewState>,
    pending_delete: Option<String>,
    reset_confirm: bool,
    clear_history_confirm: bool,
    pending_import: Option<Config>,
    help_open: bool,
    toasts: Vec<Toast>,
    tray: Option<TrayState>,
    window_visible: bool,
    exit_requested: bool,
    applied_dark: Option<bool>,
    last_theme_check: Instant,
    dirty: bool,
    dirty_at: Instant,
    save_error_shown: bool,
    picker_rx: Option<Receiver<(PickTarget, Option<PathBuf>)>>,
    file_dialog_rx: Option<Receiver<(FileDialogKind, Option<PathBuf>)>>,
    manual_runs: HashSet<String>,
    history_task_filter: Option<String>,
    history_result_filter: Option<TaskResult>,
    history_search: String,
    data_dir: PathBuf,
    start_minimized: bool,
    last_tray_update: Instant,
    preview_pending: HashSet<String>,
}

impl SyncoraApp {
    pub fn new(cc: &eframe::CreationContext<'_>, start_minimized: bool) -> SyncoraApp {
        platform::prepare_data_dir();
        platform::register_app_id();

        let mut config = Config::load();
        config.normalize();
        platform::ensure_autostart(config.settings.start_with_windows);
        // If the registry entry could not be written (locked-down machine), keep
        // the setting honest instead of showing a switch that does nothing.
        if config.settings.start_with_windows && !platform::is_autostart_enabled() {
            config.settings.start_with_windows = false;
            let _ = config.save();
        }

        let lang = Lang::detect(config.settings.language);
        let system_dark = platform::is_system_dark();
        let dark = match config.settings.theme {
            ThemeSetting::System => system_dark,
            ThemeSetting::Light => false,
            ThemeSetting::Dark => true,
        };
        theme::apply(&cc.egui_ctx, dark);

        let engine = EngineHandle::spawn(cc.egui_ctx.clone());
        let mut history = store::load_history();
        history.truncate(config.settings.max_history);
        let conflicts = store::load_conflicts();
        let data_dir = config::data_dir();

        let mut app = SyncoraApp {
            palette: theme::palette(dark),
            config,
            lang,
            dark,
            system_dark,
            page: Page::Tasks,
            engine,
            runtime: HashMap::new(),
            history,
            conflicts,
            resolving: HashSet::new(),
            editor: None,
            preview: None,
            pending_delete: None,
            reset_confirm: false,
            clear_history_confirm: false,
            pending_import: None,
            help_open: false,
            toasts: Vec::new(),
            tray: None,
            window_visible: !start_minimized,
            exit_requested: false,
            applied_dark: Some(dark),
            last_theme_check: Instant::now(),
            dirty: false,
            dirty_at: Instant::now(),
            save_error_shown: false,
            picker_rx: None,
            file_dialog_rx: None,
            manual_runs: HashSet::new(),
            history_task_filter: None,
            history_result_filter: None,
            history_search: String::new(),
            data_dir,
            start_minimized,
            last_tray_update: Instant::now(),
            preview_pending: HashSet::new(),
        };

        let tooltip = app.tray_tooltip();
        app.tray = TrayState::new(
            app.lang,
            !app.config.settings.global_enabled,
            &tooltip,
        );
        if app.tray.is_none() && start_minimized {
            // Never start invisible without a tray escape hatch.
            app.window_visible = true;
            app.start_minimized = false;
            cc.egui_ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        if app.tray.is_none() {
            util::log("tray icon unavailable; continuing with a normal window");
        }

        app.push_engine_config();
        app
    }

    // -----------------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------------

    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.dirty_at = Instant::now();
    }

    fn save_now(&mut self) {
        if let Err(error) = self.config.save() {
            util::log(&format!("config save failed: {error}"));
            if !self.save_error_shown {
                self.save_error_shown = true;
                self.toast(
                    fill(Key::ToastExportFailed.tr(self.lang), &[("{error}", &error)]),
                    ToastKind::Error,
                );
            }
        } else {
            self.dirty = false;
            self.save_error_shown = false;
        }
    }

    fn save_if_due(&mut self) {
        if self.dirty && self.dirty_at.elapsed() >= Duration::from_millis(400) {
            self.save_now();
        }
    }

    fn push_engine_config(&mut self) {
        self.engine.send(Command::SetConfig(EngineConfig {
            tasks: self.config.tasks.clone(),
            global_enabled: self.config.settings.global_enabled,
            default_policy: self.config.settings.conflict_policy,
            verify: self.config.settings.verify_copies,
            skip_hidden_system: self.config.settings.skip_hidden_system,
        }));
    }

    fn save_history(&mut self) {
        if let Err(error) = store::save_history(&self.history) {
            util::log(&format!("history save failed: {error}"));
        }
    }

    fn save_conflicts(&mut self) {
        if let Err(error) = store::save_conflicts(&self.conflicts) {
            util::log(&format!("conflict save failed: {error}"));
        }
    }

    // -----------------------------------------------------------------------
    // Engine events
    // -----------------------------------------------------------------------

    fn poll_engine(&mut self) {
        for _ in 0..300 {
            let Some(event) = self.engine.try_recv() else {
                break;
            };
            self.handle_engine_event(event);
        }
    }

    fn handle_engine_event(&mut self, event: Event) {
        match event {
            Event::ScanProgress { task_id, files } => {
                let runtime = self.runtime.entry(task_id).or_default();
                if runtime.status != TaskStatus::Syncing {
                    runtime.status = TaskStatus::Scanning;
                }
                runtime.scanned = files;
            }
            Event::Started { task_id, total } => {
                let runtime = self.runtime.entry(task_id).or_default();
                runtime.status = TaskStatus::Syncing;
                runtime.total = total;
                runtime.done = 0;
                runtime.bytes = 0;
                runtime.current.clear();
            }
            Event::Progress {
                task_id,
                done,
                total,
                current,
                bytes,
            } => {
                let runtime = self.runtime.entry(task_id).or_default();
                runtime.status = TaskStatus::Syncing;
                runtime.done = done;
                runtime.total = total;
                runtime.current = current;
                runtime.bytes = bytes;
            }
            Event::Finished { task_id, summary } => {
                self.handle_finished(task_id, summary);
            }
            Event::Failed { task_id, failure } => {
                self.handle_failed(task_id, failure);
            }
            Event::PreviewReady { task_id, preview } => {
                self.preview_pending.remove(&task_id);
                let task_name = self
                    .config
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| t.name.clone())
                    .unwrap_or_default();
                self.preview = Some(PreviewState {
                    task_id,
                    task_name,
                    data: preview,
                });
            }
            Event::PreviewFailed { task_id, failure } => {
                self.preview_pending.remove(&task_id);
                self.toast(
                    fill(
                        Key::ToastSyncFailed.tr(self.lang),
                        &[("{task}", &self.task_name(&task_id))],
                    ),
                    ToastKind::Error,
                );
                self.toast(failure_text(self.lang, &failure), ToastKind::Error);
            }
            Event::ConflictResolved {
                task_id,
                conflict_id,
                result,
            } => {
                self.resolving.remove(&conflict_id);
                match result {
                    Ok(()) => {
                        self.conflicts.retain(|c| c.id != conflict_id);
                        self.save_conflicts();
                        self.toast(
                            Key::ConflictResolvedToast.tr(self.lang).to_string(),
                            ToastKind::Success,
                        );
                        let _ = task_id;
                    }
                    Err(error) => {
                        self.toast(
                            fill(
                                Key::ConflictResolveFailed.tr(self.lang),
                                &[("{error}", &error)],
                            ),
                            ToastKind::Error,
                        );
                    }
                }
            }
            Event::WatcherWarning { task_id, error } => {
                util::log(&format!("watcher warning for {task_id}: {error}"));
                if self.config.settings.notifications {
                    self.toast(
                        fill(
                            Key::ToastWatcherWarning.tr(self.lang),
                            &[("{task}", &self.task_name(&task_id))],
                        ),
                        ToastKind::Info,
                    );
                }
            }
            Event::SnapshotWarning { task_id, error } => {
                util::log(&format!("snapshot save failed for {task_id}: {error}"));
            }
        }
    }

    fn task_name(&self, task_id: &str) -> String {
        self.config
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| Key::Unknown.tr(self.lang).to_string())
    }

    fn handle_finished(&mut self, task_id: String, summary: SyncSummary) {
        let Some(index) = self.config.tasks.iter().position(|t| t.id == task_id) else {
            self.runtime.remove(&task_id);
            self.manual_runs.remove(&task_id);
            return;
        };
        let task_name = self.config.tasks[index].name.clone();
        let policy = self.config.tasks[index].effective_policy(self.config.settings.conflict_policy);

        let result = if summary.cancelled {
            TaskResult::Cancelled
        } else if !summary.errors.is_empty() {
            TaskResult::Partial
        } else if !summary.conflicts.is_empty() {
            TaskResult::Partial
        } else {
            TaskResult::Success
        };

        let message = summary
            .errors
            .first()
            .cloned()
            .or_else(|| summary.conflicts.first().map(|c| c.rel.clone()));

        {
            let task = &mut self.config.tasks[index];
            task.last_sync = Some(summary.finished_at);
            task.last_result = Some(result);
            task.last_message = message.clone();
        }

        let runtime = self.runtime.entry(task_id.clone()).or_default();
        runtime.status = TaskStatus::Idle;
        runtime.done = 0;
        runtime.total = 0;
        runtime.current.clear();
        runtime.last_error = if result == TaskResult::Success {
            None
        } else {
            message.clone()
        };

        let entry = HistoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.clone(),
            task_name: task_name.clone(),
            mode: summary.mode,
            started_at: summary.started_at,
            finished_at: summary.finished_at,
            copied: summary.copied,
            updated: summary.updated,
            deleted: summary.deleted,
            conflicts: summary.conflicts.len() as u64,
            bytes: summary.bytes,
            errors: summary.errors.len() as u64,
            result,
            message: message.clone(),
        };
        self.history.insert(0, entry);
        self.history.truncate(self.config.settings.max_history);
        self.save_history();
        self.mark_dirty();

        // Persist new unresolved conflicts.
        let mut added_conflict = false;
        for info in &summary.conflicts {
            let record = ConflictRecord {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: task_id.clone(),
                task_name: task_name.clone(),
                rel_path: info.rel.clone(),
                source_size: info.source.map(|m| m.size),
                // Stored in seconds: the UI formats it as a wall-clock time.
                source_mtime: info.source.map(|m| m.mtime_ns / 1_000_000_000),
                target_size: info.target.map(|m| m.size),
                target_mtime: info.target.map(|m| m.mtime_ns / 1_000_000_000),
                detected_at: util::now_secs(),
            };
            let key = record.dedup_key();
            if !self.conflicts.iter().any(|c| c.dedup_key() == key) {
                self.conflicts.insert(0, record);
                added_conflict = true;
            }
        }
        if added_conflict {
            self.save_conflicts();
        }

        if let Some(runtime) = self.runtime.get_mut(&task_id) {
            runtime.last_error = if summary.errors.is_empty() {
                None
            } else {
                Some(summary.errors.join("\n"))
            };
        }

        let manual = self.manual_runs.remove(&task_id);
        let acted = summary.copied + summary.updated + summary.deleted;
        if manual && result == TaskResult::Success {
            let key = if acted == 0 && summary.conflicts.is_empty() {
                Key::ToastNothingToSync
            } else {
                Key::ToastSyncDone
            };
            self.toast(
                fill(key.tr(self.lang), &[("{task}", &task_name)]),
                ToastKind::Success,
            );
        } else if manual && !summary.conflicts.is_empty() && summary.errors.is_empty() {
            // A conflict is not a failure: the sync did its job and stopped at
            // the file that needs a decision, so say that instead.
            self.toast(
                fill(
                    Key::ToastConflicts.tr(self.lang),
                    &[("{task}", &task_name)],
                ),
                ToastKind::Warn,
            );
        } else if manual && result.is_problem() {
            self.toast(
                fill(
                    Key::ToastSyncFailed.tr(self.lang),
                    &[("{task}", &task_name)],
                ),
                ToastKind::Error,
            );
        }

        if self.config.settings.notifications {
            if !summary.conflicts.is_empty() && policy == ConflictPolicy::Ask {
                let path = &summary.conflicts[0].rel;
                platform::notify(
                    &fill(
                        Key::NotifConflictTitle.tr(self.lang),
                        &[("{task}", &task_name)],
                    ),
                    &fill(
                        Key::NotifConflictBody.tr(self.lang),
                        &[("{path}", path)],
                    ),
                );
            } else if !summary.errors.is_empty() {
                platform::notify(
                    &fill(
                        Key::NotifErrorTitle.tr(self.lang),
                        &[("{task}", &task_name)],
                    ),
                    &summary.errors[0],
                );
            } else if self.config.settings.notify_on_success {
                platform::notify(
                    &fill(
                        Key::NotifSuccessTitle.tr(self.lang),
                        &[("{task}", &task_name)],
                    ),
                    &fill(
                        Key::NotifSuccessBody.tr(self.lang),
                        &[
                            ("{copied}", &summary.copied.to_string()),
                            ("{deleted}", &summary.deleted.to_string()),
                            ("{conflicts}", &summary.conflicts.len().to_string()),
                        ],
                    ),
                );
            }
        }

        self.update_tray_text();
    }

    fn handle_failed(&mut self, task_id: String, failure: crate::engine::JobFailure) {
        let message = failure_text(self.lang, &failure);
        let task_name = self.task_name(&task_id);

        if let Some(task) = self.config.tasks.iter_mut().find(|t| t.id == task_id) {
            task.last_sync = Some(util::now_secs());
            task.last_result = Some(TaskResult::Failed);
            task.last_message = Some(message.clone());
        }
        let runtime = self.runtime.entry(task_id.clone()).or_default();
        runtime.status = TaskStatus::Error;
        runtime.last_error = Some(message.clone());

        self.history.insert(
            0,
            HistoryEntry {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: task_id.clone(),
                task_name: task_name.clone(),
                mode: self
                    .config
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| t.mode)
                    .unwrap_or(SyncMode::OneWay),
                started_at: util::now_secs(),
                finished_at: util::now_secs(),
                copied: 0,
                updated: 0,
                deleted: 0,
                conflicts: 0,
                bytes: 0,
                errors: 1,
                result: TaskResult::Failed,
                message: Some(message.clone()),
            },
        );
        self.history.truncate(self.config.settings.max_history);
        self.save_history();
        self.mark_dirty();

        if self.config.settings.notifications {
            platform::notify(
                &fill(
                    Key::NotifErrorTitle.tr(self.lang),
                    &[("{task}", &task_name)],
                ),
                &message,
            );
        }
        self.toast(
            fill(
                Key::ToastSyncFailed.tr(self.lang),
                &[("{task}", &task_name)],
            ),
            ToastKind::Error,
        );
        self.manual_runs.remove(&task_id);
        self.update_tray_text();
    }

    // -----------------------------------------------------------------------
    // Tray & window
    // -----------------------------------------------------------------------

    fn tray_tooltip(&self) -> String {
        if !self.config.settings.global_enabled {
            return Key::TrayTooltipPaused.tr(self.lang).to_string();
        }
        let active = self.config.tasks.iter().filter(|t| t.enabled).count();
        fill(
            Key::TrayTooltip.tr(self.lang),
            &[("{n}", &active.to_string())],
        )
    }

    fn update_tray_text(&mut self) {
        if let Some(tray) = &self.tray {
            tray.set_language(self.lang, !self.config.settings.global_enabled);
            tray.set_paused(self.lang, !self.config.settings.global_enabled);
            tray.set_tooltip(&self.tray_tooltip());
        }
        self.last_tray_update = Instant::now();
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let commands = self
            .tray
            .as_ref()
            .map(|tray| tray.poll())
            .unwrap_or_default();
        for command in commands {
            match command {
                TrayCommand::Show => self.show_window(ctx),
                TrayCommand::SyncAll => {
                    if self.config.settings.global_enabled {
                        for task in &self.config.tasks {
                            if task.enabled {
                                self.manual_runs.insert(task.id.clone());
                            }
                        }
                        self.engine.send(Command::SyncAll);
                    }
                }
                TrayCommand::TogglePause => self.toggle_global(),
                TrayCommand::Settings => {
                    self.page = Page::Settings;
                    self.show_window(ctx);
                }
                TrayCommand::Quit => {
                    self.exit_requested = true;
                    self.save_now();
                    self.engine.shutdown();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.window_visible = true;
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.window_visible = false;
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.config.settings.close_to_tray && !self.exit_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide_window(ctx);
            } else {
                self.save_now();
            }
        }
    }

    fn toggle_global(&mut self) {
        self.config.settings.global_enabled = !self.config.settings.global_enabled;
        self.push_engine_config();
        self.mark_dirty();
        self.update_tray_text();
    }

    // -----------------------------------------------------------------------
    // Folder / file dialogs
    // -----------------------------------------------------------------------

    fn open_folder_picker(&mut self, target: PickTarget) {
        let title = Key::SelectFolder.tr(self.lang).to_string();
        let (tx, rx) = mpsc::channel();
        self.picker_rx = Some(rx);
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new().set_title(title).pick_folder();
            let _ = tx.send((target, path));
        });
    }

    fn poll_picker(&mut self) {
        let result = self
            .picker_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        if let Some((target, path)) = result {
            self.picker_rx = None;
            if let (Some(editor), Some(path)) = (self.editor.as_mut(), path) {
                let text = path.display().to_string();
                match target {
                    PickTarget::Source => editor.source = text,
                    PickTarget::Target => editor.target = text,
                }
                editor.error = None;
            }
        }
    }

    fn start_export(&mut self) {
        let name = format!(
            "Syncora-settings-{}.json",
            chrono::Local::now().format("%Y%m%d")
        );
        let (tx, rx) = mpsc::channel();
        self.file_dialog_rx = Some(rx);
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_title("Export Syncora settings")
                .add_filter("JSON", &["json"])
                .set_file_name(&name)
                .save_file();
            let _ = tx.send((FileDialogKind::Export, path));
        });
    }

    fn start_import(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.file_dialog_rx = Some(rx);
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_title("Import Syncora settings")
                .add_filter("JSON", &["json"])
                .pick_file();
            let _ = tx.send((FileDialogKind::Import, path));
        });
    }

    fn poll_file_dialog(&mut self) {
        let result = self
            .file_dialog_rx
            .as_ref()
            .and_then(|rx| rx.try_recv().ok());
        let Some((kind, path)) = result else {
            return;
        };
        self.file_dialog_rx = None;
        let Some(path) = path else {
            return;
        };
        match kind {
            FileDialogKind::Export => match serde_json::to_vec_pretty(&self.config) {
                Ok(data) => match std::fs::write(&path, data) {
                    Ok(()) => self.toast(
                        Key::ToastSettingsExported.tr(self.lang).to_string(),
                        ToastKind::Success,
                    ),
                    Err(error) => self.toast(
                        fill(
                            Key::ToastExportFailed.tr(self.lang),
                            &[("{error}", &error.to_string())],
                        ),
                        ToastKind::Error,
                    ),
                },
                Err(error) => self.toast(
                    fill(
                        Key::ToastExportFailed.tr(self.lang),
                        &[("{error}", &error.to_string())],
                    ),
                    ToastKind::Error,
                ),
            },
            FileDialogKind::Import => match std::fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<Config>(&text) {
                    Ok(mut config) => {
                        config.normalize();
                        self.pending_import = Some(config);
                    }
                    Err(error) => self.toast(
                        fill(
                            Key::ToastImportFailed.tr(self.lang),
                            &[("{error}", &error.to_string())],
                        ),
                        ToastKind::Error,
                    ),
                },
                Err(error) => self.toast(
                    fill(
                        Key::ToastImportFailed.tr(self.lang),
                        &[("{error}", &error.to_string())],
                    ),
                    ToastKind::Error,
                ),
            },
        }
    }

    fn apply_import(&mut self, config: Config) {
        self.config = config;
        self.config.normalize();
        self.lang = Lang::detect(self.config.settings.language);
        self.system_dark = platform::is_system_dark();
        self.dark = match self.config.settings.theme {
            ThemeSetting::System => self.system_dark,
            ThemeSetting::Light => false,
            ThemeSetting::Dark => true,
        };
        platform::ensure_autostart(self.config.settings.start_with_windows);
        self.conflicts.clear();
        self.save_conflicts();
        self.runtime.clear();
        self.push_engine_config();
        self.save_now();
        self.update_tray_text();
        self.toast(
            Key::ToastSettingsImported.tr(self.lang).to_string(),
            ToastKind::Success,
        );
    }

    // -----------------------------------------------------------------------
    // Theme & toasts
    // -----------------------------------------------------------------------

    fn check_system_theme(&mut self, ctx: &egui::Context) {
        if self.config.settings.theme != ThemeSetting::System {
            return;
        }
        if self.last_theme_check.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_theme_check = Instant::now();
        let system_dark = platform::is_system_dark();
        if system_dark != self.system_dark {
            self.system_dark = system_dark;
            self.dark = system_dark;
            self.palette = theme::palette(self.dark);
            theme::apply(ctx, self.dark);
            self.applied_dark = Some(self.dark);
        }
    }

    fn apply_theme_if_needed(&mut self, ctx: &egui::Context) {
        if self.applied_dark != Some(self.dark) {
            theme::apply(ctx, self.dark);
            self.palette = theme::palette(self.dark);
            self.applied_dark = Some(self.dark);
        }
    }

    fn toast(&mut self, text: String, kind: ToastKind) {
        self.toasts.push(Toast {
            text,
            kind,
            created: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        self.toasts
            .retain(|toast| toast.created.elapsed() < Duration::from_secs(4));
        if self.toasts.is_empty() {
            return;
        }
        let p = self.palette;
        egui::Area::new(Id::unique("toast_area"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-20.0, -20.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.with_layout(Layout::bottom_up(Align::RIGHT), |ui| {
                    for toast in self.toasts.iter().rev() {
                        Frame::new()
                            .fill(p.card)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(10)
                            .inner_margin(Margin::symmetric(12, 9))
                            .shadow(egui::Shadow {
                                offset: [0, 4],
                                blur: 14,
                                spread: 0,
                                color: Color32::from_black_alpha(if p.dark { 90 } else { 24 }),
                            })
                            .show(ui, |ui| {
                                ui.set_max_width(340.0);
                                ui.horizontal(|ui| {
                                    let color = match toast.kind {
                                        ToastKind::Info => p.accent,
                                        ToastKind::Success => p.success,
                                        ToastKind::Warn => p.warning,
                                        ToastKind::Error => p.danger,
                                    };
                                    let (rect, _) = ui
                                        .allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                    ui.painter().circle_filled(rect.center(), 4.0, color);
                                    ui.add(
                                        Label::new(
                                            RichText::new(&toast.text)
                                                .size(12.5)
                                                .color(p.text),
                                        )
                                        .wrap(),
                                    );
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });
        ctx.request_repaint_after(Duration::from_millis(400));
    }

    // -----------------------------------------------------------------------
    // Task actions
    // -----------------------------------------------------------------------

    fn apply_task_action(
        &mut self,
        ctx: &egui::Context,
        task_id: String,
        action: TaskAction,
    ) {
        match action {
            TaskAction::SyncNow => {
                if !self.config.settings.global_enabled {
                    self.toast(
                        Key::StatusPaused.tr(self.lang).to_string(),
                        ToastKind::Info,
                    );
                    return;
                }
                self.manual_runs.insert(task_id.clone());
                self.engine.send(Command::SyncNow(task_id.clone()));
                let runtime = self.runtime.entry(task_id).or_default();
                if runtime.status == TaskStatus::Idle || runtime.status == TaskStatus::Error {
                    runtime.status = TaskStatus::Queued;
                }
            }
            TaskAction::Cancel => {
                self.engine.send(Command::Cancel(task_id));
            }
            TaskAction::Preview => {
                self.preview_pending.insert(task_id.clone());
                self.engine.send(Command::Preview(task_id));
            }
            TaskAction::Edit => {
                if let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id) {
                    self.editor = Some(editor_from_task(task));
                }
            }
            TaskAction::Delete => {
                self.pending_delete = Some(task_id);
            }
            TaskAction::ToggleEnabled(value) => {
                if let Some(task) = self.config.tasks.iter_mut().find(|t| t.id == task_id) {
                    task.enabled = value;
                }
                self.push_engine_config();
                self.mark_dirty();
                self.update_tray_text();
            }
            TaskAction::CopyPath(path) => {
                ctx.copy_text(path);
                self.toast(
                    Key::PathCopied.tr(self.lang).to_string(),
                    ToastKind::Success,
                );
            }
            TaskAction::OpenSource | TaskAction::OpenTarget => {
                if let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id) {
                    let path = if action == TaskAction::OpenSource {
                        &task.source
                    } else {
                        &task.target
                    };
                    if let Err(error) = util::open_path(path) {
                        util::log(&format!("open path failed: {error}"));
                        self.toast(
                            Key::OpenFailed.tr(self.lang).to_string(),
                            ToastKind::Error,
                        );
                    }
                }
            }
        }
    }

    fn open_editor_new(&mut self) {
        self.editor = Some(TaskEditor {
            id: None,
            name: String::new(),
            source: String::new(),
            target: String::new(),
            mode: SyncMode::OneWay,
            enabled: true,
            watch: true,
            interval_secs: self.config.settings.default_interval_secs,
            excludes: String::new(),
            inherit_policy: true,
            policy: self.config.settings.conflict_policy,
            advanced: false,
            error: None,
        });
    }

    fn commit_editor(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        editor.error = None;

        let name = editor.name.trim().to_string();
        if name.is_empty() {
            editor.error = Some(Key::ValidationName.tr(self.lang).to_string());
            return;
        }
        let source_text = editor.source.trim().trim_matches('"').to_string();
        let target_text = editor.target.trim().trim_matches('"').to_string();
        if source_text.is_empty() {
            editor.error = Some(Key::ValidationSource.tr(self.lang).to_string());
            return;
        }
        if target_text.is_empty() {
            editor.error = Some(Key::ValidationTarget.tr(self.lang).to_string());
            return;
        }
        let source = config::absolute_path(&PathBuf::from(&source_text));
        let target = config::absolute_path(&PathBuf::from(&target_text));
        if let Err(issue) = check_paths(&source, &target) {
            editor.error = Some(path_issue_text(self.lang, issue));
            return;
        }

        let excludes: Vec<String> = editor
            .excludes
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        let policy = if editor.inherit_policy {
            None
        } else {
            Some(editor.policy)
        };

        let is_new = editor.id.is_none();
        if let Some(id) = &editor.id {
            if let Some(task) = self.config.tasks.iter_mut().find(|t| &t.id == id) {
                task.name = name;
                task.source = source;
                task.target = target;
                task.mode = editor.mode;
                task.watch = editor.watch;
                task.interval_secs = editor.interval_secs;
                task.excludes = excludes;
                task.conflict_policy = policy;
            }
        } else {
            let mut task = SyncTask::new(name, source, target, editor.mode);
            task.enabled = editor.enabled;
            task.watch = editor.watch;
            task.interval_secs = editor.interval_secs;
            task.excludes = excludes;
            task.conflict_policy = policy;
            self.config.tasks.push(task);
        }

        self.editor = None;
        self.push_engine_config();
        self.mark_dirty();
        self.update_tray_text();
        let toast = if is_new {
            Key::ToastTaskCreated
        } else {
            Key::ToastTaskSaved
        };
        self.toast(toast.tr(self.lang).to_string(), ToastKind::Success);
    }

    fn delete_task(&mut self, task_id: String) {
        self.config.tasks.retain(|t| t.id != task_id);
        self.conflicts.retain(|c| c.task_id != task_id);
        self.runtime.remove(&task_id);
        self.manual_runs.remove(&task_id);
        let _ = std::fs::remove_file(config::snapshot_path(&task_id));
        self.save_conflicts();
        self.push_engine_config();
        self.mark_dirty();
        self.update_tray_text();
        self.toast(
            Key::ToastTaskDeleted.tr(self.lang).to_string(),
            ToastKind::Info,
        );
    }

    fn resolve_conflict(&mut self, conflict: ConflictRecord, choice: Resolution) {
        if self.resolving.contains(&conflict.id) {
            return;
        }
        self.resolving.insert(conflict.id.clone());
        self.engine.send(Command::ResolveConflict {
            task_id: conflict.task_id.clone(),
            rel_path: conflict.rel_path.clone(),
            conflict_id: conflict.id.clone(),
            choice,
        });
    }

    fn reset_everything(&mut self) {
        let tasks = std::mem::take(&mut self.config.tasks);
        for task in &tasks {
            let _ = std::fs::remove_file(config::snapshot_path(&task.id));
        }
        self.config = Config::default();
        self.history.clear();
        self.conflicts.clear();
        self.runtime.clear();
        self.resolving.clear();
        self.manual_runs.clear();
        self.page = Page::Tasks;
        let _ = store::save_history(&[]);
        let _ = store::save_conflicts(&[]);
        let _ = platform::set_autostart(false);
        self.config.save().ok();
        self.lang = Lang::detect(self.config.settings.language);
        self.system_dark = platform::is_system_dark();
        self.dark = self.system_dark;
        self.push_engine_config();
        self.update_tray_text();
        self.toast(
            Key::ResetWord.tr(self.lang).to_string(),
            ToastKind::Info,
        );
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    fn draw(&mut self, root: &mut egui::Ui) {
        let p = self.palette;
        egui::Panel::left("syncora_sidebar")
            .exact_size(SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(12, 14)),
            )
            .show(root, |ui| self.draw_sidebar(ui));

        egui::Panel::bottom("syncora_status")
            .exact_size(32.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(22, 5)),
            )
            .show(root, |ui| self.draw_status_bar(ui));

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(22, 18)),
            )
            .show(root, |ui| {
                self.draw_header(ui);
                ui.add_space(14.0);
                egui::ScrollArea::vertical()
                    .id_salt(("page_scroll", self.page as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        match self.page {
                            Page::Tasks => self.draw_tasks_page(ui),
                            Page::History => self.draw_history_page(ui),
                            Page::Conflicts => self.draw_conflicts_page(ui),
                            Page::Settings => self.draw_settings_page(ui),
                            Page::About => self.draw_about_page(ui),
                        }
                        ui.add_space(20.0);
                    });
            });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal(|ui| {
            draw_logo(ui, &p, 34.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(config::APP_NAME)
                        .size(17.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(Key::AppTagline.tr(self.lang))
                        .size(9.5)
                        .color(p.faint),
                );
            });
        });
        ui.add_space(18.0);

        let mut new_page = None;
        if nav_item(
            ui,
            &p,
            NavIcon::Tasks,
            Key::NavTasks.tr(self.lang),
            self.page == Page::Tasks,
            None,
        ) {
            new_page = Some(Page::Tasks);
        }
        if nav_item(
            ui,
            &p,
            NavIcon::History,
            Key::NavHistory.tr(self.lang),
            self.page == Page::History,
            None,
        ) {
            new_page = Some(Page::History);
        }
        if nav_item(
            ui,
            &p,
            NavIcon::Conflicts,
            Key::NavConflicts.tr(self.lang),
            self.page == Page::Conflicts,
            if self.conflicts.is_empty() {
                None
            } else {
                Some(self.conflicts.len())
            },
        ) {
            new_page = Some(Page::Conflicts);
        }
        if nav_item(
            ui,
            &p,
            NavIcon::Settings,
            Key::NavSettings.tr(self.lang),
            self.page == Page::Settings,
            None,
        ) {
            new_page = Some(Page::Settings);
        }
        if nav_item(
            ui,
            &p,
            NavIcon::About,
            Key::NavAbout.tr(self.lang),
            self.page == Page::About,
            None,
        ) {
            new_page = Some(Page::About);
        }
        if let Some(page) = new_page {
            self.page = page;
        }

        ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
            if ghost_button(ui, &p, Key::HelpTitle.tr(self.lang)).clicked() {
                self.help_open = true;
            }
            ui.add_space(2.0);
            let theme_label = match self.config.settings.theme {
                ThemeSetting::System => Key::ThemeSystem.tr(self.lang),
                ThemeSetting::Light => Key::ThemeLight.tr(self.lang),
                ThemeSetting::Dark => Key::ThemeDark.tr(self.lang),
            };
            if ghost_button(ui, &p, &format!("◐  {theme_label}")).clicked() {
                self.config.settings.theme = match self.config.settings.theme {
                    ThemeSetting::System => ThemeSetting::Light,
                    ThemeSetting::Light => ThemeSetting::Dark,
                    ThemeSetting::Dark => ThemeSetting::System,
                };
                self.dark = match self.config.settings.theme {
                    ThemeSetting::System => self.system_dark,
                    ThemeSetting::Light => false,
                    ThemeSetting::Dark => true,
                };
                self.mark_dirty();
            }
            ui.add_space(2.0);
            ui.label(
                RichText::new(Key::BuiltWith.tr(self.lang))
                    .size(9.0)
                    .color(p.faint),
            );
        });
    }

    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let running = self
            .runtime
            .values()
            .filter(|r| r.status == TaskStatus::Syncing || r.status == TaskStatus::Scanning)
            .count();
        let (text, color) = if !self.config.settings.global_enabled {
            (Key::StatusPaused.tr(self.lang).to_string(), p.warning)
        } else if running > 0 {
            (
                fill(
                    Key::StatusRunning.tr(self.lang),
                    &[("{n}", &running.to_string())],
                ),
                p.accent,
            )
        } else if !self.conflicts.is_empty() {
            (
                fill(
                    Key::StatusIssues.tr(self.lang),
                    &[("{n}", &self.conflicts.len().to_string())],
                ),
                p.warning,
            )
        } else {
            (Key::StatusAllGood.tr(self.lang).to_string(), p.success)
        };

        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.5, color);
            ui.label(RichText::new(text).size(10.5).color(p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        Key::VersionLabel.tr(self.lang),
                        config::APP_VERSION
                    ))
                    .size(10.0)
                    .color(p.faint),
                );
            });
        });
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let (title, subtitle) = match self.page {
            Page::Tasks => (
                Key::TasksTitle.tr(self.lang),
                Key::TasksSubtitle.tr(self.lang),
            ),
            Page::History => (
                Key::HistoryTitle.tr(self.lang),
                Key::HistorySubtitle.tr(self.lang),
            ),
            Page::Conflicts => (
                Key::ConflictsTitle.tr(self.lang),
                Key::ConflictsSubtitle.tr(self.lang),
            ),
            Page::Settings => (
                Key::SettingsTitle.tr(self.lang),
                Key::SettingsSubtitle.tr(self.lang),
            ),
            Page::About => (
                Key::AboutTitle.tr(self.lang),
                Key::AboutDesc.tr(self.lang),
            ),
        };

        ui.horizontal(|ui| {
            // Reserve the right-hand side first so a long title can never be
            // drawn underneath the controls.
            let controls_width = match self.page {
                Page::Tasks | Page::History => 340.0,
                _ => 210.0,
            };
            let text_width = (ui.available_width() - controls_width).max(180.0);
            ui.allocate_ui_with_layout(
                Vec2::new(text_width, 52.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.add(
                        Label::new(RichText::new(title).size(23.0).strong().color(p.text))
                            .truncate(),
                    );
                    ui.add(
                        Label::new(RichText::new(subtitle).size(11.5).color(p.muted))
                            .truncate(),
                    );
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match self.page {
                    Page::Tasks => {
                        if primary_button(
                            ui,
                            &p,
                            &format!("+  {}", Key::NewSync.tr(self.lang)),
                        )
                        .on_hover_text(Key::TipAddTask.tr(self.lang))
                        .clicked()
                        {
                            self.open_editor_new();
                        }
                    }
                    Page::History => {
                        if !self.history.is_empty()
                            && secondary_button(ui, &p, Key::ClearHistory.tr(self.lang))
                                .clicked()
                        {
                            self.clear_history_confirm = true;
                        }
                    }
                    _ => {}
                }
                ui.add_space(12.0);
                self.draw_master_switch(ui);
            });
        });
    }

    fn draw_master_switch(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let enabled = self.config.settings.global_enabled;
        let mut value = enabled;
        let response = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(11, 7))
            .show(ui, |ui| {
                ui.set_width(154.0);
                // Explicit left-to-right order: this widget is also used inside
                // right-to-left layouts, and the inner row must not flip.
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        3.5,
                        if enabled { p.success } else { p.warning },
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new(Key::GlobalAllSyncs.tr(self.lang))
                                .size(11.5)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(if enabled {
                                Key::GlobalActive.tr(self.lang)
                            } else {
                                Key::GlobalPaused.tr(self.lang)
                            })
                            .size(9.5)
                            .color(p.faint),
                        );
                    });
                    ui.add_space(6.0);
                    toggle(ui, &p, &mut value, true);
                });
            })
            .response;

        response.on_hover_text(Key::TipGlobalSwitch.tr(self.lang));
        if value != enabled {
            self.toggle_global();
        }
    }

    // -----------------------------------------------------------------------
    // Tasks page
    // -----------------------------------------------------------------------

    fn draw_tasks_page(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.config.tasks.is_empty() {
            self.draw_empty_tasks(ui);
            return;
        }

        let tasks = self.config.tasks.clone();
        let mut actions: Vec<(String, TaskAction)> = Vec::new();
        for task in &tasks {
            let runtime = self.runtime.get(&task.id).cloned().unwrap_or_default();
            draw_task_card(
                ui,
                task,
                &runtime,
                self.lang,
                &p,
                self.config.settings.conflict_policy,
                self.preview_pending.contains(&task.id),
                &mut actions,
            );
            ui.add_space(12.0);
        }
        for (task_id, action) in actions {
            self.apply_task_action(ui.ctx(), task_id, action);
        }
    }

    fn draw_empty_tasks(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            draw_logo(ui, &p, 64.0);
            ui.add_space(18.0);
            ui.label(
                RichText::new(Key::EmptyTitle.tr(self.lang))
                    .size(22.0)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(Key::EmptyDesc.tr(self.lang))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(22.0);

            let width = ui.available_width().min(760.0);
            ui.allocate_ui_with_layout(
                Vec2::new(width, 0.0),
                Layout::left_to_right(Align::TOP),
                |ui| {
                    let step_width = (width - 24.0) / 3.0;
                    for (index, (title, desc)) in [
                        (Key::EmptyStep1Title, Key::EmptyStep1Desc),
                        (Key::EmptyStep2Title, Key::EmptyStep2Desc),
                        (Key::EmptyStep3Title, Key::EmptyStep3Desc),
                    ]
                    .iter()
                    .enumerate()
                    {
                        Frame::new()
                            .fill(p.card)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(12)
                            .inner_margin(Margin::same(14))
                            .show(ui, |ui| {
                                ui.set_width(step_width - 30.0);
                                ui.vertical(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(24.0),
                                        Sense::hover(),
                                    );
                                    ui.painter().circle_filled(
                                        rect.center(),
                                        12.0,
                                        p.accent_soft,
                                    );
                                    ui.painter().text(
                                        rect.center(),
                                        Align2::CENTER_CENTER,
                                        format!("{}", index + 1),
                                        FontId::proportional(12.0),
                                        p.accent,
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(title.tr(self.lang))
                                            .size(12.5)
                                            .strong()
                                            .color(p.text),
                                    );
                                    ui.label(
                                        RichText::new(desc.tr(self.lang))
                                            .size(10.5)
                                            .color(p.muted),
                                    );
                                });
                            });
                        ui.add_space(12.0);
                    }
                },
            );

            ui.add_space(26.0);
            if primary_button(
                ui,
                &p,
                &format!("+  {}", Key::AddFirstSync.tr(self.lang)),
            )
            .clicked()
            {
                self.open_editor_new();
            }
            ui.add_space(12.0);
            ui.label(
                RichText::new(Key::HelpTip.tr(self.lang))
                    .size(10.5)
                    .color(p.faint),
            );
        });
    }

    // -----------------------------------------------------------------------
    // History page
    // -----------------------------------------------------------------------

    fn draw_history_page(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.history.is_empty() {
            empty_state(
                ui,
                &p,
                self.lang,
                Key::HistoryEmpty,
                Key::HistoryEmptyDesc,
            );
            return;
        }

        // Filters
        let mut task_filter = self.history_task_filter.clone();
        let mut result_filter = self.history_result_filter;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("history_task_filter")
                .selected_text(match &task_filter {
                    None => Key::AllTasks.tr(self.lang).to_string(),
                    Some(id) => self
                        .config
                        .tasks
                        .iter()
                        .find(|t| &t.id == id)
                        .map(|t| t.name.clone())
                        .or_else(|| {
                            self.history
                                .iter()
                                .find(|h| &h.task_id == id)
                                .map(|h| h.task_name.clone())
                        })
                        .unwrap_or_else(|| Key::Unknown.tr(self.lang).to_string()),
                })
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut task_filter,
                        None,
                        Key::AllTasks.tr(self.lang),
                    );
                    let mut seen = HashSet::new();
                    for entry in &self.history {
                        if seen.insert(entry.task_id.clone()) {
                            ui.selectable_value(
                                &mut task_filter,
                                Some(entry.task_id.clone()),
                                &entry.task_name,
                            );
                        }
                    }
                });

            egui::ComboBox::from_id_salt("history_result_filter")
                .selected_text(match result_filter {
                    None => Key::AllResults.tr(self.lang).to_string(),
                    Some(result) => result_label(result, self.lang),
                })
                .width(160.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut result_filter,
                        None,
                        Key::AllResults.tr(self.lang),
                    );
                    for result in [
                        TaskResult::Success,
                        TaskResult::Partial,
                        TaskResult::Failed,
                        TaskResult::Cancelled,
                    ] {
                        ui.selectable_value(
                            &mut result_filter,
                            Some(result),
                            result_label(result, self.lang),
                        );
                    }
                });

            ui.add(
                egui::TextEdit::singleline(&mut self.history_search)
                    .hint_text(Key::SearchPlaceholder.tr(self.lang))
                    .desired_width(220.0),
            );
        });
        if task_filter != self.history_task_filter || result_filter != self.history_result_filter {
            self.history_task_filter = task_filter;
            self.history_result_filter = result_filter;
        }
        ui.add_space(12.0);

        let search = self.history_search.to_lowercase();
        let filtered: Vec<&HistoryEntry> = self
            .history
            .iter()
            .filter(|entry| {
                if let Some(task) = &self.history_task_filter {
                    if &entry.task_id != task {
                        return false;
                    }
                }
                if let Some(result) = self.history_result_filter {
                    if entry.result != result {
                        return false;
                    }
                }
                if !search.is_empty() {
                    let haystack = format!(
                        "{} {} {}",
                        entry.task_name,
                        entry.message.clone().unwrap_or_default(),
                        util::absolute_time(entry.finished_at)
                    )
                    .to_lowercase();
                    if !haystack.contains(&search) {
                        return false;
                    }
                }
                true
            })
            .take(500)
            .collect();

        if filtered.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(Key::NoMatches.tr(self.lang))
                        .size(13.0)
                        .color(p.muted),
                );
            });
            return;
        }

        for entry in filtered {
            draw_history_row(ui, entry, self.lang, &p);
            ui.add_space(8.0);
        }
        ui.add_space(6.0);
        ui.label(
            RichText::new(fill(
                Key::ShowingLatest.tr(self.lang),
                &[("{n}", &self.history.len().to_string())],
            ))
            .size(10.0)
            .color(p.faint),
        );
    }

    // -----------------------------------------------------------------------
    // Conflicts page
    // -----------------------------------------------------------------------

    fn draw_conflicts_page(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.conflicts.is_empty() {
            empty_state(
                ui,
                &p,
                self.lang,
                Key::ConflictsEmpty,
                Key::ConflictsEmptyDesc,
            );
            return;
        }

        ui.label(
            RichText::new(fill(
                Key::UnresolvedCount.tr(self.lang),
                &[("{n}", &self.conflicts.len().to_string())],
            ))
            .size(11.5)
            .color(p.warning),
        );
        ui.add_space(10.0);

        let conflicts = self.conflicts.clone();
        let mut resolutions: Vec<(ConflictRecord, Resolution)> = Vec::new();
        let mut open_source: Option<PathBuf> = None;

        for conflict in &conflicts {
            let task = self.config.tasks.iter().find(|t| t.id == conflict.task_id);
            let source_root = task.map(|t| t.source.clone()).unwrap_or_default();
            let resolving = self.resolving.contains(&conflict.id);

            Frame::new()
                .fill(p.card)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(12)
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, p.warning);
                        ui.label(
                            RichText::new(&conflict.task_name)
                                .size(11.0)
                                .color(p.muted),
                        );
                        ui.label(RichText::new("·").size(10.0).color(p.faint));
                        ui.label(
                            RichText::new(util::format_relative_time(
                                conflict.detected_at,
                                self.lang,
                            ))
                            .size(10.5)
                            .color(p.faint),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ghost_button(ui, &p, Key::OpenSource.tr(self.lang)).clicked() {
                                open_source = Some(source_root.clone());
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.add(
                        Label::new(
                            RichText::new(&conflict.rel_path)
                                .monospace()
                                .size(12.0)
                                .color(p.text),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&conflict.rel_path);
                    ui.add_space(8.0);

                    ui.columns(2, |columns| {
                        draw_version_box(
                            &mut columns[0],
                            &p,
                            self.lang,
                            Key::ConflictSourceVersion,
                            conflict.source_size,
                            conflict.source_mtime,
                            conflict.source_missing(),
                        );
                        draw_version_box(
                            &mut columns[1],
                            &p,
                            self.lang,
                            Key::ConflictTargetVersion,
                            conflict.target_size,
                            conflict.target_mtime,
                            conflict.target_missing(),
                        );
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if primary_button(ui, &p, Key::KeepSource.tr(self.lang)).clicked() {
                            resolutions.push((conflict.clone(), Resolution::KeepSource));
                        }
                        if secondary_button(ui, &p, Key::KeepTarget.tr(self.lang)).clicked() {
                            resolutions.push((conflict.clone(), Resolution::KeepTarget));
                        }
                        if secondary_button(ui, &p, Key::KeepBoth.tr(self.lang))
                            .on_hover_text(Key::ConflictKeepBothHelp.tr(self.lang))
                            .clicked()
                        {
                            resolutions.push((conflict.clone(), Resolution::KeepBoth));
                        }
                        if resolving {
                            ui.spinner();
                            ui.label(
                                RichText::new(Key::Resolve.tr(self.lang))
                                    .size(10.0)
                                    .color(p.faint),
                            );
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(Key::ConflictResolveHelp.tr(self.lang))
                                    .size(9.5)
                                    .color(p.faint),
                            );
                        });
                    });
                });
            ui.add_space(10.0);
        }

        for (conflict, choice) in resolutions {
            self.resolve_conflict(conflict, choice);
        }
        if let Some(path) = open_source {
            let _ = util::open_path(&path);
        }
    }

    // -----------------------------------------------------------------------
    // Settings page
    // -----------------------------------------------------------------------

    fn draw_settings_page(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let lang = self.lang;

        // ---- General ------------------------------------------------------
        settings_section(ui, &p, Key::SectionGeneral.tr(lang), |ui| {
            setting_row(
                ui,
                &p,
                Key::Language.tr(lang),
                Some(Key::LanguageHelp.tr(lang)),
                |ui| {
                    let mut value = self.config.settings.language;
                    egui::ComboBox::from_id_salt("language_combo")
                        .selected_text(match value {
                            LanguageSetting::Auto => Key::LanguageAuto.tr(lang),
                            LanguageSetting::English => Key::LanguageEnglish.tr(lang),
                            LanguageSetting::Chinese => Key::LanguageChinese.tr(lang),
                        })
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut value,
                                LanguageSetting::Auto,
                                Key::LanguageAuto.tr(lang),
                            );
                            ui.selectable_value(
                                &mut value,
                                LanguageSetting::English,
                                Key::LanguageEnglish.tr(lang),
                            );
                            ui.selectable_value(
                                &mut value,
                                LanguageSetting::Chinese,
                                Key::LanguageChinese.tr(lang),
                            );
                        });
                    if value != self.config.settings.language {
                        self.config.settings.language = value;
                        self.lang = Lang::detect(value);
                        self.mark_dirty();
                        self.update_tray_text();
                    }
                },
            );

            setting_row(
                ui,
                &p,
                Key::Appearance.tr(lang),
                Some(Key::ThemeHelp.tr(lang)),
                |ui| {
                    ui.horizontal(|ui| {
                        for (setting, key) in [
                            (ThemeSetting::System, Key::ThemeSystem),
                            (ThemeSetting::Light, Key::ThemeLight),
                            (ThemeSetting::Dark, Key::ThemeDark),
                        ] {
                            let selected = self.config.settings.theme == setting;
                            let button = Button::new(
                                RichText::new(key.tr(lang))
                                    .size(11.5)
                                    .color(if selected {
                                        Color32::WHITE
                                    } else {
                                        p.muted
                                    }),
                            )
                            .fill(if selected { p.accent } else { p.card })
                            .stroke(Stroke::new(1.0, if selected { p.accent } else { p.border }))
                            .corner_radius(7)
                            .min_size(Vec2::new(74.0, 28.0));
                            if ui.add(button).clicked() {
                                self.config.settings.theme = setting;
                                self.dark = match setting {
                                    ThemeSetting::System => self.system_dark,
                                    ThemeSetting::Light => false,
                                    ThemeSetting::Dark => true,
                                };
                                self.palette = theme::palette(self.dark);
                                self.mark_dirty();
                            }
                        }
                    });
                },
            );
        });

        // ---- Startup ------------------------------------------------------
        settings_section(ui, &p, Key::SectionStartup.tr(lang), |ui| {
            let mut start_with_windows = self.config.settings.start_with_windows;
            setting_row(
                ui,
                &p,
                Key::StartWithWindows.tr(lang),
                Some(Key::StartWithWindowsHelp.tr(lang)),
                |ui| {
                    if toggle(ui, &p, &mut start_with_windows, true).changed() {
                        match platform::set_autostart(start_with_windows) {
                            Ok(()) => {
                                self.config.settings.start_with_windows = start_with_windows;
                                self.mark_dirty();
                            }
                            Err(error) => {
                                start_with_windows = !start_with_windows;
                                self.toast(
                                    fill(
                                        Key::ToastAutostartFailed.tr(lang),
                                        &[("{error}", &error)],
                                    ),
                                    ToastKind::Error,
                                );
                            }
                        }
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::StartMinimized.tr(lang),
                Some(Key::StartMinimizedHelp.tr(lang)),
                |ui| {
                    if toggle(
                        ui,
                        &p,
                        &mut self.config.settings.start_minimized,
                        true,
                    )
                    .changed()
                    {
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::CloseToTray.tr(lang),
                Some(Key::CloseToTrayHelp.tr(lang)),
                |ui| {
                    if toggle(ui, &p, &mut self.config.settings.close_to_tray, true).changed() {
                        self.mark_dirty();
                    }
                },
            );
        });

        // ---- Sync behavior ------------------------------------------------
        settings_section(ui, &p, Key::SectionSync.tr(lang), |ui| {
            setting_row(
                ui,
                &p,
                Key::DefaultInterval.tr(lang),
                Some(Key::IntervalHelpShort.tr(lang)),
                |ui| {
                    let mut interval = self.config.settings.default_interval_secs;
                    interval_combo(ui, &p, lang, "default_interval", &mut interval);
                    if interval != self.config.settings.default_interval_secs {
                        self.config.settings.default_interval_secs = interval;
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::DefaultConflict.tr(lang),
                Some(Key::ConflictHelp.tr(lang)),
                |ui| {
                    let mut policy = self.config.settings.conflict_policy;
                    egui::ComboBox::from_id_salt("default_policy")
                        .selected_text(policy_label(policy, lang))
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            for option in [
                                ConflictPolicy::Ask,
                                ConflictPolicy::KeepBoth,
                                ConflictPolicy::PreferSource,
                                ConflictPolicy::PreferTarget,
                                ConflictPolicy::Skip,
                            ] {
                                ui.selectable_value(
                                    &mut policy,
                                    option,
                                    policy_label(option, lang),
                                );
                            }
                        });
                    if policy != self.config.settings.conflict_policy {
                        self.config.settings.conflict_policy = policy;
                        self.push_engine_config();
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::Notifications.tr(lang),
                Some(Key::NotificationsHelp.tr(lang)),
                |ui| {
                    if toggle(ui, &p, &mut self.config.settings.notifications, true).changed() {
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::NotifySuccess.tr(lang),
                None,
                |ui| {
                    ui.add_enabled_ui(self.config.settings.notifications, |ui| {
                        if toggle(ui, &p, &mut self.config.settings.notify_on_success, true)
                            .changed()
                        {
                            self.mark_dirty();
                        }
                    });
                },
            );
            setting_row(
                ui,
                &p,
                Key::VerifyLabel.tr(lang),
                Some(Key::VerifyHelp.tr(lang)),
                |ui| {
                    if toggle(ui, &p, &mut self.config.settings.verify_copies, true).changed() {
                        self.push_engine_config();
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::SkipHidden.tr(lang),
                Some(Key::SkipHiddenHelp.tr(lang)),
                |ui| {
                    if toggle(ui, &p, &mut self.config.settings.skip_hidden_system, true).changed()
                    {
                        self.push_engine_config();
                        self.mark_dirty();
                    }
                },
            );
            setting_row(
                ui,
                &p,
                Key::MaxHistory.tr(lang),
                Some(Key::MaxHistoryHelp.tr(lang)),
                |ui| {
                    let mut value = self.config.settings.max_history;
                    let response = ui.add(
                        egui::DragValue::new(&mut value)
                            .range(100..=5000)
                            .speed(10.0),
                    );
                    if response.changed() {
                        self.config.settings.max_history = value;
                        self.history.truncate(value);
                        self.mark_dirty();
                    }
                },
            );
        });

        // ---- Data ---------------------------------------------------------
        settings_section(ui, &p, Key::SectionData.tr(lang), |ui| {
            ui.horizontal(|ui| {
                if secondary_button(ui, &p, Key::ExportSettings.tr(lang))
                    .on_hover_text(Key::ExportHelp.tr(lang))
                    .clicked()
                {
                    self.start_export();
                }
                if secondary_button(ui, &p, Key::ImportSettings.tr(lang))
                    .on_hover_text(Key::ImportHelp.tr(lang))
                    .clicked()
                {
                    self.start_import();
                }
                if ghost_button(ui, &p, Key::OpenDataFolder.tr(lang)).clicked() {
                    let _ = util::open_path(&self.data_dir);
                }
                let has_log = platform::log_path_exists();
                if ui
                    .add_enabled_ui(has_log, |ui| {
                        ghost_button(ui, &p, Key::OpenLog.tr(lang))
                    })
                    .inner
                    .on_hover_text(Key::OpenLogHelp.tr(lang))
                    .clicked()
                {
                    let _ = util::reveal_path(&config::log_path());
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(Key::ExportHelp.tr(lang))
                    .size(10.0)
                    .color(p.faint),
            );
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(8.0);
            if danger_button(ui, &p, Key::ResetSettings.tr(lang)).clicked() {
                self.reset_confirm = true;
            }
        });
    }

    // -----------------------------------------------------------------------
    // About page
    // -----------------------------------------------------------------------

    fn draw_about_page(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.add_space(10.0);
        Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(16)
            .inner_margin(Margin::same(26))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    draw_logo(ui, &p, 76.0);
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(config::APP_NAME)
                            .size(28.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(Key::AppTagline.tr(self.lang))
                            .size(13.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            Key::VersionLabel.tr(self.lang),
                            config::APP_VERSION
                        ))
                        .size(11.0)
                        .color(p.faint),
                    );
                    ui.add_space(20.0);

                    for key in [
                        Key::AboutFeature1,
                        Key::AboutFeature2,
                        Key::AboutFeature3,
                        Key::AboutFeature4,
                        Key::AboutFeature5,
                    ] {
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                            ui.painter().circle_filled(rect.center(), 8.0, p.accent_soft);
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                "✓",
                                FontId::proportional(10.0),
                                p.accent,
                            );
                            ui.label(
                                RichText::new(key.tr(self.lang))
                                    .size(12.0)
                                    .color(p.text),
                            );
                        });
                        ui.add_space(2.0);
                    }

                    ui.add_space(18.0);
                    ui.label(
                        RichText::new(Key::LicenseNote.tr(self.lang))
                            .size(11.0)
                            .color(p.faint),
                    );
                    ui.add_space(6.0);
                    // Project link, kept next to the version so it is the first
                    // thing a curious user can click.
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(Key::AboutSource.tr(self.lang))
                                .size(11.5)
                                .color(p.faint),
                        );
                        ui.hyperlink_to(
                            RichText::new(config::APP_REPO_LABEL)
                                .size(11.5)
                                .color(p.accent),
                            config::APP_REPO,
                        );
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                        ui.label(RichText::new("·").color(p.faint));
                        ui.hyperlink_to("Rust", "https://www.rust-lang.org");
                        ui.label(RichText::new("·").color(p.faint));
                        if ui
                            .link(RichText::new(Key::HelpTitle.tr(self.lang)).size(12.0))
                            .clicked()
                        {
                            self.help_open = true;
                        }
                    });
                });
            });
    }

    // -----------------------------------------------------------------------
    // Modals
    // -----------------------------------------------------------------------

    fn draw_modals(&mut self, ctx: &egui::Context) {
        self.draw_editor_modal(ctx);
        self.draw_preview_modal(ctx);
        self.draw_confirm_modals(ctx);
        self.draw_help_modal(ctx);
    }

    fn draw_editor_modal(&mut self, ctx: &egui::Context) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let p = self.palette;
        let lang = self.lang;
        let global_policy = self.config.settings.conflict_policy;
        let mut action: Option<EditorAction> = None;

        let frame = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(16)
            .inner_margin(Margin::same(22));
        let response = egui::Modal::new(Id::unique("task_editor"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(570.0);
                draw_editor_contents(ui, editor, &p, lang, global_policy, &mut action);
            });

        if response.should_close() {
            action = Some(EditorAction::Cancel);
        }

        match action {
            Some(EditorAction::Cancel) => self.editor = None,
            Some(EditorAction::Save) => self.commit_editor(),
            Some(EditorAction::Browse(target)) => self.open_folder_picker(target),
            None => {}
        }
    }

    fn draw_preview_modal(&mut self, ctx: &egui::Context) {
        let Some(preview) = &self.preview else {
            return;
        };
        let p = self.palette;
        let lang = self.lang;
        let mut close = false;
        let mut run = false;
        let task_name = preview.task_name.clone();
        let preview_task_id = preview.task_id.clone();

        let frame = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(16)
            .inner_margin(Margin::same(22));
        let response = egui::Modal::new(Id::unique("preview_modal"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(560.0);
                ui.label(
                    RichText::new(Key::PreviewTitle.tr(lang))
                        .size(19.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(Key::PreviewDesc.tr(lang))
                        .size(11.0)
                        .color(p.muted),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(&task_name)
                        .size(11.0)
                        .color(p.accent),
                );
                ui.add_space(14.0);

                let data = &preview.data;
                ui.columns(2, |columns| {
                    preview_stat(
                        &mut columns[0],
                        &p,
                        Key::PreviewCopy.tr(lang),
                        data.new_files,
                        p.success,
                    );
                    preview_stat(
                        &mut columns[0],
                        &p,
                        Key::PreviewUpdate.tr(lang),
                        data.updated_files,
                        p.accent,
                    );
                    preview_stat(
                        &mut columns[1],
                        &p,
                        Key::PreviewDelete.tr(lang),
                        data.deleted_files,
                        p.danger,
                    );
                    preview_stat(
                        &mut columns[1],
                        &p,
                        Key::PreviewConflicts.tr(lang),
                        data.conflicts,
                        p.warning,
                    );
                });

                ui.add_space(10.0);
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {}",
                        Key::Transferred.tr(lang),
                        util::format_bytes(data.bytes)
                    ))
                    .size(10.5)
                    .color(p.faint),
                );
                ui.label(
                    RichText::new(fill(
                        Key::PreviewScanned.tr(lang),
                        &[
                            ("{src}", &data.source_files.to_string()),
                            ("{dst}", &data.target_files.to_string()),
                            ("{src_size}", &util::format_bytes(data.source_bytes)),
                            ("{dst_size}", &util::format_bytes(data.target_bytes)),
                        ],
                    ))
                    .size(10.0)
                    .color(p.faint),
                );

                if data.total_actions == 0 && data.conflicts == 0 {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(Key::PreviewNone.tr(lang))
                            .size(12.5)
                            .color(p.success),
                    );
                } else if !data.sample.is_empty() {
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new(Key::PreviewSample.tr(lang))
                            .size(10.5)
                            .color(p.faint),
                    );
                    Frame::new()
                        .fill(p.input)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(8)
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(180.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for path in &data.sample {
                                        ui.label(
                                            RichText::new(path)
                                                .monospace()
                                                .size(10.5)
                                                .color(p.muted),
                                        );
                                    }
                                    if data.total_actions > data.sample.len() {
                                        ui.label(
                                            RichText::new(fill(
                                                Key::PreviewMore.tr(lang),
                                                &[(
                                                    "{n}",
                                                    &(data.total_actions
                                                        - data.sample.len())
                                                    .to_string(),
                                                )],
                                            ))
                                            .size(10.0)
                                            .color(p.faint),
                                        );
                                    }
                                });
                        });
                }

                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if primary_button(ui, &p, Key::PreviewRun.tr(lang)).clicked() {
                        run = true;
                    }
                    if secondary_button(ui, &p, Key::Close.tr(lang)).clicked() {
                        close = true;
                    }
                });
            });

        if response.should_close() {
            close = true;
        }
        if run {
            self.preview = None;
            if self.config.tasks.iter().any(|t| t.id == preview_task_id) {
                self.apply_task_action(ctx, preview_task_id, TaskAction::SyncNow);
            }
        } else if close {
            self.preview = None;
        }
    }

    fn draw_confirm_modals(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let lang = self.lang;

        // Delete task
        if let Some(task_id) = self.pending_delete.clone() {
            let name = self.task_name(&task_id);
            let mut close = false;
            let mut confirm = false;
            let response = confirm_modal(
                ctx,
                "delete_task",
                &p,
                Key::DeleteTaskTitle.tr(lang),
                &format!("{}  {}", name, Key::DeleteTaskDesc.tr(lang)),
                Key::Cancel.tr(lang),
                Key::DeleteWord.tr(lang),
                true,
                &mut close,
                &mut confirm,
            );
            if response.should_close() {
                close = true;
            }
            if confirm {
                self.delete_task(task_id);
                self.pending_delete = None;
            } else if close {
                self.pending_delete = None;
            }
        }

        // Clear history
        if self.clear_history_confirm {
            let mut close = false;
            let mut confirm = false;
            let response = confirm_modal(
                ctx,
                "clear_history",
                &p,
                Key::ClearHistoryTitle.tr(lang),
                Key::ClearHistoryDesc.tr(lang),
                Key::Cancel.tr(lang),
                Key::ClearHistory.tr(lang),
                true,
                &mut close,
                &mut confirm,
            );
            if response.should_close() {
                close = true;
            }
            if confirm {
                self.history.clear();
                self.save_history();
                self.clear_history_confirm = false;
            } else if close {
                self.clear_history_confirm = false;
            }
        }

        // Reset everything
        if self.reset_confirm {
            let mut close = false;
            let mut confirm = false;
            let response = confirm_modal(
                ctx,
                "reset_all",
                &p,
                Key::ResetTitle.tr(lang),
                Key::ResetDesc.tr(lang),
                Key::Cancel.tr(lang),
                Key::ResetWord.tr(lang),
                true,
                &mut close,
                &mut confirm,
            );
            if response.should_close() {
                close = true;
            }
            if confirm {
                self.reset_everything();
                self.reset_confirm = false;
            } else if close {
                self.reset_confirm = false;
            }
        }

        // Import settings
        if let Some(import) = self.pending_import.take() {
            let mut close = false;
            let mut confirm = false;
            let response = confirm_modal(
                ctx,
                "import_settings",
                &p,
                Key::ImportConfirmTitle.tr(lang),
                Key::ImportConfirmDesc.tr(lang),
                Key::Cancel.tr(lang),
                Key::ImportWord.tr(lang),
                false,
                &mut close,
                &mut confirm,
            );
            if response.should_close() {
                close = true;
            }
            if confirm {
                self.apply_import(import);
            } else if !close {
                self.pending_import = Some(import);
            }
        }
    }

    fn draw_help_modal(&mut self, ctx: &egui::Context) {
        if !self.help_open {
            return;
        }
        let p = self.palette;
        let lang = self.lang;
        let mut close = false;
        let frame = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(16)
            .inner_margin(Margin::same(22));
        let response = egui::Modal::new(Id::unique("help_modal"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(540.0);
                ui.label(
                    RichText::new(Key::HelpTitle.tr(lang))
                        .size(20.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(Key::HelpIntro.tr(lang))
                        .size(11.5)
                        .color(p.muted),
                );
                ui.add_space(16.0);
                for (title, desc) in [
                    (Key::HelpStep1Title, Key::HelpStep1Desc),
                    (Key::HelpStep2Title, Key::HelpStep2Desc),
                    (Key::HelpStep3Title, Key::HelpStep3Desc),
                    (Key::HelpConflictTitle, Key::HelpConflictDesc),
                    (Key::HelpTrayTitle, Key::HelpTrayDesc),
                ] {
                    ui.label(
                        RichText::new(title.tr(lang))
                            .size(13.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(desc.tr(lang))
                            .size(11.0)
                            .color(p.muted),
                    );
                    ui.add_space(8.0);
                }
                ui.add_space(6.0);
                Frame::new()
                    .fill(p.accent_soft)
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(Key::HelpTip.tr(lang))
                                .size(10.5)
                                .color(p.accent),
                        );
                    });
                ui.add_space(14.0);
                if primary_button(ui, &p, Key::Close.tr(lang)).clicked() {
                    close = true;
                }
            });
        if response.should_close() || close {
            self.help_open = false;
        }
    }
}

impl eframe::App for SyncoraApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_picker();
        self.poll_file_dialog();
        self.poll_engine();
        self.poll_tray(&ctx);
        self.handle_close_request(&ctx);
        self.check_system_theme(&ctx);
        self.apply_theme_if_needed(&ctx);
        self.save_if_due();

        if self.last_tray_update.elapsed() > Duration::from_secs(5) {
            self.update_tray_text();
        }

        self.draw(ui);
        self.draw_modals(&ctx);
        self.draw_toasts(&ctx);

        // Tray menu clicks arrive on a channel that is only drained while a
        // frame runs, and eframe sleeps until something wakes it. Without a
        // periodic repaint a tray command would sit unhandled until the user
        // touched the window, so keep a slow tick alive even when idle.
        let tick = if self.dirty { 500 } else { 1000 };
        ctx.request_repaint_after(Duration::from_millis(tick));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_now();
    }
}

// ===========================================================================
// Small UI helpers
// ===========================================================================

#[derive(Clone, Copy)]
enum NavIcon {
    Tasks,
    History,
    Conflicts,
    Settings,
    About,
}

fn nav_item(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: NavIcon,
    label: &str,
    selected: bool,
    badge: Option<usize>,
) -> bool {
    let height = 36.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::click(),
    );
    if selected {
        ui.painter().rect_filled(rect, 8.0, p.accent_soft);
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + 9.0),
            Vec2::new(3.0, height - 18.0),
        );
        ui.painter().rect_filled(bar, 2.0, p.accent);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, p.card_hover);
    }

    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        Vec2::splat(16.0),
    );
    nav_icon(ui.painter(), icon_rect, icon, if selected { p.accent } else { p.muted });

    ui.painter().text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        if selected { p.text } else { p.muted },
    );

    if let Some(count) = badge {
        let text = count.to_string();
        let galley = ui.painter().layout_no_wrap(
            text.clone(),
            FontId::proportional(9.5),
            Color32::WHITE,
        );
        let size = galley.size() + Vec2::new(12.0, 3.0);
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 16.0, rect.center().y),
            size,
        );
        ui.painter().rect_filled(badge_rect, 7.0, p.warning);
        ui.painter()
            .galley(badge_rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }

    response.clicked()
}

fn nav_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let c = rect.center();
    match icon {
        NavIcon::Tasks => {
            let back = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 3.0),
                Vec2::new(9.0, 9.0),
            );
            let front = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 5.0, rect.top() + 6.0),
                Vec2::new(9.0, 9.0),
            );
            painter.rect_stroke(back, 2.0, stroke, StrokeKind::Inside);
            painter.rect_stroke(front, 2.0, stroke, StrokeKind::Inside);
        }
        NavIcon::History => {
            painter.circle_stroke(c, 6.5, stroke);
            painter.line_segment([c, egui::pos2(c.x, c.y - 3.5)], stroke);
            painter.line_segment([c, egui::pos2(c.x + 3.0, c.y + 1.5)], stroke);
        }
        NavIcon::Conflicts => {
            let points = [
                egui::pos2(c.x, rect.top() + 2.0),
                egui::pos2(rect.right() - 1.5, rect.bottom() - 2.0),
                egui::pos2(rect.left() + 1.5, rect.bottom() - 2.0),
            ];
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
            painter.text(
                egui::pos2(c.x, c.y + 1.0),
                Align2::CENTER_CENTER,
                "!",
                FontId::proportional(8.0),
                color,
            );
        }
        NavIcon::Settings => {
            for (index, y) in [4.0f32, 8.0, 12.0].iter().enumerate() {
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 2.0, rect.top() + y),
                        egui::pos2(rect.right() - 2.0, rect.top() + y),
                    ],
                    Stroke::new(1.3, color),
                );
                let x = match index {
                    0 => rect.left() + 10.0,
                    1 => rect.left() + 5.0,
                    _ => rect.left() + 11.0,
                };
                painter.circle_filled(egui::pos2(x, rect.top() + y), 2.0, color);
            }
        }
        NavIcon::About => {
            painter.circle_stroke(c, 6.5, stroke);
            painter.text(
                c,
                Align2::CENTER_CENTER,
                "i",
                FontId::proportional(9.0),
                color,
            );
        }
    }
}

fn draw_logo(ui: &mut egui::Ui, p: &Palette, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, size * 0.24, p.accent);
    let inset = size * 0.22;
    let inner = rect.shrink(inset);
    let stroke = Stroke::new((size * 0.055).max(1.4), Color32::WHITE);
    let y_top = inner.top() + inner.height() * 0.32;
    let y_bottom = inner.top() + inner.height() * 0.68;
    painter.line_segment(
        [
            egui::pos2(inner.left(), y_top),
            egui::pos2(inner.right() - inner.width() * 0.12, y_top),
        ],
        stroke,
    );
    painter.line_segment(
        [
            egui::pos2(inner.right(), y_bottom),
            egui::pos2(inner.left() + inner.width() * 0.12, y_bottom),
        ],
        stroke,
    );
    let head = size * 0.09;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(inner.right() - inner.width() * 0.12, y_top - head),
            egui::pos2(inner.right() - inner.width() * 0.12, y_top + head),
            egui::pos2(inner.right() + head * 0.4, y_top),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(inner.left() + inner.width() * 0.12, y_bottom - head),
            egui::pos2(inner.left() + inner.width() * 0.12, y_bottom + head),
            egui::pos2(inner.left() - head * 0.4, y_bottom),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
}

fn chip(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), FontId::proportional(9.5), fg);
    let size = galley.size() + Vec2::new(14.0, 4.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 5.0, bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, fg);
    response
}

fn mode_chip(ui: &mut egui::Ui, mode: SyncMode, lang: Lang, p: &Palette) -> egui::Response {
    let (fg, bg) = match mode {
        SyncMode::OneWay => (p.accent, p.accent_soft),
        SyncMode::TwoWay => (
            if p.dark {
                Color32::from_rgb(0xB9, 0xA2, 0xFF)
            } else {
                Color32::from_rgb(0x7C, 0x3A, 0xED)
            },
            Color32::from_rgba_unmultiplied(124, 58, 237, if p.dark { 46 } else { 22 }),
        ),
        SyncMode::Mirror => (p.warning, Color32::from_rgba_unmultiplied(217, 119, 6, 30)),
    };
    let text = match mode {
        SyncMode::OneWay => Key::ModeOneWay.tr(lang),
        SyncMode::TwoWay => Key::ModeTwoWay.tr(lang),
        SyncMode::Mirror => Key::ModeMirror.tr(lang),
    };
    chip(ui, text, fg, bg)
}

fn status_dot(ui: &mut egui::Ui, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    response
}

fn toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool, enabled: bool) -> egui::Response {
    let size = Vec2::new(40.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(
        size,
        if enabled { Sense::click() } else { Sense::hover() },
    );
    if enabled && response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui
        .ctx()
        .animate_bool_with_time(response.id, *on, 0.12);
    let bg = if !enabled {
        p.track
    } else if *on {
        p.accent
    } else {
        p.track
    };
    ui.painter().rect_filled(rect, 11.0, bg);
    let knob_x = egui::lerp(
        (rect.left() + 11.0)..=(rect.right() - 11.0),
        if enabled { t } else { 0.0 },
    );
    ui.painter().circle_filled(
        egui::pos2(knob_x, rect.center().y),
        8.0,
        if enabled { Color32::WHITE } else { p.faint },
    );
    response
}

fn primary_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).size(12.5).color(Color32::WHITE))
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .corner_radius(8)
            .min_size(Vec2::new(0.0, 30.0)),
    )
}

fn secondary_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).size(12.0).color(p.text))
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(8)
            .min_size(Vec2::new(0.0, 28.0)),
    )
}

fn ghost_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).size(11.5).color(p.muted))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(7)
            .min_size(Vec2::new(0.0, 26.0)),
    )
}

fn danger_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(
        Button::new(RichText::new(text).size(11.5).color(p.danger))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.35)))
            .corner_radius(7)
            .min_size(Vec2::new(0.0, 28.0)),
    )
}

fn help_badge(ui: &mut egui::Ui, p: &Palette, text: &str) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 7.0, p.track);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "?",
        FontId::proportional(9.0),
        p.muted,
    );
    response.on_hover_text(text);
}

fn setting_row(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    help: Option<&str>,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.5).color(p.text));
        if let Some(help) = help {
            help_badge(ui, p, help);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            control(ui);
        });
    });
    if let Some(help) = help {
        ui.label(RichText::new(help).size(9.8).color(p.faint));
    }
    ui.add_space(7.0);
}

fn settings_section(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14)
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(14.5).strong().color(p.text));
            ui.add_space(12.0);
            contents(ui);
        });
    ui.add_space(14.0);
}

fn interval_combo(
    ui: &mut egui::Ui,
    _p: &Palette,
    lang: Lang,
    id: &str,
    value: &mut u64,
) {
    let options = [
        (0u64, Key::IntervalManual),
        (60, Key::Interval1m),
        (300, Key::Interval5m),
        (900, Key::Interval15m),
        (1800, Key::Interval30m),
        (3600, Key::Interval1h),
        (21600, Key::Interval6h),
        (86400, Key::Interval24h),
    ];
    let selected = options
        .iter()
        .find(|(seconds, _)| seconds == value)
        .map(|(_, key)| key.tr(lang))
        .unwrap_or(Key::IntervalManual.tr(lang));
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(180.0)
        .show_ui(ui, |ui| {
            for (seconds, key) in options {
                ui.selectable_value(value, seconds, key.tr(lang));
            }
        });
}

fn empty_state(
    ui: &mut egui::Ui,
    p: &Palette,
    lang: Lang,
    title: Key,
    description: Key,
) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 28.0, p.accent_soft);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "✓",
            FontId::proportional(24.0),
            p.accent,
        );
        ui.add_space(16.0);
        ui.label(RichText::new(title.tr(lang)).size(18.0).strong().color(p.text));
        ui.add_space(4.0);
        ui.label(
            RichText::new(description.tr(lang))
                .size(11.5)
                .color(p.muted),
        );
    });
}

fn draw_task_card(
    ui: &mut egui::Ui,
    task: &SyncTask,
    runtime: &TaskRuntime,
    lang: Lang,
    p: &Palette,
    global_policy: ConflictPolicy,
    previewing: bool,
    actions: &mut Vec<(String, TaskAction)>,
) {
    let busy = runtime.is_busy();
    let last_result = task.last_result;
    let status_color = if !task.enabled {
        p.faint
    } else {
        match runtime.status {
            TaskStatus::Syncing | TaskStatus::Scanning | TaskStatus::Queued => p.accent,
            TaskStatus::Error => p.danger,
            TaskStatus::Idle => match last_result {
                Some(TaskResult::Success) => p.success,
                Some(TaskResult::Partial) => p.warning,
                Some(TaskResult::Failed) => p.danger,
                Some(TaskResult::Cancelled) => p.faint,
                None => p.faint,
            },
        }
    };

    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            ui.horizontal(|ui| {
                status_dot(ui, status_color)
                    .on_hover_text(Key::TipStatusDot.tr(lang));
                ui.add(
                    Label::new(
                        RichText::new(&task.name)
                            .size(15.5)
                            .strong()
                            .color(p.text),
                    )
                    .truncate(),
                );
                mode_chip(ui, task.mode, lang, p)
                    .on_hover_text(Key::TipModeChip.tr(lang));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut enabled = task.enabled;
                    let response = toggle(ui, p, &mut enabled, true);
                    response.on_hover_text(Key::TipTaskToggle.tr(lang));
                    if enabled != task.enabled {
                        actions.push((task.id.clone(), TaskAction::ToggleEnabled(enabled)));
                    }
                    ui.label(
                        RichText::new(if task.enabled {
                            Key::Enabled.tr(lang)
                        } else {
                            Key::PausedLabel.tr(lang)
                        })
                        .size(10.0)
                        .color(p.faint),
                    );
                });
            });

            ui.add_space(10.0);

            let copy_tooltip = Key::CopyPath.tr(lang);
            for (label, path, open_tooltip, open_action) in [
                (
                    Key::Source.tr(lang),
                    &task.source,
                    Key::TipOpenSource.tr(lang),
                    TaskAction::OpenSource,
                ),
                (
                    Key::Target.tr(lang),
                    &task.target,
                    Key::TipOpenTarget.tr(lang),
                    TaskAction::OpenTarget,
                ),
            ] {
                if let Some(clicked) =
                    path_row(ui, p, label, path, open_tooltip, copy_tooltip)
                {
                    let action = match clicked {
                        PathRowAction::Open => open_action,
                        PathRowAction::Copy => {
                            TaskAction::CopyPath(path.display().to_string())
                        }
                    };
                    actions.push((task.id.clone(), action));
                }
            }

            ui.add_space(7.0);
            ui.horizontal(|ui| {
                let last = task
                    .last_sync
                    .map(|ts| util::format_relative_time(ts, lang))
                    .unwrap_or_else(|| Key::Never.tr(lang).to_string());
                ui.label(
                    RichText::new(format!("{} · {}", Key::LastSync.tr(lang), last))
                        .size(10.5)
                        .color(p.faint),
                );
                if task.watch {
                    ui.label(RichText::new("·").size(9.0).color(p.faint));
                    ui.label(
                        RichText::new(Key::Watching.tr(lang))
                            .size(10.0)
                            .color(p.faint),
                    );
                }
                if task.interval_secs > 0 {
                    ui.label(RichText::new("·").size(9.0).color(p.faint));
                    ui.label(
                        RichText::new(interval_text(task.interval_secs, lang))
                            .size(10.0)
                            .color(p.faint),
                    );
                }
                let policy = task.effective_policy(global_policy);
                if task.mode == SyncMode::TwoWay {
                    ui.label(RichText::new("·").size(9.0).color(p.faint));
                    ui.label(
                        RichText::new(fill(
                            Key::OnConflict.tr(lang),
                            &[("{policy}", policy_label(policy, lang))],
                        ))
                        .size(10.0)
                        .color(p.faint),
                    );
                }
            });

            if runtime.status == TaskStatus::Scanning {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(format!(
                            "{}  {}",
                            Key::Scanning.tr(lang),
                            runtime.scanned
                        ))
                        .size(10.5)
                        .color(p.accent),
                    );
                });
            } else if runtime.status == TaskStatus::Syncing {
                ui.add_space(8.0);
                let fraction = if runtime.total == 0 {
                    0.0
                } else {
                    runtime.done as f32 / runtime.total as f32
                };
                progress_bar(ui, p, fraction);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(fill(
                            Key::ProgressFiles.tr(lang),
                            &[
                                ("{done}", &runtime.done.to_string()),
                                ("{total}", &runtime.total.to_string()),
                            ],
                        ))
                        .size(10.0)
                        .color(p.muted),
                    );
                    ui.label(
                        RichText::new(util::format_bytes(runtime.bytes))
                            .size(10.0)
                            .color(p.faint),
                    );
                    if !runtime.current.is_empty() {
                        ui.add(
                            Label::new(
                                RichText::new(util::shorten_path(&runtime.current, 60))
                                    .monospace()
                                    .size(9.5)
                                    .color(p.faint),
                            )
                            .truncate(),
                        );
                    }
                });
            } else if runtime.status == TaskStatus::Error || last_result == Some(TaskResult::Failed)
            {
                ui.add_space(6.0);
                if let Some(error) = &runtime.last_error {
                    ui.add(
                        Label::new(
                            RichText::new(util::shorten_path(error, 140))
                                .size(10.0)
                                .color(p.danger),
                        )
                        .truncate(),
                    )
                    .on_hover_text(error);
                }
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if busy {
                    if secondary_button(ui, p, Key::Cancel.tr(lang))
                        .on_hover_text(Key::TipCancel.tr(lang))
                        .clicked()
                    {
                        actions.push((task.id.clone(), TaskAction::Cancel));
                    }
                } else if primary_button(ui, p, Key::SyncNow.tr(lang))
                    .on_hover_text(Key::TipSyncNow.tr(lang))
                    .clicked()
                {
                    actions.push((task.id.clone(), TaskAction::SyncNow));
                }

                if secondary_button(
                    ui,
                    p,
                    if previewing {
                        Key::PreviewScanning.tr(lang)
                    } else {
                        Key::Preview.tr(lang)
                    },
                )
                .on_hover_text(Key::TipPreview.tr(lang))
                .clicked()
                    && !busy
                    && !previewing
                {
                    actions.push((task.id.clone(), TaskAction::Preview));
                }

                ui.add_enabled_ui(!busy, |ui| {
                    if ghost_button(ui, p, Key::Edit.tr(lang))
                        .on_hover_text(Key::TipEdit.tr(lang))
                        .clicked()
                    {
                        actions.push((task.id.clone(), TaskAction::Edit));
                    }
                    if ui
                        .add(
                            Button::new(
                                RichText::new(Key::Delete.tr(lang))
                                    .size(11.0)
                                    .color(p.danger),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(7)
                            .min_size(Vec2::new(0.0, 26.0)),
                        )
                        .on_hover_text(Key::TipDelete.tr(lang))
                        .clicked()
                    {
                        actions.push((task.id.clone(), TaskAction::Delete));
                    }
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (text, color) = task_status_text(task, runtime, lang, p);
                    ui.label(RichText::new(text).size(10.5).color(color));
                });
            });
        });
}

/// What the user clicked on in a path row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathRowAction {
    Open,
    Copy,
}

fn path_row(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    path: &std::path::Path,
    open_tooltip: &str,
    copy_tooltip: &str,
) -> Option<PathRowAction> {
    let mut result = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(48.0, 16.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(9.5).color(p.faint));
            },
        );
        let full = path.display().to_string();
        let available = (ui.available_width() - 52.0).max(80.0);
        let max_chars = (available / 6.2).max(20.0) as usize;
        let shown = util::shorten_path(&full, max_chars);
        ui.allocate_ui_with_layout(
            Vec2::new(available, 18.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add(
                    Label::new(
                        RichText::new(shown)
                            .monospace()
                            .size(10.8)
                            .color(p.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(&full);
            },
        );
        if copy_icon(ui, p, copy_tooltip).clicked() {
            result = Some(PathRowAction::Copy);
        }
        if open_icon(ui, p, open_tooltip).clicked() {
            result = Some(PathRowAction::Open);
        }
    });
    result
}

fn open_icon(ui: &mut egui::Ui, p: &Palette, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.card_hover);
    }
    let stroke = Stroke::new(1.2, p.muted);
    let box_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 5.0, rect.top() + 7.0),
        Vec2::new(9.0, 9.0),
    );
    ui.painter().rect_stroke(box_rect, 2.0, stroke, StrokeKind::Inside);
    ui.painter().line_segment(
        [
            egui::pos2(rect.left() + 11.0, rect.top() + 10.0),
            egui::pos2(rect.right() - 5.0, rect.top() + 5.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.right() - 9.0, rect.top() + 5.0),
            egui::pos2(rect.right() - 5.0, rect.top() + 5.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.right() - 5.0, rect.top() + 5.0),
            egui::pos2(rect.right() - 5.0, rect.top() + 9.0),
        ],
        stroke,
    );
    response.on_hover_text(tooltip)
}

/// Two stacked sheets: the usual "copy to clipboard" affordance.
fn copy_icon(ui: &mut egui::Ui, p: &Palette, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.card_hover);
    }
    let stroke = Stroke::new(1.2, p.muted);
    let back = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 9.0, rect.top() + 5.0),
        Vec2::new(8.0, 9.0),
    );
    ui.painter().rect_stroke(back, 2.0, stroke, StrokeKind::Inside);
    let front = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 5.0, rect.top() + 8.0),
        Vec2::new(8.0, 9.0),
    );
    ui.painter().rect_filled(front, 2.0, p.card);
    ui.painter().rect_stroke(front, 2.0, stroke, StrokeKind::Inside);
    response.on_hover_text(tooltip)
}

fn progress_bar(ui: &mut egui::Ui, p: &Palette, fraction: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 5.0), Sense::hover());
    ui.painter().rect_filled(rect, 2.5, p.track);
    let filled = (fraction.clamp(0.0, 1.0) * rect.width()).max(2.0);
    let fill_rect = egui::Rect::from_min_size(rect.min, Vec2::new(filled, rect.height()));
    ui.painter().rect_filled(fill_rect, 2.5, p.accent);
}

fn task_status_text(
    task: &SyncTask,
    runtime: &TaskRuntime,
    lang: Lang,
    p: &Palette,
) -> (String, Color32) {
    if !task.enabled {
        return (Key::PausedLabel.tr(lang).to_string(), p.faint);
    }
    match runtime.status {
        TaskStatus::Syncing => (Key::Syncing.tr(lang).to_string(), p.accent),
        TaskStatus::Scanning => (Key::Scanning.tr(lang).to_string(), p.accent),
        TaskStatus::Queued => (Key::Queued.tr(lang).to_string(), p.faint),
        TaskStatus::Error => (
            Key::ResultFailed.tr(lang).to_string(),
            p.danger,
        ),
        TaskStatus::Idle => match task.last_result {
            Some(TaskResult::Success) => (Key::UpToDate.tr(lang).to_string(), p.success),
            Some(TaskResult::Partial) => (Key::ResultPartial.tr(lang).to_string(), p.warning),
            Some(TaskResult::Failed) => (Key::ResultFailed.tr(lang).to_string(), p.danger),
            Some(TaskResult::Cancelled) => (Key::ResultCancelled.tr(lang).to_string(), p.faint),
            None => (Key::TaskNotSynced.tr(lang).to_string(), p.faint),
        },
    }
}

fn interval_text(seconds: u64, lang: Lang) -> String {
    match seconds {
        0 => Key::ManualOnly.tr(lang).to_string(),
        60 => Key::Interval1m.tr(lang).to_string(),
        300 => Key::Interval5m.tr(lang).to_string(),
        900 => Key::Interval15m.tr(lang).to_string(),
        1800 => Key::Interval30m.tr(lang).to_string(),
        3600 => Key::Interval1h.tr(lang).to_string(),
        21600 => Key::Interval6h.tr(lang).to_string(),
        86400 => Key::Interval24h.tr(lang).to_string(),
        other => util::format_duration(other, lang),
    }
}

fn result_label(result: TaskResult, lang: Lang) -> String {
    match result {
        TaskResult::Success => Key::ResultSuccess.tr(lang).to_string(),
        TaskResult::Partial => Key::ResultPartial.tr(lang).to_string(),
        TaskResult::Failed => Key::ResultFailed.tr(lang).to_string(),
        TaskResult::Cancelled => Key::ResultCancelled.tr(lang).to_string(),
    }
}

fn result_color(result: TaskResult, p: &Palette) -> Color32 {
    match result {
        TaskResult::Success => p.success,
        TaskResult::Partial => p.warning,
        TaskResult::Failed => p.danger,
        TaskResult::Cancelled => p.faint,
    }
}

fn draw_history_row(ui: &mut egui::Ui, entry: &HistoryEntry, lang: Lang, p: &Palette) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(11)
        .inner_margin(Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, result_color(entry.result, p));
                ui.label(
                    RichText::new(&entry.task_name)
                        .size(12.5)
                        .strong()
                        .color(p.text),
                );
                chip(
                    ui,
                    result_label(entry.result, lang).as_str(),
                    result_color(entry.result, p),
                    Color32::from_rgba_unmultiplied(
                        result_color(entry.result, p).r(),
                        result_color(entry.result, p).g(),
                        result_color(entry.result, p).b(),
                        if p.dark { 32 } else { 18 },
                    ),
                );
                ui.label(
                    RichText::new(mode_label(entry.mode, lang))
                        .size(9.5)
                        .color(p.faint),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let relative = util::format_relative_time(entry.finished_at, lang);
                    ui.label(RichText::new(relative).size(10.0).color(p.faint))
                        .on_hover_text(util::absolute_time(entry.finished_at));
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(fill(
                        Key::HistorySummary.tr(lang),
                        &[
                            ("{copied}", &entry.copied.to_string()),
                            ("{updated}", &entry.updated.to_string()),
                            ("{deleted}", &entry.deleted.to_string()),
                            ("{conflicts}", &entry.conflicts.to_string()),
                        ],
                    ))
                    .size(10.0)
                    .color(p.muted),
                );
                ui.label(RichText::new("·").size(9.0).color(p.faint));
                ui.label(
                    RichText::new(format!(
                        "{} {}",
                        Key::Duration.tr(lang),
                        util::format_duration(
                            (entry.finished_at - entry.started_at).max(0) as u64,
                            lang
                        )
                    ))
                    .size(10.0)
                    .color(p.faint),
                );
                if entry.bytes > 0 {
                    ui.label(RichText::new("·").size(9.0).color(p.faint));
                    ui.label(
                        RichText::new(util::format_bytes(entry.bytes))
                            .size(10.0)
                            .color(p.faint),
                    );
                }
            });
            if let Some(message) = &entry.message {
                ui.add_space(3.0);
                ui.add(
                    Label::new(
                        RichText::new(util::shorten_path(message, 150))
                            .size(9.5)
                            .color(if entry.result == TaskResult::Success {
                                p.faint
                            } else {
                                p.danger
                            }),
                    )
                    .truncate(),
                )
                .on_hover_text(message);
            }
        });
}

fn mode_label(mode: SyncMode, lang: Lang) -> &'static str {
    match mode {
        SyncMode::OneWay => Key::ModeOneWay.tr(lang),
        SyncMode::TwoWay => Key::ModeTwoWay.tr(lang),
        SyncMode::Mirror => Key::ModeMirror.tr(lang),
    }
}

fn draw_version_box(
    ui: &mut egui::Ui,
    p: &Palette,
    lang: Lang,
    label: Key,
    size: Option<u64>,
    mtime: Option<i64>,
    missing: bool,
) {
    Frame::new()
        .fill(p.input)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(label.tr(lang)).size(9.5).color(p.faint));
            if missing {
                ui.label(
                    RichText::new(if label == Key::ConflictSourceVersion {
                        Key::ConflictSourceMissing.tr(lang)
                    } else {
                        Key::ConflictTargetMissing.tr(lang)
                    })
                    .size(10.5)
                    .color(p.danger),
                );
            } else {
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {}",
                        util::format_bytes(size.unwrap_or(0)),
                        mtime
                            .map(util::absolute_time)
                            .unwrap_or_else(|| Key::Unknown.tr(lang).to_string())
                    ))
                    .size(10.0)
                    .color(p.muted),
                );
            }
        });
}

fn preview_stat(ui: &mut egui::Ui, p: &Palette, label: &str, value: u64, color: Color32) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(7.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, color);
        ui.label(RichText::new(label).size(11.0).color(p.muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value.to_string()).size(15.0).strong().color(p.text));
        });
    });
    ui.add_space(4.0);
}

fn confirm_modal(
    ctx: &egui::Context,
    id: &str,
    p: &Palette,
    title: &str,
    description: &str,
    cancel_text: &str,
    confirm_text: &str,
    danger: bool,
    close: &mut bool,
    confirm: &mut bool,
) -> egui::ModalResponse<()> {
    let frame = Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14)
        .inner_margin(Margin::same(20));
    egui::Modal::new(Id::unique(id))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(title).size(17.0).strong().color(p.text));
            ui.add_space(6.0);
            ui.label(RichText::new(description).size(11.5).color(p.muted));
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                if danger {
                    if ui
                        .add(
                            Button::new(
                                RichText::new(confirm_text)
                                    .size(12.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(p.danger)
                            .stroke(Stroke::NONE)
                            .corner_radius(8)
                            .min_size(Vec2::new(0.0, 30.0)),
                        )
                        .clicked()
                    {
                        *confirm = true;
                    }
                } else if primary_button(ui, p, confirm_text).clicked() {
                    *confirm = true;
                }
                if secondary_button(ui, p, cancel_text).clicked() {
                    *close = true;
                }
            });
        })
}

fn failure_text(lang: Lang, failure: &crate::engine::JobFailure) -> String {
    let prefix = match failure.code {
        FailureCode::SourceMissing => Key::ErrSourceMissing.tr(lang),
        FailureCode::TargetMissing => Key::ErrTargetMissing.tr(lang),
        FailureCode::ScanFailed => Key::ErrScan.tr(lang),
        FailureCode::Other => Key::ErrOther.tr(lang),
    };
    if failure.detail.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} — {}", failure.detail)
    }
}

fn path_issue_text(lang: Lang, issue: PathIssue) -> String {
    match issue {
        PathIssue::SourceMissing => Key::ValidationSourceMissing.tr(lang),
        PathIssue::SourceIsFile => Key::ValidationSourceIsFile.tr(lang),
        PathIssue::TargetIsFile => Key::ValidationTargetIsFile.tr(lang),
        PathIssue::Same => Key::ValidationSame.tr(lang),
        PathIssue::Nested => Key::ValidationNested.tr(lang),
    }
    .to_string()
}

fn editor_from_task(task: &SyncTask) -> TaskEditor {
    TaskEditor {
        id: Some(task.id.clone()),
        name: task.name.clone(),
        source: task.source.display().to_string(),
        target: task.target.display().to_string(),
        mode: task.mode,
        enabled: task.enabled,
        watch: task.watch,
        interval_secs: task.interval_secs,
        excludes: task.excludes.join("\n"),
        inherit_policy: task.conflict_policy.is_none(),
        policy: task.conflict_policy.unwrap_or(ConflictPolicy::Ask),
        advanced: false,
        error: None,
    }
}

fn draw_editor_contents(
    ui: &mut egui::Ui,
    editor: &mut TaskEditor,
    p: &Palette,
    lang: Lang,
    global_policy: ConflictPolicy,
    action: &mut Option<EditorAction>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if editor.id.is_some() {
                Key::EditTaskTitle.tr(lang)
            } else {
                Key::NewTaskTitle.tr(lang)
            })
            .size(19.0)
            .strong()
            .color(p.text),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(
                    Button::new(RichText::new("×").size(18.0).color(p.muted))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE),
                )
                .clicked()
            {
                *action = Some(EditorAction::Cancel);
            }
        });
    });
    ui.add_space(12.0);

    ui.label(RichText::new(Key::FieldName.tr(lang)).size(10.5).color(p.faint));
    ui.add(
        egui::TextEdit::singleline(&mut editor.name)
            .hint_text(Key::FieldNameHint.tr(lang))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(10.0);

    ui.label(RichText::new(Key::FieldSource.tr(lang)).size(10.5).color(p.faint));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut editor.source)
                .hint_text("C:\\Users\\you\\Documents")
                .desired_width(ui.available_width() - 84.0),
        );
        if secondary_button(ui, p, Key::Browse.tr(lang)).clicked() {
            *action = Some(EditorAction::Browse(PickTarget::Source));
        }
    });
    ui.add_space(8.0);

    ui.label(RichText::new(Key::FieldTarget.tr(lang)).size(10.5).color(p.faint));
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut editor.target)
                .hint_text("D:\\Backup\\Documents")
                .desired_width(ui.available_width() - 84.0),
        );
        if secondary_button(ui, p, Key::Browse.tr(lang)).clicked() {
            *action = Some(EditorAction::Browse(PickTarget::Target));
        }
    });
    ui.label(RichText::new(Key::PathHint.tr(lang)).size(9.5).color(p.faint));
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        ui.label(RichText::new(Key::ModeLabel.tr(lang)).size(10.5).color(p.faint));
        help_badge(ui, p, Key::ModeHelp.tr(lang));
    });
    ui.add_space(4.0);
    ui.columns(3, |columns| {
        for (index, mode) in [SyncMode::OneWay, SyncMode::TwoWay, SyncMode::Mirror]
            .into_iter()
            .enumerate()
        {
            draw_mode_card(&mut columns[index], p, lang, editor, mode);
        }
    });
    if editor.mode == SyncMode::Mirror {
        ui.add_space(4.0);
        ui.label(
            RichText::new(Key::MirrorWarning.tr(lang))
                .size(9.5)
                .color(p.warning),
        );
    } else if editor.mode == SyncMode::TwoWay {
        ui.add_space(4.0);
        ui.label(
            RichText::new(Key::TwoWayWarning.tr(lang))
                .size(9.5)
                .color(p.warning),
        );
    }
    ui.add_space(12.0);

    ui.label(RichText::new(Key::Automation.tr(lang)).size(10.5).color(p.faint));
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut editor.watch, Key::WatchLabel.tr(lang));
        help_badge(ui, p, Key::WatchHelp.tr(lang));
        ui.add_space(12.0);
        ui.label(RichText::new(Key::IntervalLabel.tr(lang)).size(10.5).color(p.faint));
        interval_combo(ui, p, lang, "editor_interval", &mut editor.interval_secs);
        help_badge(ui, p, Key::IntervalHelp.tr(lang));
    });
    ui.add_space(6.0);

    // `default_open` (not `open`) so the user's click is what drives the state.
    let advanced = egui::CollapsingHeader::new(
        RichText::new(Key::Advanced.tr(lang)).size(11.5).color(p.muted),
    )
    .default_open(editor.advanced)
    .show(ui, |ui| {
        // Two columns keep the dialog short enough to fit on 768 px tall screens.
        ui.columns(2, |columns| {
            columns[0].label(
                RichText::new(Key::ExcludesLabel.tr(lang))
                    .size(10.0)
                    .color(p.faint),
            );
            columns[0].add(
                egui::TextEdit::multiline(&mut editor.excludes)
                    .hint_text("*.tmp\ncache/**\n**/node_modules")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            columns[0].label(
                RichText::new(Key::ExcludesHelp.tr(lang))
                    .size(9.2)
                    .color(p.faint),
            );

            columns[1].label(
                RichText::new(Key::ConflictLabel.tr(lang))
                    .size(10.0)
                    .color(p.faint),
            );
            columns[1].checkbox(
                &mut editor.inherit_policy,
                Key::ConflictInheritShort.tr(lang),
            );
            if editor.inherit_policy {
                columns[1].label(
                    RichText::new(fill(
                        Key::ConflictInherit.tr(lang),
                        &[("{policy}", policy_label(global_policy, lang))],
                    ))
                    .size(9.5)
                    .color(p.faint),
                );
            } else {
                egui::ComboBox::from_id_salt("editor_policy")
                    .selected_text(policy_label(editor.policy, lang))
                    .width(170.0)
                    .show_ui(&mut columns[1], |ui| {
                        for option in [
                            ConflictPolicy::Ask,
                            ConflictPolicy::KeepBoth,
                            ConflictPolicy::PreferSource,
                            ConflictPolicy::PreferTarget,
                            ConflictPolicy::Skip,
                        ] {
                            ui.selectable_value(
                                &mut editor.policy,
                                option,
                                policy_label(option, lang),
                            );
                        }
                    });
            }
            columns[1].label(
                RichText::new(Key::ConflictHelp.tr(lang))
                    .size(9.2)
                    .color(p.faint),
            );
        });
    });
    editor.advanced = advanced.openness > 0.5;

    if let Some(error) = &editor.error {
        ui.add_space(10.0);
        Frame::new()
            .fill(Color32::from_rgba_unmultiplied(
                p.danger.r(),
                p.danger.g(),
                p.danger.b(),
                if p.dark { 36 } else { 18 },
            ))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.label(RichText::new(error).size(10.5).color(p.danger));
            });
    }

    ui.add_space(16.0);
    ui.horizontal(|ui| {
        let label = if editor.id.is_some() {
            Key::SaveChanges.tr(lang)
        } else {
            Key::CreateTask.tr(lang)
        };
        if primary_button(ui, p, label).clicked() {
            *action = Some(EditorAction::Save);
        }
        if secondary_button(ui, p, Key::Cancel.tr(lang)).clicked() {
            *action = Some(EditorAction::Cancel);
        }
    });
}

fn draw_mode_card(
    ui: &mut egui::Ui,
    p: &Palette,
    lang: Lang,
    editor: &mut TaskEditor,
    mode: SyncMode,
) {
    let selected = editor.mode == mode;
    let (title, description) = match mode {
        SyncMode::OneWay => (Key::ModeOneWay, Key::ModeOneWayDesc),
        SyncMode::TwoWay => (Key::ModeTwoWay, Key::ModeTwoWayDesc),
        SyncMode::Mirror => (Key::ModeMirror, Key::ModeMirrorDesc),
    };
    let response = Frame::new()
        .fill(if selected { p.accent_soft } else { p.input })
        .stroke(Stroke::new(1.0, if selected { p.accent } else { p.border }))
        .corner_radius(9)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter()
                    .circle_stroke(rect.center(), 5.5, Stroke::new(1.2, p.faint));
                if selected {
                    ui.painter().circle_filled(rect.center(), 3.0, p.accent);
                }
                ui.label(
                    RichText::new(title.tr(lang))
                        .size(11.5)
                        .strong()
                        .color(if selected { p.accent } else { p.text }),
                );
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(match mode {
                    SyncMode::OneWay => Key::ModeOneWayShort.tr(lang),
                    SyncMode::TwoWay => Key::ModeTwoWayShort.tr(lang),
                    SyncMode::Mirror => Key::ModeMirrorShort.tr(lang),
                })
                .size(9.0)
                .color(p.faint),
            );
        })
        .response
        .interact(Sense::click());

    response.clone().on_hover_text(description.tr(lang));
    if response.clicked() {
        editor.mode = mode;
    }
}
