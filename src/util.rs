//! Small shared helpers: logging, formatting and Windows file utilities.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::TimeZone;

use crate::i18n::{fill, Key, Lang};

/// Append a line to the log file. Logging must never panic or block the app.
pub fn log(message: &str) {
    let line = format!(
        "[{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        message
    );
    let path = crate::config::log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Keep the log file from growing forever.
pub fn rotate_log_if_needed() {
    let path = crate::config::log_path();
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > 2 * 1024 * 1024 {
            let old = path.with_extension("log.old");
            let _ = fs::remove_file(&old);
            let _ = fs::rename(&path, old);
        }
    }
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn system_time_to_ns(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().min(i64::MAX as u128) as i64,
        Err(error) => -(error.duration().as_nanos().min(i64::MAX as u128) as i64),
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

pub fn format_duration(seconds: u64, lang: Lang) -> String {
    if seconds < 60 {
        fill(Key::SecondsShort.tr(lang), &[("{n}", &seconds.to_string())])
    } else if seconds < 3600 {
        let minutes = seconds / 60;
        let rest = seconds % 60;
        let head = fill(Key::MinutesShort.tr(lang), &[("{n}", &minutes.to_string())]);
        if rest == 0 {
            head
        } else {
            format!(
                "{head} {}",
                fill(Key::SecondsShort.tr(lang), &[("{n}", &rest.to_string())])
            )
        }
    } else if seconds < 86_400 {
        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let head = fill(Key::HoursShort.tr(lang), &[("{n}", &hours.to_string())]);
        if minutes == 0 {
            head
        } else {
            format!(
                "{head} {}",
                fill(Key::MinutesShort.tr(lang), &[("{n}", &minutes.to_string())])
            )
        }
    } else {
        let days = seconds / 86_400;
        fill(Key::DaysShort.tr(lang), &[("{n}", &days.to_string())])
    }
}

/// "2 min ago", "Yesterday", or an absolute date for older entries.
pub fn format_relative_time(timestamp: i64, lang: Lang) -> String {
    let now = now_secs();
    let diff = now.saturating_sub(timestamp);
    if diff < 0 {
        return absolute_time(timestamp);
    }
    match diff {
        0..=59 => Key::JustNow.tr(lang).to_string(),
        60..=3599 => fill(
            Key::MinutesAgo.tr(lang),
            &[("{n}", &(diff / 60).to_string())],
        ),
        3600..=86_399 => fill(
            Key::HoursAgo.tr(lang),
            &[("{n}", &(diff / 3600).to_string())],
        ),
        86_400..=172_799 => Key::Yesterday.tr(lang).to_string(),
        172_800..=604_799 => fill(
            Key::DaysAgo.tr(lang),
            &[("{n}", &(diff / 86_400).to_string())],
        ),
        _ => absolute_time(timestamp),
    }
}

pub fn absolute_time(timestamp: i64) -> String {
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// Keep the beginning and end of a long path so both drive and file name stay visible.
pub fn shorten_path(path: &str, max_chars: usize) -> String {
    let count = path.chars().count();
    if count <= max_chars || max_chars < 12 {
        return path.to_string();
    }
    let keep = max_chars - 3;
    let head = (keep + 1) / 2;
    let tail = keep - head;
    let start: String = path.chars().take(head).collect();
    let end: String = path.chars().skip(count - tail).collect();
    format!("{start}...{end}")
}

pub fn open_path(path: &Path) -> Result<(), String> {
    // Explorer handles both folders and files and needs no extra privileges.
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Open a folder and select a specific file inside it.
pub fn reveal_path(path: &Path) -> Result<(), String> {
    let mut command = std::process::Command::new("explorer");
    if path.is_file() {
        command.arg(format!("/select,{}", path.display()));
    } else {
        command.arg(path);
    }
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn hash_file(path: &Path) -> std::io::Result<blake3::Hash> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

/// True when a file has the Windows hidden or system attribute.
#[cfg(windows)]
pub fn is_hidden_or_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    fs::symlink_metadata(path)
        .map(|meta| {
            let attributes = meta.file_attributes();
            attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_hidden_or_system(path: &Path) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

/// Remove the read-only flag so a file can be replaced or deleted.
pub fn make_writable(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut permissions = meta.permissions();
        if permissions.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

pub fn unique_conflict_name(rel_path: &str, when: &chrono::DateTime<chrono::Local>) -> String {
    let stamp = when.format("%Y-%m-%d %H%M%S").to_string();
    let path = PathBuf::from(rel_path);
    let parent = path.parent().map(|p| p.to_string_lossy().replace('\\', "/"));
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let (stem, extension) = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), format!(".{ext}"))
        }
        _ => (file_name, String::new()),
    };
    let name = format!("{stem} (conflict {stamp}){extension}");
    match parent {
        Some(parent) if !parent.is_empty() && parent != "." => format!("{parent}/{name}"),
        _ => name,
    }
}

pub fn normalize_rel(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
