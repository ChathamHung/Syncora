# Syncora

**Keep two or more folders identical — automatically, in the background.**

Syncora is a small, fast Windows desktop app that watches your folders and keeps them
in sync. Add a task, pick two folders and a mode, and forget about it: changes are
detected within seconds, copied incrementally, and conflicts are surfaced instead of
silently overwriting your work.

Built entirely with **Rust** and **egui**. Ships as a single portable `.exe` — no
installer, no account, no admin rights, no telemetry.

![Syncora main window](docs/screenshot.png)

---

## Features

### Sync modes

| Mode | What it does | Typical use |
| --- | --- | --- |
| **One-way** | Copies new and changed files from source to target. Nothing is ever deleted on the target. | Simple backups |
| **Two-way** | Changes on either side propagate to the other side. | Keeping a laptop and a desktop folder in step |
| **Mirror** | Makes the target an exact copy of the source, including deletions. | Publishing, device copies, USB drives |

All modes are **incremental**: only new or modified files are transferred, so a sync of
a large folder with a few edits finishes in milliseconds.

### Conflict handling

When the same file changes on both sides, Syncora does not guess. It:

- records the conflict and shows a badge in the sidebar,
- sends a **Windows toast notification**,
- lets you resolve it with **Keep source**, **Keep target** or **Keep both**
  (the newer version keeps the original name, the older one is renamed with a
  `(conflict <date>)` suffix so nothing is lost),
- or applies an automatic rule you configure: *Ask*, *Keep both*, *Prefer source*,
  *Prefer target* or *Skip*.

### Background operation

- Runs from the **system tray**; closing the window keeps the app running.
- Each task syncs on its own schedule and, optionally, **in real time** via filesystem
  watching with a short debounce.
- A **master switch** pauses every task at once (also available from the tray menu).

### Interface

- Guided first-run screen and tooltips on every non-obvious control.
- **English and Chinese**, chosen from the system language on first launch and
  switchable in Settings.
- Follows the Windows **light/dark** theme, with a manual override.
- **Sync history** with per-run file counts, bytes, duration and result.
- **Preview** any task before it runs: see exactly what would be copied, updated,
  deleted or flagged as a conflict.
- Export and import settings as a single JSON file.

### Extras

- **Start with Windows** (off by default, stored per user in `HKCU`, never needs admin).
- **Checksum verification** — re-reads each copy and compares a BLAKE3 hash.
- **Exclude patterns** per task (glob syntax, e.g. `*.tmp`, `node_modules/**`).
- **Skip hidden and system files** toggle.
- **Per-task conflict policy** that overrides the global default.
- **Path validation** — refuses source/target pairs that are identical, nested, or
  missing, before anything is touched.
- **Live progress** with a cancel button, plus copy-path and open-folder shortcuts on
  every task card.
- **Log file** with automatic rotation, reachable from Settings.

---

## Install

### Download

Grab `Syncora.exe` from the
[Releases page](https://github.com/ChathamHung/Syncora/releases) and run it. It is a
single portable executable — nothing to install, no dependencies, no admin prompt.

### Build from source

Requires a recent stable Rust toolchain and the MSVC build tools.

```bash
git clone https://github.com/ChathamHung/Syncora.git
cd Syncora
cargo build --release
```

The executable is written to `target/release/Syncora.exe`.

Run the test suite (sync engine and configuration):

```bash
cargo test
```

---

## How it works

Syncora keeps a small snapshot of every task — the relative path, size, modification
time and BLAKE3 hash of each file at the end of the last successful sync. A sync run
scans both sides and compares them against that snapshot, which is enough to tell
*"this file is new"* from *"this file was deleted"* and to recognise a genuine
two-sided edit as a conflict rather than a deletion.

Copies are atomic: data is written to a temporary file, the original modification time
is restored, and the file is renamed into place, so an interrupted sync never leaves a
half-written file behind.

### Where things are stored

| Path | Contents |
| --- | --- |
| `%APPDATA%\Syncora\config.json` | Tasks and settings |
| `%APPDATA%\Syncora\history.json` | Sync history |
| `%APPDATA%\Syncora\conflicts.json` | Unresolved conflicts |
| `%APPDATA%\Syncora\snapshots\` | Per-task last-synced snapshots |
| `%APPDATA%\Syncora\syncora.log` | Diagnostic log (rotated at 2 MB) |

Everything is plain JSON, so the configuration is easy to inspect, back up or version.

---

## Project layout

```
src/
  main.rs      Entry point, window setup, single-instance guard
  app.rs       All UI: pages, task cards, modals, toasts
  engine.rs    Background worker thread: scheduling, watchers, job queue
  sync.rs      Sync engine: scanning, planning, executing (UI-free and unit tested)
  config.rs    Settings, tasks and persistence
  theme.rs     Colour palette and egui styling
  i18n.rs      English / Chinese strings
  platform.rs  Windows integration: autostart, toasts, tray, DPI, mutex
  tray.rs      Tray icon and menu
  util.rs      Logging, formatting and file helpers
```

The sync engine in `sync.rs` has no UI dependencies, which is why the planning and
conflict logic can be tested without a window.

## Tech stack

[egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui) for the
GUI, [`notify`](https://crates.io/crates/notify) for filesystem watching,
[`blake3`](https://crates.io/crates/blake3) for hashing,
[`tray-icon`](https://crates.io/crates/tray-icon) for the tray icon and
[`tauri-winrt-notification`](https://crates.io/crates/tauri-winrt-notification) for
Windows toasts.

---

## 中文简介

**Syncora 是一款让多个文件夹始终保持一致的 Windows 同步工具**，使用 Rust 与 egui
编写，单文件绿色运行，无需安装、无需账号、无需管理员权限。

**三种同步模式**

- **单向同步**：只从来源复制到目标，目标端永不删除，适合做备份。
- **双向同步**：两侧改动互相传播，适合多台设备之间保持一致。
- **镜像同步**：让目标与来源完全相同，来源端删除的文件在目标端也会被删除。

所有模式都是**增量同步**，只传输新增或修改过的文件。

**冲突处理**：同一文件两侧都被修改时不会擅自覆盖。程序会在侧边栏标记、弹出 Windows
通知，由你选择「保留来源」「保留目标」或「两者都保留」（较旧的版本会加上冲突后缀），
也可以在设置中指定自动规则。

**其他特性**：实时文件监视与定时检查、总开关、同步历史、执行前预览、排除规则、
BLAKE3 校验、开机自启（默认关闭）、托盘常驻、中英文界面、跟随系统深/浅色主题、
设置导入导出。

**自行编译**

```bash
git clone https://github.com/ChathamHung/Syncora.git
cd Syncora
cargo build --release
```

生成的 `target/release/Syncora.exe` 即为可直接运行的单文件程序。

---

## License

[MIT](LICENSE) © 2026 ChathamHung

Free to use, modify and redistribute — including commercially — as long as the
copyright notice and permission notice are kept.

本项目采用 [MIT 许可证](LICENSE)，可自由使用、修改与再分发（含商业用途），
只需保留版权声明与许可声明。

