//! Background sync engine.
//!
//! The UI talks to this module through a command channel and receives progress
//! through an event channel. All disk work happens on worker threads, so the
//! interface never freezes, even during very large syncs.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{ConflictPolicy, SyncMode, SyncTask};
use crate::sync::{
    self, ConflictInfo, ExecOptions, Plan, Resolution, ScanResult, Snapshot,
};
use crate::util;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub tasks: Vec<SyncTask>,
    pub global_enabled: bool,
    pub default_policy: ConflictPolicy,
    pub verify: bool,
    pub skip_hidden_system: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            global_enabled: true,
            default_policy: ConflictPolicy::Ask,
            verify: false,
            skip_hidden_system: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureCode {
    SourceMissing,
    TargetMissing,
    ScanFailed,
    Other,
}

#[derive(Clone, Debug)]
pub struct JobFailure {
    pub code: FailureCode,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct SyncSummary {
    pub mode: SyncMode,
    pub started_at: i64,
    pub finished_at: i64,
    pub copied: u64,
    pub updated: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub conflicts: Vec<ConflictInfo>,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PreviewData {
    pub new_files: u64,
    pub updated_files: u64,
    pub deleted_files: u64,
    pub conflicts: u64,
    pub bytes: u64,
    pub total_actions: usize,
    pub sample: Vec<String>,
    pub source_files: u64,
    pub target_files: u64,
    pub source_bytes: u64,
    pub target_bytes: u64,
}

#[derive(Clone, Debug)]
pub enum Event {
    ScanProgress {
        task_id: String,
        files: u64,
    },
    Started {
        task_id: String,
        total: usize,
    },
    Progress {
        task_id: String,
        done: usize,
        total: usize,
        current: String,
        bytes: u64,
    },
    Finished {
        task_id: String,
        summary: SyncSummary,
    },
    Failed {
        task_id: String,
        failure: JobFailure,
    },
    PreviewReady {
        task_id: String,
        preview: PreviewData,
    },
    PreviewFailed {
        task_id: String,
        failure: JobFailure,
    },
    ConflictResolved {
        task_id: String,
        conflict_id: String,
        result: Result<(), String>,
    },
    WatcherWarning {
        task_id: String,
        error: String,
    },
    SnapshotWarning {
        task_id: String,
        error: String,
    },
}

#[derive(Clone, Debug)]
pub enum Command {
    SetConfig(EngineConfig),
    SyncNow(String),
    SyncAll,
    Preview(String),
    Cancel(String),
    ResolveConflict {
        task_id: String,
        rel_path: String,
        conflict_id: String,
        choice: Resolution,
    },
    Shutdown,
}

enum Input {
    Command(Command),
    FsEvent(String),
    JobDone {
        task_id: String,
        outcome: JobOutcome,
    },
}

enum JobOutcome {
    Synced {
        summary: SyncSummary,
        snapshot: Snapshot,
    },
    Preview(Result<PreviewData, JobFailure>),
    Failed(JobFailure),
}

struct RunningJob {
    task_id: String,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone)]
struct WatchSpec {
    id: String,
    source: PathBuf,
    target: PathBuf,
    target_exists: bool,
}

impl PartialEq for WatchSpec {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.source == other.source
            && self.target == other.target
            && self.target_exists == other.target_exists
    }
}

pub struct EngineHandle {
    tx: Sender<Input>,
    rx: Receiver<Event>,
    join: Option<thread::JoinHandle<()>>,
}

impl EngineHandle {
    pub fn spawn(ctx: egui::Context) -> EngineHandle {
        let (input_tx, input_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());

        let engine_tx = input_tx.clone();
        let join = thread::Builder::new()
            .name("syncora-engine".to_string())
            .spawn(move || {
                let mut engine = Engine::new(engine_tx, event_tx, repaint);
                engine.run(input_rx);
            })
            .ok();

        EngineHandle {
            tx: input_tx,
            rx: event_rx,
            join,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(Input::Command(command));
    }

    pub fn try_recv(&self) -> Option<Event> {
        match self.rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Input::Command(Command::Shutdown));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Input::Command(Command::Shutdown));
    }
}

struct Engine {
    input_tx: Sender<Input>,
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    config: EngineConfig,
    watchers: HashMap<String, RecommendedWatcher>,
    watch_specs: Vec<WatchSpec>,
    warned_watchers: HashSet<String>,
    queue: VecDeque<String>,
    queued: HashSet<String>,
    running: Option<RunningJob>,
    previewing: HashSet<String>,
    next_due: HashMap<String, Instant>,
    task_signatures: HashMap<String, (PathBuf, PathBuf, u64, bool, bool)>,
    pending_fs: HashMap<String, Instant>,
    suppress_until: HashMap<String, Instant>,
    last_watch_check: Instant,
    last_global: bool,
    shutdown: bool,
}

impl Engine {
    fn new(
        input_tx: Sender<Input>,
        events: Sender<Event>,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            input_tx,
            events,
            repaint,
            config: EngineConfig::default(),
            watchers: HashMap::new(),
            watch_specs: Vec::new(),
            warned_watchers: HashSet::new(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            running: None,
            previewing: HashSet::new(),
            next_due: HashMap::new(),
            task_signatures: HashMap::new(),
            pending_fs: HashMap::new(),
            suppress_until: HashMap::new(),
            last_watch_check: Instant::now(),
            last_global: true,
            shutdown: false,
        }
    }

    fn run(&mut self, rx: Receiver<Input>) {
        loop {
            if self.shutdown {
                break;
            }
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(input) => {
                    self.handle_input(input);
                    // Drain queued input before doing any work.
                    while let Ok(next) = rx.try_recv() {
                        self.handle_input(next);
                        if self.shutdown {
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if self.shutdown {
                break;
            }
            self.tick();
        }
        self.watchers.clear();
    }

    fn handle_input(&mut self, input: Input) {
        match input {
            Input::Command(command) => self.handle_command(command),
            Input::FsEvent(task_id) => {
                if !self.config.global_enabled {
                    return;
                }
                if self.running.as_ref().map(|r| r.task_id == task_id).unwrap_or(false) {
                    return;
                }
                if let Some(until) = self.suppress_until.get(&task_id) {
                    if Instant::now() < *until {
                        return;
                    }
                }
                self.pending_fs.insert(task_id, Instant::now());
            }
            Input::JobDone { task_id, outcome } => self.handle_job_done(task_id, outcome),
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::SetConfig(config) => self.set_config(config),
            Command::SyncNow(task_id) => {
                if self.config.global_enabled {
                    self.enqueue(task_id);
                }
            }
            Command::SyncAll => {
                if self.config.global_enabled {
                    let ids: Vec<String> = self
                        .config
                        .tasks
                        .iter()
                        .filter(|task| task.enabled)
                        .map(|task| task.id.clone())
                        .collect();
                    for id in ids {
                        self.enqueue(id);
                    }
                }
            }
            Command::Preview(task_id) => self.start_preview(task_id),
            Command::Cancel(task_id) => {
                self.queue.retain(|id| id != &task_id);
                self.queued.remove(&task_id);
                if let Some(running) = &self.running {
                    if running.task_id == task_id {
                        running.cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
            Command::ResolveConflict {
                task_id,
                rel_path,
                conflict_id,
                choice,
            } => self.resolve_conflict(task_id, rel_path, conflict_id, choice),
            Command::Shutdown => {
                self.shutdown = true;
                if let Some(running) = &self.running {
                    running.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    fn set_config(&mut self, config: EngineConfig) {
        let now = Instant::now();
        let previous_global = self.last_global;
        self.last_global = config.global_enabled;

        let mut seen = HashSet::new();
        for task in &config.tasks {
            seen.insert(task.id.clone());
            let signature = (
                task.source.clone(),
                task.target.clone(),
                task.interval_secs,
                task.watch,
                task.enabled,
            );
            let changed = self
                .task_signatures
                .get(&task.id)
                .map(|old| old != &signature)
                .unwrap_or(true);
            if changed {
                self.task_signatures.insert(task.id.clone(), signature);
                if task.enabled && (task.watch || task.interval_secs > 0) {
                    self.next_due.insert(task.id.clone(), now + Duration::from_secs(2));
                } else {
                    self.next_due.remove(&task.id);
                }
            } else if !task.enabled {
                self.next_due.remove(&task.id);
            }
        }

        self.next_due.retain(|id, _| seen.contains(id));
        self.task_signatures.retain(|id, _| seen.contains(id));

        if !config.global_enabled {
            self.queue.clear();
            self.queued.clear();
            self.pending_fs.clear();
            if let Some(running) = &self.running {
                running.cancel.store(true, Ordering::Relaxed);
            }
        } else if !previous_global {
            // Catch up after the master switch is turned back on.
            for task in &config.tasks {
                if task.enabled && (task.watch || task.interval_secs > 0) {
                    self.next_due
                        .insert(task.id.clone(), now + Duration::from_secs(1));
                }
            }
        }

        self.config = config;
        self.ensure_watchers(true);
    }

    fn enqueue(&mut self, task_id: String) {
        if !self.config.global_enabled {
            return;
        }
        let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id) else {
            return;
        };
        if !task.enabled {
            return;
        }
        if self.running.as_ref().map(|r| r.task_id == task_id).unwrap_or(false) {
            return;
        }
        if self.queued.contains(&task_id) {
            return;
        }
        self.queued.insert(task_id.clone());
        self.queue.push_back(task_id);
    }

    fn tick(&mut self) {
        let now = Instant::now();

        if now.duration_since(self.last_watch_check) > Duration::from_secs(5) {
            self.last_watch_check = now;
            self.ensure_watchers(false);
        }

        // Debounced filesystem events.
        let mut ready = Vec::new();
        for (task_id, last) in &self.pending_fs {
            if now.duration_since(*last) >= Duration::from_millis(1500) {
                ready.push(task_id.clone());
            }
        }
        for task_id in ready {
            self.pending_fs.remove(&task_id);
            if let Some(until) = self.suppress_until.get(&task_id) {
                if now < *until {
                    continue;
                }
            }
            self.enqueue(task_id);
        }

        // Scheduled checks.
        if self.config.global_enabled && self.running.is_none() {
            let mut due: Vec<(String, Instant)> = self
                .next_due
                .iter()
                .filter(|(_, when)| **when <= now)
                .map(|(id, when)| (id.clone(), *when))
                .collect();
            due.sort_by_key(|(_, when)| *when);
            for (task_id, _) in due {
                let Some((enabled, interval_secs)) = self
                    .config
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .map(|t| (t.enabled, t.interval_secs))
                else {
                    continue;
                };
                if !enabled {
                    self.next_due.remove(&task_id);
                    continue;
                }
                self.enqueue(task_id.clone());
                if interval_secs > 0 {
                    self.next_due
                        .insert(task_id, now + Duration::from_secs(interval_secs));
                } else {
                    self.next_due.remove(&task_id);
                }
            }
        }

        self.start_next();
    }

    fn start_next(&mut self) {
        if self.running.is_some() {
            return;
        }
        while let Some(task_id) = self.queue.pop_front() {
            self.queued.remove(&task_id);
            let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id).cloned() else {
                continue;
            };
            if !task.enabled || !self.config.global_enabled {
                continue;
            }
            let snapshot = Snapshot::load(&task.id);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_config = WorkerConfig {
                policy: task.effective_policy(self.config.default_policy),
                verify: self.config.verify,
                skip_hidden_system: self.config.skip_hidden_system,
            };
            let events = self.events.clone();
            let repaint = self.repaint.clone();
            let tx = self.input_tx.clone();
            let worker_task = task.clone();
            let worker_cancel = cancel.clone();
            let worker_id = task.id.clone();

            let spawned = thread::Builder::new()
                .name(format!("syncora-sync-{}", task.id))
                .spawn(move || {
                    let outcome = run_sync_job(
                        worker_task,
                        worker_config,
                        snapshot,
                        worker_cancel,
                        events,
                        repaint,
                    );
                    let _ = tx.send(Input::JobDone {
                        task_id: worker_id,
                        outcome,
                    });
                });

            match spawned {
                Ok(_) => {
                    self.running = Some(RunningJob {
                        task_id: task.id.clone(),
                        cancel,
                    });
                    break;
                }
                Err(error) => {
                    let _ = self.events.send(Event::Failed {
                        task_id: task.id.clone(),
                        failure: JobFailure {
                            code: FailureCode::Other,
                            detail: format!("could not start worker: {error}"),
                        },
                    });
                    (self.repaint)();
                }
            }
        }
    }

    fn handle_job_done(&mut self, task_id: String, outcome: JobOutcome) {
        match outcome {
            JobOutcome::Synced { summary, snapshot } => {
                if self
                    .running
                    .as_ref()
                    .map(|r| r.task_id == task_id)
                    .unwrap_or(false)
                {
                    self.running = None;
                }
                self.suppress_until
                    .insert(task_id.clone(), Instant::now() + Duration::from_secs(2));
                if let Err(error) = snapshot.save(&task_id) {
                    let _ = self.events.send(Event::SnapshotWarning {
                        task_id: task_id.clone(),
                        error,
                    });
                }
                if let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id) {
                    if task.interval_secs > 0 {
                        self.next_due.insert(
                            task_id.clone(),
                            Instant::now() + Duration::from_secs(task.interval_secs),
                        );
                    } else {
                        self.next_due.remove(&task_id);
                    }
                }
                let _ = self.events.send(Event::Finished { task_id, summary });
            }
            JobOutcome::Preview(result) => {
                self.previewing.remove(&task_id);
                match result {
                    Ok(preview) => {
                        let _ = self.events.send(Event::PreviewReady { task_id, preview });
                    }
                    Err(failure) => {
                        let _ = self.events.send(Event::PreviewFailed { task_id, failure });
                    }
                }
                (self.repaint)();
                return;
            }
            JobOutcome::Failed(failure) => {
                if self
                    .running
                    .as_ref()
                    .map(|r| r.task_id == task_id)
                    .unwrap_or(false)
                {
                    self.running = None;
                }
                self.suppress_until
                    .insert(task_id.clone(), Instant::now() + Duration::from_secs(2));
                let _ = self.events.send(Event::Failed { task_id, failure });
            }
        }
        (self.repaint)();
        self.start_next();
    }

    fn resolve_conflict(
        &mut self,
        task_id: String,
        rel_path: String,
        conflict_id: String,
        choice: Resolution,
    ) {
        let result = (|| -> Result<(), String> {
            if self
                .running
                .as_ref()
                .map(|r| r.task_id == task_id)
                .unwrap_or(false)
            {
                return Err("this task is syncing right now".to_string());
            }
            let task = self
                .config
                .tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| "task no longer exists".to_string())?;
            let mut snapshot = Snapshot::load(&task_id);
            sync::apply_resolution(
                &task.source,
                &task.target,
                &rel_path,
                choice,
                &mut snapshot,
                self.config.verify,
            )?;
            snapshot.save(&task_id)?;
            Ok(())
        })();

        let ok = result.is_ok();
        let _ = self.events.send(Event::ConflictResolved {
            task_id: task_id.clone(),
            conflict_id,
            result,
        });
        (self.repaint)();
        if ok {
            self.enqueue(task_id);
        }
    }

    fn start_preview(&mut self, task_id: String) {
        if self.previewing.contains(&task_id) {
            return;
        }
        let Some(task) = self.config.tasks.iter().find(|t| t.id == task_id).cloned() else {
            return;
        };
        if self.running.as_ref().map(|r| r.task_id == task_id).unwrap_or(false) {
            let _ = self.events.send(Event::PreviewFailed {
                task_id,
                failure: JobFailure {
                    code: FailureCode::Other,
                    detail: "this task is syncing right now".to_string(),
                },
            });
            (self.repaint)();
            return;
        }

        self.previewing.insert(task_id.clone());
        let tx = self.input_tx.clone();
        let config = WorkerConfig {
            policy: task.effective_policy(self.config.default_policy),
            verify: self.config.verify,
            skip_hidden_system: self.config.skip_hidden_system,
        };
        let worker_id = task_id.clone();
        let spawned = thread::Builder::new()
            .name(format!("syncora-preview-{}", task.id))
            .spawn(move || {
                let outcome = run_preview_job(task, config);
                let _ = tx.send(Input::JobDone {
                    task_id: worker_id,
                    outcome,
                });
            });
        if let Err(error) = spawned {
            self.previewing.remove(&task_id);
            let _ = self.events.send(Event::PreviewFailed {
                task_id,
                failure: JobFailure {
                    code: FailureCode::Other,
                    detail: format!("could not start preview: {error}"),
                },
            });
            (self.repaint)();
        }
    }

    fn ensure_watchers(&mut self, force: bool) {
        let mut specs = Vec::new();
        if self.config.global_enabled {
            for task in &self.config.tasks {
                if task.enabled && task.watch {
                    specs.push(WatchSpec {
                        id: task.id.clone(),
                        source: task.source.clone(),
                        target: task.target.clone(),
                        target_exists: task.target.exists(),
                    });
                }
            }
        }
        if !force && specs == self.watch_specs {
            return;
        }

        self.watchers.clear();
        self.watch_specs = specs;
        let specs = self.watch_specs.clone();
        for spec in &specs {
            let tx = self.input_tx.clone();
            let task_id = spec.id.clone();
            let handler = move |result: notify::Result<notify::Event>| {
                if result.is_ok() {
                    let _ = tx.send(Input::FsEvent(task_id.clone()));
                }
            };
            match RecommendedWatcher::new(handler, notify::Config::default()) {
                Ok(mut watcher) => {
                    let mut failed = false;
                    if let Err(error) = watcher.watch(&spec.source, RecursiveMode::Recursive) {
                        failed = true;
                        self.warn_watcher(&spec.id, error.to_string());
                    }
                    if spec.target_exists {
                        if let Err(error) = watcher.watch(&spec.target, RecursiveMode::Recursive) {
                            failed = true;
                            self.warn_watcher(&spec.id, error.to_string());
                        }
                    }
                    if !failed {
                        self.watchers.insert(spec.id.clone(), watcher);
                    }
                }
                Err(error) => {
                    self.warn_watcher(&spec.id, error.to_string());
                }
            }
        }
    }

    fn warn_watcher(&mut self, task_id: &str, error: String) {
        if self.warned_watchers.insert(task_id.to_string()) {
            let _ = self.events.send(Event::WatcherWarning {
                task_id: task_id.to_string(),
                error,
            });
            (self.repaint)();
        }
    }
}

struct WorkerConfig {
    policy: ConflictPolicy,
    verify: bool,
    skip_hidden_system: bool,
}

fn run_sync_job(
    task: SyncTask,
    config: WorkerConfig,
    mut snapshot: Snapshot,
    cancel: Arc<AtomicBool>,
    events: Sender<Event>,
    repaint: Arc<dyn Fn() + Send + Sync>,
) -> JobOutcome {
    let started_at = util::now_secs();

    if !task.source.exists() {
        return JobOutcome::Failed(JobFailure {
            code: FailureCode::SourceMissing,
            detail: task.source.display().to_string(),
        });
    }
    if !task.target.exists() {
        if let Err(error) = std::fs::create_dir_all(&task.target) {
            return JobOutcome::Failed(JobFailure {
                code: FailureCode::TargetMissing,
                detail: format!("{}: {error}", task.target.display()),
            });
        }
    }

    let excludes = match sync::build_excludes(&task.all_excludes()) {
        Ok(set) => set,
        Err(error) => {
            return JobOutcome::Failed(JobFailure {
                code: FailureCode::Other,
                detail: error,
            })
        }
    };

    let task_id = task.id.clone();
    let events_scan = events.clone();
    let repaint_scan = repaint.clone();
    let scan_id = task_id.clone();
    let src = match sync::scan_tree(
        &task.source,
        &excludes,
        config.skip_hidden_system,
        move |files| {
            let _ = events_scan.send(Event::ScanProgress {
                task_id: scan_id.clone(),
                files,
            });
            (repaint_scan)();
        },
    ) {
        Ok(scan) => scan,
        Err(error) => {
            let code = if !task.source.exists() {
                FailureCode::SourceMissing
            } else {
                FailureCode::ScanFailed
            };
            return JobOutcome::Failed(JobFailure {
                code,
                detail: error.text(),
            });
        }
    };

    let events_scan = events.clone();
    let repaint_scan = repaint.clone();
    let scan_id = task_id.clone();
    let dst = match sync::scan_tree(
        &task.target,
        &excludes,
        config.skip_hidden_system,
        move |files| {
            let _ = events_scan.send(Event::ScanProgress {
                task_id: scan_id.clone(),
                files,
            });
            (repaint_scan)();
        },
    ) {
        Ok(scan) => scan,
        Err(error) => {
            let code = if !task.target.exists() {
                FailureCode::TargetMissing
            } else {
                FailureCode::ScanFailed
            };
            return JobOutcome::Failed(JobFailure {
                code,
                detail: error.text(),
            });
        }
    };

    let plan = sync::build_plan(
        task.mode,
        config.policy,
        &task.source,
        &task.target,
        &src,
        &dst,
        &snapshot,
        config.verify,
    );

    let _ = events.send(Event::Started {
        task_id: task_id.clone(),
        total: plan.actions.len(),
    });
    (repaint)();

    let outcome = sync::execute_plan(
        &plan,
        &task.source,
        &task.target,
        &mut snapshot,
        &ExecOptions {
            verify: config.verify,
        },
        &cancel,
        {
            let events = events.clone();
            let repaint = repaint.clone();
            let task_id = task_id.clone();
            let mut last_emit = Instant::now() - Duration::from_secs(1);
            move |done, total, current, bytes| {
                let now = Instant::now();
                if now.duration_since(last_emit) >= Duration::from_millis(60) || done == total {
                    last_emit = now;
                    let _ = events.send(Event::Progress {
                        task_id: task_id.clone(),
                        done,
                        total,
                        current: current.to_string(),
                        bytes,
                    });
                    (repaint)();
                }
            }
        },
    );

    // For one-way and mirror the source is the source of truth, so the next
    // run does not need any history.
    if task.mode != SyncMode::TwoWay {
        snapshot.entries = src.entries.clone();
    } else {
        // Files that were already identical on both sides must be recorded,
        // otherwise they would look "changed on both sides" next time.
        for (rel, meta) in &src.entries {
            if let Some(target_meta) = dst.entries.get(rel) {
                if sync::same_meta(Some(meta), Some(target_meta)) {
                    snapshot.entries.insert(rel.clone(), *meta);
                }
            }
        }
        let missing: Vec<String> = snapshot
            .entries
            .keys()
            .filter(|rel| !src.entries.contains_key(*rel) && !dst.entries.contains_key(*rel))
            .cloned()
            .collect();
        for rel in missing {
            snapshot.entries.remove(&rel);
        }
    }

    let summary = SyncSummary {
        mode: task.mode,
        started_at,
        finished_at: util::now_secs(),
        copied: outcome.copied,
        updated: outcome.updated,
        deleted: outcome.deleted,
        bytes: outcome.bytes,
        conflicts: plan.conflicts.clone(),
        errors: outcome.errors.clone(),
        cancelled: outcome.cancelled,
    };

    JobOutcome::Synced { summary, snapshot }
}

fn run_preview_job(task: SyncTask, config: WorkerConfig) -> JobOutcome {
    if !task.source.exists() {
        return JobOutcome::Preview(Err(JobFailure {
            code: FailureCode::SourceMissing,
            detail: task.source.display().to_string(),
        }));
    }
    let excludes = match sync::build_excludes(&task.all_excludes()) {
        Ok(set) => set,
        Err(error) => {
            return JobOutcome::Preview(Err(JobFailure {
                code: FailureCode::Other,
                detail: error,
            }))
        }
    };
    let snapshot = Snapshot::load(&task.id);

    let src = match sync::scan_tree(&task.source, &excludes, config.skip_hidden_system, |_| {}) {
        Ok(scan) => scan,
        Err(error) => {
            return JobOutcome::Preview(Err(JobFailure {
                code: FailureCode::ScanFailed,
                detail: error.text(),
            }))
        }
    };
    // A missing target simply means "everything is new".
    let dst = if task.target.exists() {
        match sync::scan_tree(&task.target, &excludes, config.skip_hidden_system, |_| {}) {
            Ok(scan) => scan,
            Err(error) => {
                return JobOutcome::Preview(Err(JobFailure {
                    code: FailureCode::ScanFailed,
                    detail: error.text(),
                }))
            }
        }
    } else {
        ScanResult::default()
    };

    let plan: Plan = sync::build_plan(
        task.mode,
        config.policy,
        &task.source,
        &task.target,
        &src,
        &dst,
        &snapshot,
        config.verify,
    );

    let preview = PreviewData {
        new_files: plan.new_files,
        updated_files: plan.updated_files,
        deleted_files: plan.deleted_files,
        conflicts: plan.conflicts.len() as u64,
        bytes: plan.bytes,
        total_actions: plan.actions.len(),
        sample: plan.sample(80),
        source_files: plan.source_files,
        target_files: plan.target_files,
        source_bytes: plan.source_bytes,
        target_bytes: plan.target_bytes,
    };
    JobOutcome::Preview(Ok(preview))
}
