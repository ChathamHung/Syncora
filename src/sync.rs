//! The synchronization core.
//!
//! This module is deliberately UI-free so it can be unit tested and reasoned
//! about on its own. It provides:
//!
//! * `scan_tree` - a fast recursive snapshot of one folder.
//! * `build_plan` - a three-way diff that turns two scans plus the last synced
//!   state into a list of file operations.
//! * `execute_plan` - applies the operations with atomic copies and updates the
//!   snapshot so the next run stays incremental.
//!
//! All paths inside a snapshot are relative and use `/` as separator.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::config::SyncMode;
use crate::util;

/// Two seconds of slack absorbs FAT/exFAT timestamp rounding.
pub const MTIME_TOLERANCE_NS: i64 = 2_000_000_000;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMeta {
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ns: i64,
}

impl FileMeta {
    pub fn dir() -> Self {
        Self {
            is_dir: true,
            size: 0,
            mtime_ns: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub entries: BTreeMap<String, FileMeta>,
}

impl Snapshot {
    pub fn load(task_id: &str) -> Snapshot {
        crate::config::read_json(&crate::config::snapshot_path(task_id)).unwrap_or_default()
    }

    pub fn save(&self, task_id: &str) -> Result<(), String> {
        crate::config::write_json(&crate::config::snapshot_path(task_id), self)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanResult {
    pub entries: BTreeMap<String, FileMeta>,
    pub files: u64,
    pub dirs: u64,
    pub total_bytes: u64,
    pub skipped: u64,
}

#[derive(Clone, Debug)]
pub struct ScanError {
    pub path: PathBuf,
    pub message: String,
}

impl ScanError {
    pub fn text(&self) -> String {
        format!("{}: {}", self.path.display(), self.message)
    }
}

/// Compile user exclude patterns. Invalid patterns are reported once.
pub fn build_excludes(patterns: &[String]) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(false)
            .backslash_escape(false)
            .build()
            .map_err(|e| format!("{pattern}: {e}"))?;
        builder.add(glob);
    }
    builder.build().map_err(|e| e.to_string())
}

/// Recursively scan a folder into a relative-path map.
pub fn scan_tree(
    root: &Path,
    excludes: &GlobSet,
    skip_hidden_system: bool,
    mut on_progress: impl FnMut(u64),
) -> Result<ScanResult, ScanError> {
    if !root.exists() {
        return Err(ScanError {
            path: root.to_path_buf(),
            message: "folder does not exist".to_string(),
        });
    }
    if !root.is_dir() {
        return Err(ScanError {
            path: root.to_path_buf(),
            message: "not a folder".to_string(),
        });
    }

    let mut result = ScanResult::default();
    let mut last_report = 0u64;

    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let rel = entry
                .path()
                .strip_prefix(root)
                .map(util::normalize_rel)
                .unwrap_or_default();
            if rel.is_empty() {
                return true;
            }
            if excludes.is_match(&rel) {
                return false;
            }
            if entry.file_type().is_dir() && excludes.is_match(format!("{rel}/")) {
                return false;
            }
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".syncora-tmp-")
            {
                return false;
            }
            if skip_hidden_system && util::is_hidden_or_system(entry.path()) {
                return false;
            }
            true
        });

    for item in walker {
        let entry = match item {
            Ok(entry) => entry,
            Err(error) => {
                // Permission problems deep inside a tree should not abort the
                // whole scan; count them and keep going.
                if error.depth() == 0 {
                    return Err(ScanError {
                        path: root.to_path_buf(),
                        message: error.to_string(),
                    });
                }
                result.skipped += 1;
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            result.skipped += 1;
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(root)
            .map(util::normalize_rel)
            .unwrap_or_default();
        if rel.is_empty() {
            continue;
        }

        if file_type.is_dir() {
            result.entries.insert(rel, FileMeta::dir());
            result.dirs += 1;
        } else if file_type.is_file() {
            match entry.metadata() {
                Ok(meta) => {
                    let mtime_ns = meta
                        .modified()
                        .map(util::system_time_to_ns)
                        .unwrap_or(0);
                    result.entries.insert(
                        rel,
                        FileMeta {
                            is_dir: false,
                            size: meta.len(),
                            mtime_ns,
                        },
                    );
                    result.files += 1;
                    result.total_bytes = result.total_bytes.saturating_add(meta.len());
                }
                Err(_) => {
                    result.skipped += 1;
                }
            }
        }

        let seen = result.files + result.dirs;
        if seen.saturating_sub(last_report) >= 500 {
            last_report = seen;
            on_progress(seen);
        }
    }

    on_progress(result.files + result.dirs);
    Ok(result)
}

/// Compare two snapshot entries, treating directories as "exists / not exists".
pub fn same_meta(a: Option<&FileMeta>, b: Option<&FileMeta>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.is_dir != b.is_dir {
                return false;
            }
            if a.is_dir {
                return true;
            }
            a.size == b.size && (a.mtime_ns - b.mtime_ns).abs() <= MTIME_TOLERANCE_NS
        }
        _ => false,
    }
}

/// Byte-for-byte comparison used to avoid false conflicts.
///
/// Callers use this only for files that look like candidates for a conflict, so
/// always hashing here is acceptable and avoids trusting timestamps that are
/// too close together to distinguish two edits.
pub fn contents_equal(a: &Path, b: &Path, meta_a: &FileMeta, meta_b: &FileMeta) -> bool {
    if meta_a.size != meta_b.size {
        return false;
    }
    match (util::hash_file(a), util::hash_file(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Source,
    Target,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Source => Side::Target,
            Side::Target => Side::Source,
        }
    }

    pub fn root<'a>(self, source: &'a Path, target: &'a Path) -> &'a Path {
        match self {
            Side::Source => source,
            Side::Target => target,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    CopyFile {
        rel: String,
        from: Side,
        existed: bool,
        size: u64,
    },
    RenameFile {
        rel: String,
        to_rel: String,
        side: Side,
    },
    DeleteFile {
        rel: String,
        side: Side,
    },
    CreateDir {
        rel: String,
        side: Side,
    },
    DeleteDir {
        rel: String,
        side: Side,
    },
    /// Align timestamps for files whose content is already identical.
    /// Without this, two-way sync could ping-pong a file forever.
    SetMtime {
        rel: String,
        side: Side,
        mtime_ns: i64,
    },
}

impl Action {
    pub fn rel(&self) -> &str {
        match self {
            Action::CopyFile { rel, .. }
            | Action::RenameFile { rel, .. }
            | Action::DeleteFile { rel, .. }
            | Action::CreateDir { rel, .. }
            | Action::DeleteDir { rel, .. }
            | Action::SetMtime { rel, .. } => rel,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConflictInfo {
    pub rel: String,
    pub source: Option<FileMeta>,
    pub target: Option<FileMeta>,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub conflicts: Vec<ConflictInfo>,
    pub bytes: u64,
    pub new_files: u64,
    pub updated_files: u64,
    pub deleted_files: u64,
    pub source_files: u64,
    pub target_files: u64,
    pub source_bytes: u64,
    pub target_bytes: u64,
}

impl Plan {
    /// A short sample of paths for the preview dialog.
    pub fn sample(&self, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        for action in &self.actions {
            out.push(action.rel().to_string());
            if out.len() >= limit {
                break;
            }
        }
        for conflict in &self.conflicts {
            out.push(conflict.rel.clone());
            if out.len() >= limit {
                break;
            }
        }
        out
    }
}

/// Build the operation list for one task.
pub fn build_plan(
    mode: SyncMode,
    policy: crate::config::ConflictPolicy,
    source_root: &Path,
    target_root: &Path,
    src: &ScanResult,
    dst: &ScanResult,
    base: &Snapshot,
    verify: bool,
) -> Plan {
    let _ = verify;
    let mut plan = Plan {
        source_files: src.files,
        target_files: dst.files,
        source_bytes: src.total_bytes,
        target_bytes: dst.total_bytes,
        ..Plan::default()
    };

    match mode {
        SyncMode::OneWay => plan_one_way(&mut plan, src, dst),
        SyncMode::Mirror => plan_mirror(&mut plan, src, dst),
        SyncMode::TwoWay => plan_two_way(
            &mut plan,
            policy,
            source_root,
            target_root,
            src,
            dst,
            base,
        ),
    }

    // Deterministic, safe execution order.
    let mut creates = Vec::new();
    let mut renames = Vec::new();
    let mut copies = Vec::new();
    let mut deletes = Vec::new();
    let mut dir_deletes = Vec::new();
    for action in plan.actions.drain(..) {
        match action {
            Action::CreateDir { .. } => creates.push(action),
            Action::RenameFile { .. } => renames.push(action),
            Action::CopyFile { .. } | Action::SetMtime { .. } => copies.push(action),
            Action::DeleteFile { .. } => deletes.push(action),
            Action::DeleteDir { .. } => dir_deletes.push(action),
        }
    }
    creates.sort_by_key(|action| action.rel().matches('/').count());
    dir_deletes.sort_by_key(|action| std::cmp::Reverse(action.rel().matches('/').count()));
    plan.actions.extend(creates);
    plan.actions.extend(renames);
    plan.actions.extend(copies);
    plan.actions.extend(deletes);
    plan.actions.extend(dir_deletes);
    plan
}

fn plan_one_way(plan: &mut Plan, src: &ScanResult, dst: &ScanResult) {
    for (rel, meta) in &src.entries {
        if meta.is_dir {
            if !dst.entries.contains_key(rel) {
                plan.actions.push(Action::CreateDir {
                    rel: rel.clone(),
                    side: Side::Target,
                });
            }
        } else {
            match dst.entries.get(rel) {
                None => {
                    plan.new_files += 1;
                    plan.bytes = plan.bytes.saturating_add(meta.size);
                    plan.actions.push(Action::CopyFile {
                        rel: rel.clone(),
                        from: Side::Source,
                        existed: false,
                        size: meta.size,
                    });
                }
                Some(existing) if !same_meta(Some(meta), Some(existing)) => {
                    plan.updated_files += 1;
                    plan.bytes = plan.bytes.saturating_add(meta.size);
                    plan.actions.push(Action::CopyFile {
                        rel: rel.clone(),
                        from: Side::Source,
                        existed: true,
                        size: meta.size,
                    });
                }
                _ => {}
            }
        }
    }
}

fn plan_mirror(plan: &mut Plan, src: &ScanResult, dst: &ScanResult) {
    plan_one_way(plan, src, dst);

    // Remove target files that no longer exist in the source.
    for (rel, meta) in &dst.entries {
        if !meta.is_dir && !src.entries.contains_key(rel) {
            plan.deleted_files += 1;
            plan.actions.push(Action::DeleteFile {
                rel: rel.clone(),
                side: Side::Target,
            });
        }
    }
    for (rel, meta) in &dst.entries {
        if meta.is_dir && !src.entries.contains_key(rel) {
            plan.actions.push(Action::DeleteDir {
                rel: rel.clone(),
                side: Side::Target,
            });
        }
    }
}

fn plan_two_way(
    plan: &mut Plan,
    policy: crate::config::ConflictPolicy,
    source_root: &Path,
    target_root: &Path,
    src: &ScanResult,
    dst: &ScanResult,
    base: &Snapshot,
) {
    use crate::config::ConflictPolicy;

    // ---- files -------------------------------------------------------------
    let mut paths: BTreeSet<&String> = BTreeSet::new();
    paths.extend(src.entries.keys());
    paths.extend(dst.entries.keys());
    paths.extend(base.entries.keys());

    for rel in paths {
        let s = src.entries.get(rel).filter(|m| !m.is_dir);
        let t = dst.entries.get(rel).filter(|m| !m.is_dir);
        let b = base.entries.get(rel).filter(|m| !m.is_dir);

        let s_changed = !same_meta(s, b);
        let t_changed = !same_meta(t, b);

        match (s_changed, t_changed) {
            (false, false) => {}
            (true, false) => {
                if let Some(meta) = s {
                    plan.actions.push(Action::CopyFile {
                        rel: rel.clone(),
                        from: Side::Source,
                        existed: t.is_some(),
                        size: meta.size,
                    });
                    plan.bytes = plan.bytes.saturating_add(meta.size);
                    if t.is_some() {
                        plan.updated_files += 1;
                    } else {
                        plan.new_files += 1;
                    }
                } else if t.is_some() {
                    plan.deleted_files += 1;
                    plan.actions.push(Action::DeleteFile {
                        rel: rel.clone(),
                        side: Side::Target,
                    });
                }
            }
            (false, true) => {
                if let Some(meta) = t {
                    plan.actions.push(Action::CopyFile {
                        rel: rel.clone(),
                        from: Side::Target,
                        existed: s.is_some(),
                        size: meta.size,
                    });
                    plan.bytes = plan.bytes.saturating_add(meta.size);
                    if s.is_some() {
                        plan.updated_files += 1;
                    } else {
                        plan.new_files += 1;
                    }
                } else if s.is_some() {
                    plan.deleted_files += 1;
                    plan.actions.push(Action::DeleteFile {
                        rel: rel.clone(),
                        side: Side::Source,
                    });
                }
            }
            (true, true) => {
                let source_path = source_root.join(rel);
                let target_path = target_root.join(rel);

                if let (Some(sm), Some(tm)) = (s, t) {
                    // Same size and exact timestamp, with a known base, means
                    // both sides were untouched since the last sync. Avoid a
                    // needless hash in the common case.
                    if b.is_some() && sm.mtime_ns == tm.mtime_ns && sm.size == tm.size {
                        continue;
                    }
                    // Same bytes but different timestamps: align them instead
                    // of copying (and instead of reporting a conflict).
                    if contents_equal(&source_path, &target_path, sm, tm) {
                        if !same_meta(Some(sm), Some(tm)) {
                            plan.actions.push(Action::SetMtime {
                                rel: rel.clone(),
                                side: Side::Target,
                                mtime_ns: sm.mtime_ns,
                            });
                        }
                        continue;
                    }
                }

                match (s, t) {
                    (Some(sm), Some(tm)) => {
                        if policy == ConflictPolicy::Ask || policy == ConflictPolicy::Skip {
                            plan.conflicts.push(ConflictInfo {
                                rel: rel.clone(),
                                source: Some(*sm),
                                target: Some(*tm),
                            });
                            continue;
                        }
                        if policy == ConflictPolicy::PreferSource {
                            plan.actions.push(Action::CopyFile {
                                rel: rel.clone(),
                                from: Side::Source,
                                existed: true,
                                size: sm.size,
                            });
                            plan.bytes = plan.bytes.saturating_add(sm.size);
                            plan.updated_files += 1;
                        } else if policy == ConflictPolicy::PreferTarget {
                            plan.actions.push(Action::CopyFile {
                                rel: rel.clone(),
                                from: Side::Target,
                                existed: true,
                                size: tm.size,
                            });
                            plan.bytes = plan.bytes.saturating_add(tm.size);
                            plan.updated_files += 1;
                        } else {
                            // KeepBoth: the newer file keeps the original name.
                            let source_wins = sm.mtime_ns >= tm.mtime_ns;
                            add_keep_both_actions(
                                plan,
                                rel,
                                sm,
                                tm,
                                source_wins,
                                src,
                                dst,
                            );
                            plan.conflicts.push(ConflictInfo {
                                rel: rel.clone(),
                                source: Some(*sm),
                                target: Some(*tm),
                            });
                        }
                    }
                    (Some(sm), None) => {
                        // Source modified, target deleted.
                        if policy == ConflictPolicy::Ask || policy == ConflictPolicy::Skip {
                            plan.conflicts.push(ConflictInfo {
                                rel: rel.clone(),
                                source: Some(*sm),
                                target: None,
                            });
                            continue;
                        }
                        if policy == ConflictPolicy::PreferTarget {
                            plan.deleted_files += 1;
                            plan.actions.push(Action::DeleteFile {
                                rel: rel.clone(),
                                side: Side::Source,
                            });
                        } else {
                            plan.actions.push(Action::CopyFile {
                                rel: rel.clone(),
                                from: Side::Source,
                                existed: false,
                                size: sm.size,
                            });
                            plan.bytes = plan.bytes.saturating_add(sm.size);
                            plan.new_files += 1;
                        }
                    }
                    (None, Some(tm)) => {
                        if policy == ConflictPolicy::Ask || policy == ConflictPolicy::Skip {
                            plan.conflicts.push(ConflictInfo {
                                rel: rel.clone(),
                                source: None,
                                target: Some(*tm),
                            });
                            continue;
                        }
                        if policy == ConflictPolicy::PreferSource {
                            plan.deleted_files += 1;
                            plan.actions.push(Action::DeleteFile {
                                rel: rel.clone(),
                                side: Side::Target,
                            });
                        } else {
                            plan.actions.push(Action::CopyFile {
                                rel: rel.clone(),
                                from: Side::Target,
                                existed: false,
                                size: tm.size,
                            });
                            plan.bytes = plan.bytes.saturating_add(tm.size);
                            plan.new_files += 1;
                        }
                    }
                    (None, None) => {}
                }
            }
        }
    }

    // ---- directories -------------------------------------------------------
    let mut dirs: BTreeSet<&String> = BTreeSet::new();
    for (rel, meta) in &src.entries {
        if meta.is_dir {
            dirs.insert(rel);
        }
    }
    for (rel, meta) in &dst.entries {
        if meta.is_dir {
            dirs.insert(rel);
        }
    }
    for (rel, meta) in &base.entries {
        if meta.is_dir {
            dirs.insert(rel);
        }
    }

    for rel in dirs {
        let s = src.entries.get(rel).map(|m| m.is_dir).unwrap_or(false);
        let t = dst.entries.get(rel).map(|m| m.is_dir).unwrap_or(false);
        let b = base.entries.get(rel).map(|m| m.is_dir).unwrap_or(false);

        let s_changed = s != b;
        let t_changed = t != b;

        match (s, t) {
            (true, false) => {
                if b && !s_changed && t_changed {
                    // The target deleted this directory; mirror the deletion.
                    plan.actions.push(Action::DeleteDir {
                        rel: rel.clone(),
                        side: Side::Source,
                    });
                } else {
                    plan.actions.push(Action::CreateDir {
                        rel: rel.clone(),
                        side: Side::Target,
                    });
                }
            }
            (false, true) => {
                if b && s_changed && !t_changed {
                    plan.actions.push(Action::DeleteDir {
                        rel: rel.clone(),
                        side: Side::Target,
                    });
                } else {
                    plan.actions.push(Action::CreateDir {
                        rel: rel.clone(),
                        side: Side::Source,
                    });
                }
            }
            _ => {}
        }
    }
}

fn add_keep_both_actions(
    plan: &mut Plan,
    rel: &str,
    source_meta: &FileMeta,
    target_meta: &FileMeta,
    source_wins: bool,
    src: &ScanResult,
    dst: &ScanResult,
) {
    let conflict_rel = unique_conflict_rel(rel, src, dst);
    let (winner_side, loser_side, winner_meta) = if source_wins {
        (Side::Source, Side::Target, source_meta)
    } else {
        (Side::Target, Side::Source, target_meta)
    };

    // Rename the losing version out of the way.
    plan.actions.push(Action::RenameFile {
        rel: rel.to_string(),
        to_rel: conflict_rel.clone(),
        side: loser_side,
    });
    // Put the winning version in the original place.
    plan.actions.push(Action::CopyFile {
        rel: rel.to_string(),
        from: winner_side,
        existed: true,
        size: winner_meta.size,
    });
    // Keep the losing version on both sides as well.
    plan.actions.push(Action::CopyFile {
        rel: conflict_rel,
        from: loser_side,
        existed: false,
        size: if source_wins {
            target_meta.size
        } else {
            source_meta.size
        },
    });
    plan.bytes = plan
        .bytes
        .saturating_add(source_meta.size)
        .saturating_add(target_meta.size);
    plan.updated_files += 1;
}

fn unique_conflict_rel(rel: &str, src: &ScanResult, dst: &ScanResult) -> String {
    let now = chrono::Local::now();
    let base = util::unique_conflict_name(rel, &now);
    if !src.entries.contains_key(&base) && !dst.entries.contains_key(&base) {
        return base;
    }
    let path = PathBuf::from(&base);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let extension = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    for index in 2..1000 {
        let name = format!("{stem} ({index}){extension}");
        let candidate = if parent.is_empty() {
            name
        } else {
            format!("{parent}/{name}")
        };
        if !src.entries.contains_key(&candidate) && !dst.entries.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{stem}-{}{extension}", util::now_secs())
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ExecOptions {
    pub verify: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ExecOutcome {
    pub copied: u64,
    pub updated: u64,
    pub deleted: u64,
    pub dirs_created: u64,
    pub dirs_deleted: u64,
    pub bytes: u64,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

/// Apply a plan and keep the snapshot in sync with what actually happened.
pub fn execute_plan(
    plan: &Plan,
    source_root: &Path,
    target_root: &Path,
    snapshot: &mut Snapshot,
    options: &ExecOptions,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(usize, usize, &str, u64),
) -> ExecOutcome {
    let mut outcome = ExecOutcome::default();
    let total = plan.actions.len();

    for (index, action) in plan.actions.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            break;
        }

        match action {
            Action::CreateDir { rel, side } => {
                let path = side.root(source_root, target_root).join(rel);
                match fs::create_dir_all(&path) {
                    Ok(()) => {
                        outcome.dirs_created += 1;
                        snapshot.entries.insert(rel.clone(), FileMeta::dir());
                    }
                    Err(error) => push_error(&mut outcome, rel, &error),
                }
            }
            Action::DeleteDir { rel, side } => {
                let path = side.root(source_root, target_root).join(rel);
                match fs::remove_dir(&path) {
                    Ok(()) => {
                        outcome.dirs_deleted += 1;
                        snapshot.entries.remove(rel);
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        snapshot.entries.remove(rel);
                    }
                    Err(error) => {
                        // Not empty simply means something inside is still in
                        // use or conflicted; that is not a hard failure.
                        if error.kind() != io::ErrorKind::DirectoryNotEmpty {
                            push_error(&mut outcome, rel, &error);
                        }
                    }
                }
            }
            Action::DeleteFile { rel, side } => {
                let path = side.root(source_root, target_root).join(rel);
                match remove_file_with_retry(&path) {
                    Ok(()) => {
                        outcome.deleted += 1;
                        snapshot.entries.remove(rel);
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        snapshot.entries.remove(rel);
                    }
                    Err(error) => push_error(&mut outcome, rel, &error),
                }
            }
            Action::RenameFile { rel, to_rel, side } => {
                let root = side.root(source_root, target_root);
                let from = root.join(rel);
                let to = root.join(to_rel);
                if let Some(parent) = to.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                match fs::rename(&from, &to) {
                    Ok(()) => {
                        if let Ok(meta) = fs::metadata(&to) {
                            let file_meta = FileMeta {
                                is_dir: false,
                                size: meta.len(),
                                mtime_ns: meta.modified().map(util::system_time_to_ns).unwrap_or(0),
                            };
                            snapshot.entries.remove(rel);
                            snapshot.entries.insert(to_rel.clone(), file_meta);
                        }
                    }
                    Err(error) => push_error(&mut outcome, rel, &error),
                }
            }
            Action::CopyFile {
                rel,
                from,
                existed,
                size,
            } => {
                let src = from.root(source_root, target_root).join(rel);
                let dst = from.other().root(source_root, target_root).join(rel);
                match copy_file_preserve(&src, &dst, options.verify) {
                    Ok(meta) => {
                        if *existed {
                            outcome.updated += 1;
                        } else {
                            outcome.copied += 1;
                        }
                        outcome.bytes = outcome.bytes.saturating_add(*size);
                        snapshot.entries.insert(rel.clone(), meta);
                    }
                    Err(error) => push_error(&mut outcome, rel, &error),
                }
            }
            Action::SetMtime {
                rel,
                side,
                mtime_ns,
            } => {
                let path = side.root(source_root, target_root).join(rel);
                let seconds = mtime_ns.div_euclid(1_000_000_000);
                let nanos = mtime_ns.rem_euclid(1_000_000_000) as u32;
                match filetime::set_file_mtime(
                    &path,
                    filetime::FileTime::from_unix_time(seconds, nanos),
                ) {
                    Ok(()) => {
                        if let Ok(meta) = fs::metadata(&path) {
                            snapshot.entries.insert(rel.clone(), meta_to_file_meta(&meta));
                        }
                    }
                    Err(error) => push_error(&mut outcome, rel, &error),
                }
            }
        }

        on_progress(index + 1, total, action.rel(), outcome.bytes);
    }

    outcome
}

fn push_error(outcome: &mut ExecOutcome, rel: &str, error: &io::Error) {
    if outcome.errors.len() < 25 {
        outcome.errors.push(format!("{rel}: {error}"));
    }
}

fn remove_file_with_retry(path: &Path) -> io::Result<()> {
    util::make_writable(path);
    let mut last_error = None;
    for attempt in 0..3 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(120));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("unknown error")))
}

/// Copy a file to a temporary sibling, preserve its timestamp, then rename it
/// into place. This avoids readers ever seeing a half-written file.
pub fn copy_file_preserve(src: &Path, dst: &Path, verify: bool) -> io::Result<FileMeta> {
    let source_meta = fs::metadata(src)?;
    let file_meta = FileMeta {
        is_dir: false,
        size: source_meta.len(),
        mtime_ns: source_meta
            .modified()
            .map(util::system_time_to_ns)
            .unwrap_or(0),
    };

    let parent = dst.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination has no parent")
    })?;
    fs::create_dir_all(parent)?;

    let temp_name = format!(
        ".syncora-tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let temp = parent.join(temp_name);
    let _ = fs::remove_file(&temp);

    let mut last_error = None;
    for attempt in 0..3 {
        match fs::copy(src, &temp) {
            Ok(_) => {
                last_error = None;
                break;
            }
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(120));
                }
            }
        }
    }
    if let Some(error) = last_error {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    // Preserve the modification time so future scans can detect "unchanged".
    let seconds = file_meta.mtime_ns.div_euclid(1_000_000_000);
    let nanos = file_meta.mtime_ns.rem_euclid(1_000_000_000) as u32;
    let _ = filetime::set_file_mtime(&temp, filetime::FileTime::from_unix_time(seconds, nanos));
    util::make_writable(&temp);

    if verify {
        let source_hash = util::hash_file(src)?;
        let temp_hash = util::hash_file(&temp)?;
        if source_hash != temp_hash {
            let _ = fs::remove_file(&temp);
            return Err(io::Error::other("checksum mismatch after copy"));
        }
    }

    util::make_writable(dst);
    if let Err(error) = fs::rename(&temp, dst) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(file_meta)
}

// ---------------------------------------------------------------------------
// Manual conflict resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    KeepSource,
    KeepTarget,
    KeepBoth,
}

/// Apply a user's choice for one conflicted file.
///
/// The caller is responsible for saving the snapshot afterwards.
pub fn apply_resolution(
    source_root: &Path,
    target_root: &Path,
    rel: &str,
    choice: Resolution,
    snapshot: &mut Snapshot,
    verify: bool,
) -> Result<(), String> {
    let source_path = source_root.join(rel);
    let target_path = target_root.join(rel);
    let source_meta = fs::metadata(&source_path).ok().filter(|m| m.is_file());
    let target_meta = fs::metadata(&target_path).ok().filter(|m| m.is_file());

    match choice {
        Resolution::KeepSource => match source_meta {
            Some(meta) => {
                copy_file_preserve(&source_path, &target_path, verify)
                    .map_err(|e| e.to_string())?;
                snapshot.entries.insert(rel.to_string(), meta_to_file_meta(&meta));
            }
            None => {
                if target_meta.is_some() {
                    remove_file_with_retry(&target_path).map_err(|e| e.to_string())?;
                }
                snapshot.entries.remove(rel);
            }
        },
        Resolution::KeepTarget => match target_meta {
            Some(meta) => {
                copy_file_preserve(&target_path, &source_path, verify)
                    .map_err(|e| e.to_string())?;
                snapshot.entries.insert(rel.to_string(), meta_to_file_meta(&meta));
            }
            None => {
                if source_meta.is_some() {
                    remove_file_with_retry(&source_path).map_err(|e| e.to_string())?;
                }
                snapshot.entries.remove(rel);
            }
        },
        Resolution::KeepBoth => match (source_meta, target_meta) {
            (Some(sm), Some(tm)) => {
                let source_wins = sm.modified().ok().map(util::system_time_to_ns).unwrap_or(0)
                    >= tm.modified().ok().map(util::system_time_to_ns).unwrap_or(0);
                let conflict_rel = unique_conflict_name_on_disk(source_root, target_root, rel);
                let (winner_path, loser_path, winner_meta) = if source_wins {
                    (&source_path, &target_path, &sm)
                } else {
                    (&target_path, &source_path, &tm)
                };
                let winner_side_root = if source_wins { source_root } else { target_root };
                let loser_side_root = if source_wins { target_root } else { source_root };
                let conflict_on_winner = winner_side_root.join(&conflict_rel);

                // Move the losing file aside, then put the winner in place.
                let conflict_on_loser = loser_side_root.join(&conflict_rel);
                if let Some(parent) = conflict_on_loser.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::rename(loser_path, &conflict_on_loser).map_err(|e| e.to_string())?;
                copy_file_preserve(winner_path, loser_path, verify).map_err(|e| e.to_string())?;
                copy_file_preserve(&conflict_on_loser, &conflict_on_winner, verify)
                    .map_err(|e| e.to_string())?;

                snapshot
                    .entries
                    .insert(rel.to_string(), meta_to_file_meta(winner_meta));
                if let Ok(meta) = fs::metadata(&conflict_on_winner) {
                    snapshot
                        .entries
                        .insert(conflict_rel, meta_to_file_meta(&meta));
                }
            }
            (Some(_), None) => {
                copy_file_preserve(&source_path, &target_path, verify)
                    .map_err(|e| e.to_string())?;
                if let Ok(meta) = fs::metadata(&target_path) {
                    snapshot.entries.insert(rel.to_string(), meta_to_file_meta(&meta));
                }
            }
            (None, Some(_)) => {
                copy_file_preserve(&target_path, &source_path, verify)
                    .map_err(|e| e.to_string())?;
                if let Ok(meta) = fs::metadata(&source_path) {
                    snapshot.entries.insert(rel.to_string(), meta_to_file_meta(&meta));
                }
            }
            (None, None) => {
                snapshot.entries.remove(rel);
            }
        },
    }
    Ok(())
}

fn meta_to_file_meta(meta: &fs::Metadata) -> FileMeta {
    FileMeta {
        is_dir: meta.is_dir(),
        size: meta.len(),
        mtime_ns: meta.modified().map(util::system_time_to_ns).unwrap_or(0),
    }
}

fn unique_conflict_name_on_disk(source_root: &Path, target_root: &Path, rel: &str) -> String {
    let base = util::unique_conflict_name(rel, &chrono::Local::now());
    let path = PathBuf::from(&base);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let extension = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    for index in 1..1000 {
        let candidate = if index == 1 {
            base.clone()
        } else {
            let name = format!("{stem} ({index}){extension}");
            if parent.is_empty() {
                name
            } else {
                format!("{parent}/{name}")
            }
        };
        if !source_root.join(&candidate).exists() && !target_root.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}{extension}", util::now_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct TempTree {
        path: PathBuf,
    }

    impl TempTree {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "syncora-test-{name}-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let path = self.path.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
            path
        }

        fn scan(&self) -> ScanResult {
            let excludes = build_excludes(&[]).unwrap();
            scan_tree(&self.path, &excludes, false, |_| {}).unwrap()
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_plan(
        mode: SyncMode,
        policy: crate::config::ConflictPolicy,
        source: &TempTree,
        target: &TempTree,
        base: &Snapshot,
    ) -> Plan {
        build_plan(
            mode,
            policy,
            &source.path,
            &target.path,
            &source.scan(),
            &target.scan(),
            base,
            false,
        )
    }

    fn run_plan(
        plan: &Plan,
        source: &TempTree,
        target: &TempTree,
        snapshot: &mut Snapshot,
    ) -> ExecOutcome {
        execute_plan(
            plan,
            &source.path,
            &target.path,
            snapshot,
            &ExecOptions { verify: false },
            &AtomicBool::new(false),
            |_, _, _, _| {},
        )
    }

    #[test]
    fn one_way_copies_new_and_changed_files() {
        let source = TempTree::new("one-way-src");
        let target = TempTree::new("one-way-dst");
        source.write("a.txt", "hello");
        source.write("sub/b.txt", "world");
        target.write("a.txt", "old");
        target.write("keep.txt", "target only");

        let mut snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::OneWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert_eq!(plan.new_files, 1);
        assert_eq!(plan.updated_files, 1);
        assert_eq!(plan.deleted_files, 0);

        let outcome = run_plan(&plan, &source, &target, &mut snapshot);
        assert_eq!(outcome.errors.len(), 0);
        assert_eq!(fs::read_to_string(target.path.join("a.txt")).unwrap(), "hello");
        assert_eq!(
            fs::read_to_string(target.path.join("sub/b.txt")).unwrap(),
            "world"
        );
        // One-way never deletes target extras.
        assert!(target.path.join("keep.txt").exists());

        let again = make_plan(
            SyncMode::OneWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert!(again.actions.is_empty(), "second run should be incremental");
    }

    #[test]
    fn mirror_removes_extra_files_and_folders() {
        let source = TempTree::new("mirror-src");
        let target = TempTree::new("mirror-dst");
        source.write("a.txt", "same");
        target.write("a.txt", "same");
        target.write("extra.txt", "remove me");
        target.write("old/nested.txt", "remove me too");

        let mut snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::Mirror,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert_eq!(plan.deleted_files, 2);
        let outcome = run_plan(&plan, &source, &target, &mut snapshot);
        assert_eq!(outcome.errors.len(), 0);
        assert!(!target.path.join("extra.txt").exists());
        assert!(!target.path.join("old").exists());
        assert!(target.path.join("a.txt").exists());
    }

    #[test]
    fn two_way_merges_both_sides() {
        let source = TempTree::new("two-way-src");
        let target = TempTree::new("two-way-dst");
        source.write("from-source.txt", "A");
        target.write("from-target.txt", "B");

        let mut snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert_eq!(plan.conflicts.len(), 0);
        assert_eq!(plan.new_files, 2);
        let outcome = run_plan(&plan, &source, &target, &mut snapshot);
        assert_eq!(outcome.errors.len(), 0);

        assert!(target.path.join("from-source.txt").exists());
        assert!(source.path.join("from-target.txt").exists());

        let again = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert!(again.actions.is_empty());
        assert!(again.conflicts.is_empty());
    }

    #[test]
    fn two_way_detects_conflicts() {
        let source = TempTree::new("conflict-src");
        let target = TempTree::new("conflict-dst");
        source.write("a.txt", "source version");
        target.write("a.txt", "target version");

        let snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert_eq!(plan.conflicts.len(), 1);
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn keep_both_creates_conflict_copy() {
        let source = TempTree::new("keepboth-src");
        let target = TempTree::new("keepboth-dst");
        source.write("a.txt", "source version");
        target.write("a.txt", "target version");

        let mut snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::KeepBoth,
            &source,
            &target,
            &snapshot,
        );
        // KeepBoth still records the conflict for the history/audit trail, but it
        // resolves it automatically by keeping both versions instead of asking.
        assert_eq!(plan.conflicts.len(), 1);
        assert!(!plan.actions.is_empty(), "keep-both must schedule copy actions");
        let outcome = run_plan(&plan, &source, &target, &mut snapshot);
        assert_eq!(outcome.errors.len(), 0);

        let source_files: Vec<String> = fs::read_dir(&source.path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let target_files: Vec<String> = fs::read_dir(&target.path)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(source_files.len(), 2, "source should have both versions");
        assert_eq!(target_files.len(), 2, "target should have both versions");
    }

    #[test]
    fn two_way_propagates_deletion() {
        let source = TempTree::new("delete-src");
        let target = TempTree::new("delete-dst");
        source.write("a.txt", "content");

        let mut snapshot = Snapshot::default();
        let plan = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        run_plan(&plan, &source, &target, &mut snapshot);
        assert!(target.path.join("a.txt").exists());

        fs::remove_file(source.path.join("a.txt")).unwrap();
        let plan = make_plan(
            SyncMode::TwoWay,
            crate::config::ConflictPolicy::Ask,
            &source,
            &target,
            &snapshot,
        );
        assert_eq!(plan.deleted_files, 1);
        run_plan(&plan, &source, &target, &mut snapshot);
        assert!(!target.path.join("a.txt").exists());
    }

    #[test]
    fn manual_resolution_copies_chosen_side() {
        let source = TempTree::new("resolve-src");
        let target = TempTree::new("resolve-dst");
        source.write("a.txt", "source version");
        target.write("a.txt", "target version");

        let mut snapshot = Snapshot::default();
        apply_resolution(
            &source.path,
            &target.path,
            "a.txt",
            Resolution::KeepSource,
            &mut snapshot,
            false,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(target.path.join("a.txt")).unwrap(),
            "source version"
        );
    }

    #[test]
    fn excludes_skip_matching_files() {
        let source = TempTree::new("exclude-src");
        let target = TempTree::new("exclude-dst");
        source.write("keep.txt", "keep");
        source.write("skip.tmp", "skip");
        source.write("cache/deep.txt", "skip");

        let excludes = build_excludes(&["*.tmp".to_string(), "cache/**".to_string()]).unwrap();
        let scan = scan_tree(&source.path, &excludes, false, |_| {}).unwrap();
        assert!(scan.entries.contains_key("keep.txt"));
        assert!(!scan.entries.contains_key("skip.tmp"));
        assert!(!scan.entries.contains_key("cache/deep.txt"));

        let plan = build_plan(
            SyncMode::OneWay,
            crate::config::ConflictPolicy::Ask,
            &source.path,
            &target.path,
            &scan,
            &target.scan(),
            &Snapshot::default(),
            false,
        );
        let mut snapshot = Snapshot::default();
        run_plan(&plan, &source, &target, &mut snapshot);
        assert!(target.path.join("keep.txt").exists());
        assert!(!target.path.join("skip.tmp").exists());
    }
}
