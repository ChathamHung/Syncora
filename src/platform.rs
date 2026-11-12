//! Windows integration that does not need administrator rights.
//!
//! * Per-user autostart through `HKCU\...\Run`
//! * Light/dark mode detection through the Personalize registry key
//! * Toast notifications with a per-user AppUserModelID
//! * A named mutex so only one copy of Syncora runs

use std::path::Path;

use tauri_winrt_notification::{Duration as ToastDuration, Sound, Toast};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

use crate::config::APP_NAME;

const APP_ID: &str = "Syncora.Desktop";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Syncora";

// ---------------------------------------------------------------------------
// Autostart
// ---------------------------------------------------------------------------

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY)
        .map_err(|e| format!("cannot open startup registry key: {e}"))?;

    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let command = format!("\"{}\" --minimized", exe.display());
        key.set_value(RUN_VALUE, &command)
            .map_err(|e| format!("cannot write startup entry: {e}"))?;
    } else {
        match key.delete_value(RUN_VALUE) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot remove startup entry: {error}")),
        }
    }
    Ok(())
}

pub fn autostart_command() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ).ok()?;
    key.get_value::<String, _>(RUN_VALUE).ok()
}

pub fn is_autostart_enabled() -> bool {
    autostart_command().is_some()
}

/// Keep the registry in sync with the saved setting and the current exe path.
pub fn ensure_autostart(enabled: bool) {
    let expected = std::env::current_exe()
        .ok()
        .map(|exe| format!("\"{}\" --minimized", exe.display()));
    let current = autostart_command();

    if enabled {
        if current != expected {
            if let Err(error) = set_autostart(true) {
                crate::util::log(&format!("autostart update failed: {error}"));
            }
        }
    } else if current.is_some() {
        if let Err(error) = set_autostart(false) {
            crate::util::log(&format!("autostart removal failed: {error}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// Windows stores 0 for dark mode and 1 for light mode.
pub fn is_system_dark() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        KEY_READ,
    ) else {
        return false;
    };
    match key.get_value::<u32, _>("AppsUseLightTheme") {
        Ok(0) => true,
        Ok(_) => false,
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Register a per-user AppUserModelID so toasts are labelled "Syncora"
/// instead of "Windows PowerShell". This writes only to HKCU.
pub fn register_app_id() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(r"Software\Classes\AppUserModelId\{APP_ID}");
    if let Ok((key, _)) = hkcu.create_subkey(&path) {
        let _ = key.set_value("DisplayName", &APP_NAME);
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value("IconUri", &exe.display().to_string());
            let _ = key.set_value(
                "IconBackgroundColor",
                &"FF5B8CFF".to_string(),
            );
        }
    }
}

pub fn notify(title: &str, body: &str) {
    let first = Toast::new(APP_ID)
        .title(title)
        .text1(body)
        .sound(Some(Sound::Default))
        .duration(ToastDuration::Short)
        .show();
    if first.is_err() {
        // Fall back to the well-known PowerShell AppID, which works even when
        // the custom AppUserModelID has not been indexed yet.
        let _ = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(body)
            .duration(ToastDuration::Short)
            .show();
    }
}

// ---------------------------------------------------------------------------
// Single instance
// ---------------------------------------------------------------------------

/// Convert a string into a NUL-terminated UTF-16 buffer.
///
/// Win32 string APIs read until they meet a terminator, so passing a buffer
/// without one makes the dialog print whatever happens to sit next to it in
/// memory.
fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns true when another Syncora instance already owns the mutex.
/// The handle is intentionally leaked so the mutex lives as long as the app.
pub fn already_running() -> bool {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = to_wide("Local\\Syncora_SingleInstance_v1");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        // Without a mutex the app still works, it just loses the guard.
        return false;
    }
    unsafe { GetLastError() == ERROR_ALREADY_EXISTS }
}

/// Tell the user where the running copy is. The text is supplied by the caller
/// so it can be localized.
pub fn show_already_running_message(body: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK,
    };
    let title = to_wide(APP_NAME);
    let text = to_wide(body);
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

/// Draw the app icon procedurally so the executable stays a single file with
/// no external image assets. Returns tightly packed RGBA pixels.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let size = size.max(8);
    let supersample = 4u32;
    let big = size * supersample;
    let samples = (supersample * supersample) as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];

    for py in 0..size {
        for px in 0..size {
            let mut acc_r = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_b = 0.0f32;
            let mut acc_a = 0.0f32;
            for sy in 0..supersample {
                for sx in 0..supersample {
                    let x = (px * supersample + sx) as f32 / big as f32;
                    let y = (py * supersample + sy) as f32 / big as f32;
                    let (r, g, b, a) = sample_icon(x, y);
                    acc_r += r * a;
                    acc_g += g * a;
                    acc_b += b * a;
                    acc_a += a;
                }
            }
            let index = ((py * size + px) * 4) as usize;
            let alpha = acc_a / samples;
            if acc_a > 0.0001 {
                out[index] = (acc_r / acc_a).clamp(0.0, 255.0) as u8;
                out[index + 1] = (acc_g / acc_a).clamp(0.0, 255.0) as u8;
                out[index + 2] = (acc_b / acc_a).clamp(0.0, 255.0) as u8;
                out[index + 3] = (alpha * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn sample_icon(x: f32, y: f32) -> (f32, f32, f32, f32) {
    if !inside_rounded_rect(x, y, 0.03, 0.03, 0.97, 0.97, 0.22) {
        return (0.0, 0.0, 0.0, 0.0);
    }

    // Diagonal blue -> violet gradient.
    let t = ((x + y) * 0.5).clamp(0.0, 1.0);
    let mut r = lerp(86.0, 128.0, t);
    let mut g = lerp(132.0, 92.0, t);
    let mut b = lerp(255.0, 245.0, t);

    // A soft highlight in the top-left corner.
    let highlight = ((x - 0.18).powi(2) + (y - 0.16).powi(2)).sqrt();
    if highlight < 0.55 {
        let amount = (1.0 - highlight / 0.55).clamp(0.0, 1.0) * 0.16;
        r = lerp(r, 255.0, amount);
        g = lerp(g, 255.0, amount);
        b = lerp(b, 255.0, amount);
    }

    // Two arrows forming a cycle.
    let top_bar = dist_to_segment(x, y, 0.27, 0.365, 0.64, 0.365) <= 0.043;
    let top_head = point_in_triangle(
        x,
        y,
        (0.615, 0.272),
        (0.615, 0.458),
        (0.775, 0.365),
    );
    let bottom_bar = dist_to_segment(x, y, 0.73, 0.635, 0.36, 0.635) <= 0.043;
    let bottom_head = point_in_triangle(
        x,
        y,
        (0.385, 0.542),
        (0.385, 0.728),
        (0.225, 0.635),
    );

    if top_bar || top_head || bottom_bar || bottom_head {
        (255.0, 255.0, 255.0, 1.0)
    } else {
        (r, g, b, 1.0)
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn inside_rounded_rect(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> bool {
    if x < left || x > right || y < top || y > bottom {
        return false;
    }
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let wx = px - ax;
    let wy = py - ay;
    let length_sq = vx * vx + vy * vy;
    if length_sq <= f32::EPSILON {
        return ((px - ax).powi(2) + (py - ay).powi(2)).sqrt();
    }
    let t = ((wx * vx + wy * vy) / length_sq).clamp(0.0, 1.0);
    let dx = px - (ax + t * vx);
    let dy = py - (ay + t * vy);
    (dx * dx + dy * dy).sqrt()
}

fn point_in_triangle(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = sign(px, py, a, b);
    let d2 = sign(px, py, b, c);
    let d3 = sign(px, py, c, a);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn sign(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    (px - b.0) * (a.1 - b.1) - (a.0 - b.0) * (py - b.1)
}

/// Convert an RGBA buffer into an egui icon.
pub fn egui_icon(size: u32) -> egui::IconData {
    egui::IconData {
        rgba: icon_rgba(size),
        width: size,
        height: size,
    }
}

/// Ensure the data directory exists and is writable.
pub fn prepare_data_dir() -> bool {
    match crate::config::ensure_data_dir() {
        Ok(_) => true,
        Err(error) => {
            crate::util::log(&format!("cannot create data directory: {error}"));
            false
        }
    }
}

pub fn log_path_exists() -> bool {
    Path::new(&crate::config::log_path()).exists()
}
