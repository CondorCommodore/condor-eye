# Condor Eye macOS Port

**Date:** 2026-04-06
**Status:** Approved (hive-local, 4 agents, 1 round, converged 0.95)
**Target:** This MacBook (18-core Apple Silicon, Darwin 25.4.0)

## Goal

Port Condor Eye from Windows-only to macOS. Scope: vision + window management + click-through overlay. No audio. Remove direct Anthropic API calls; replace with opt-in review via local CLI tools (claude or codex).

## Architecture

Single codebase, `#[cfg(target_os)]` gates for platform-specific code. The existing `windows.rs` already uses this pattern. No separate macOS fork.

### Module Map

| Module | Windows (existing) | macOS (new) | Shared |
|--------|-------------------|-------------|--------|
| `capture.rs` | `screenshots` crate (Win32 GDI) | `screenshots` crate (CGDisplayCreateImage) | Yes, no changes |
| `windows.rs` | Win32 FFI: `EnumWindows`, `SetForegroundWindow`, `keybd_event` | `CGWindowListCopyWindowInfo`, `NSRunningApplication`, `CGEventCreateKeyboardEvent` | Public API unchanged |
| `main.rs` click-through | `GetWindowLongW` + `WS_EX_TRANSPARENT` via `window.hwnd()` | `NSWindow.setIgnoresMouseEvents` via `window.ns_window()` + objc2 | Tauri command signature unchanged |
| `main.rs` hotkeys | `Modifiers::CONTROL \| Modifiers::SHIFT` | `Modifiers::META \| Modifiers::SHIFT` | Same key codes (C, M, T) |
| `main.rs` .env paths | `%APPDATA%/Condor Eye/` | `~/Library/Application Support/Condor Eye/` via `dirs::config_dir()` | `dirs` crate handles both |
| `http_api.rs` grid path | `%APPDATA%/Condor Eye/grid.json` | `~/Library/Application Support/Condor Eye/grid.json` via `dirs::config_dir()` | `dirs` crate handles both |
| `audio.rs` | WASAPI loopback capture | Compile-gated out (empty stubs) | Already stubbed |
| `audio_watcher.rs` | Background session monitor | Not spawned on macOS | Gate at `main.rs:594-597` |
| `review.rs` (new) | — | — | Replaces `claude.rs` direct API calls |
| `claude.rs` | Direct reqwest to api.anthropic.com | Removed | Replaced by `review.rs` |
| `compare.rs` | Redis comparison | Same | Platform-agnostic, kept |
| `config.rs` | AppConfig from env | Same, minus API key requirement | `ANTHROPIC_API_KEY` no longer required |
| `truth.rs` | Redis ground truth | Same | Platform-agnostic |
| `http_api.rs` | Axum vision + audio servers | Vision server only on macOS | Audio server gated out |

### What Does NOT Change

- Frontend (`src/index.html`, `src/app.js`) — vanilla HTML/JS, no platform deps
- MCP server (`mcp/index.js`) — wraps HTTP API, unaffected
- Extraction profiles (`profiles/*.json`) — JSON files, platform-agnostic
- `compare.rs`, `truth.rs` — pure Rust + Redis, no Win32

## Compile Blockers (from hive review)

### 1. `set_click_through` — main.rs:382-410

The entire function body uses Win32 FFI with no `#[cfg]` guard. `window.hwnd()` does not exist on macOS.

**Fix:** Split into two `#[cfg]` bodies:

```rust
#[tauri::command]
async fn set_click_through(window: tauri::Window, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // existing Win32 GetWindowLongW / SetWindowLongW code
        // uses window.hwnd()
    }

    #[cfg(target_os = "macos")]
    {
        // window.ns_window() returns Result<*mut c_void>
        // cast to objc2 NSWindow, call setIgnoresMouseEvents:
        use objc2::rc::Retained;
        use objc2_app_kit::NSWindow;
        let ns_win = window.ns_window().map_err(|e| format!("No NSWindow: {}", e))?;
        unsafe {
            let ns_window: Retained<NSWindow> = Retained::retain(ns_win as *mut NSWindow)
                .ok_or("Failed to retain NSWindow")?;
            ns_window.setIgnoresMouseEvents(enabled);
        }
    }

    Ok(())
}
```

### 2. `audio_watcher` spawn — main.rs:594-597

```rust
// Gate the audio watcher spawn
#[cfg(target_os = "windows")]
tauri::async_runtime::spawn(audio_watcher::run_watcher(
    ce_config,
    audio_registry.clone(),
));
```

### 3. Hotkey modifiers — main.rs:539-579

```rust
#[cfg(target_os = "macos")]
let modifier = Modifiers::META | Modifiers::SHIFT;
#[cfg(not(target_os = "macos"))]
let modifier = Modifiers::CONTROL | Modifiers::SHIFT;

let shortcut = Shortcut::new(Some(modifier), Code::KeyC);
```

## Path Handling

Replace all manual `APPDATA`/`LOCALAPPDATA` reads with the `dirs` crate. This resolves three sites:

| Site | Current code | Replacement |
|------|-------------|-------------|
| `main.rs:464` | `std::env::var("APPDATA")` | `dirs::config_dir().map(\|d\| d.join("Condor Eye"))` |
| `http_api.rs:grid_config_path()` | `std::env::var("APPDATA")` fallback to cwd | `dirs::config_dir().map(\|d\| d.join("Condor Eye").join("grid.json"))` |
| `config.rs:default_audio_output_dir()` | `std::env::var("LOCALAPPDATA")` | `dirs::data_local_dir()` (low priority, audio stubbed) |

The `dirs` crate returns:
- Windows: `C:\Users\<user>\AppData\Roaming` / `Local`
- macOS: `~/Library/Application Support`

## API Changes

### Remove direct Anthropic calls

Delete the `reqwest` calls to `api.anthropic.com` in:
- `capture_free` (`main.rs:213-260`)
- `extract_from_screenshot` / `describe_screenshot` (`claude.rs`)

### New: `review.rs`

Replaces `claude.rs`. Spawns local CLI tools as child processes:

```rust
pub enum ReviewProvider {
    Claude,
    Codex,
    None,
}

pub async fn review_screenshot(
    provider: ReviewProvider,
    image_path: &Path,
    prompt: &str,
) -> Result<String, String> {
    match provider {
        ReviewProvider::None => Ok(String::new()),
        ReviewProvider::Claude => {
            // spawn: claude -p "prompt" --image <path>
            // capture stdout
        }
        ReviewProvider::Codex => {
            // spawn: codex -q "prompt" --image <path>
            // capture stdout
        }
    }
}
```

### HTTP API changes

- `POST /api/capture` gains optional `review` param: `claude`, `codex`, or `none` (default: `none`)
- `CaptureResponse` shape is preserved — `description` is `""` when no review
- `POST /api/screenshot` (raw capture) unchanged
- Audio API (port 9051) not started on macOS

### MCP server

`mcp/index.js` — add `review_provider` param to `condor_eye_capture` tool. Default: `none`.

## macOS Window Management

### `windows.rs` — new `#[cfg(target_os = "macos")]` platform module

**`list_windows()`** — `CGWindowListCopyWindowInfo` with `kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements`. Returns window ID, title, PID, bounds. Map `kCGWindowNumber` to the `hwnd` field (reused as a generic window ID).

**`focus_window(hwnd)`** — Look up PID from window ID, get `NSRunningApplication` for that PID, call `activateWithOptions(.activateIgnoringOtherApps)`. Then use AXUIElement to raise the specific window if the app has multiple.

**`send_key_combo(combo)`** — `CGEventCreateKeyboardEvent` + `CGEventPost`. Map modifier names to `CGEventFlags` (`kCGEventFlagMaskCommand`, `kCGEventFlagMaskControl`, etc.). Map key names to virtual key codes.

## Bundle Configuration

### `tauri.conf.json` changes

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis", "app"],
    "icon": ["icons/icon.ico", "icons/icon.icns"],
    "macOS": {
      "minimumSystemVersion": "13.0",
      "exceptionDomain": null
    }
  }
}
```

### Info.plist entries (via Tauri macOS config)

- `NSScreenRecordingUsageDescription`: "Condor Eye needs screen recording access to capture screen regions for analysis."
- `NSAccessibilityUsageDescription`: "Condor Eye needs accessibility access for global hotkeys and window management."

### Icon

Generate `icon.icns` from existing `icon.ico` using `sips` or `iconutil` on macOS.

## Startup Permission Check

On macOS, at app launch after Tauri setup:

1. Attempt a 1x1 test capture via `screenshots::Screen::all()` + `capture()`
2. If the result is a black image or error, emit a Tauri event `permission-needed` to the frontend
3. Frontend shows a clear message: "Grant Screen Recording permission in System Settings > Privacy & Security"

## Cargo.toml Changes

```toml
[dependencies]
# ... existing deps ...
dirs = "5"

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.5"
objc2-app-kit = { version = "0.2", features = ["NSWindow", "NSRunningApplication"] }
core-graphics = "0.24"
```

## Removed

- `ANTHROPIC_API_KEY` env var requirement
- Direct reqwest calls to `api.anthropic.com`
- Cost estimation logic in `config.rs`
- `claude.rs` module (replaced by `review.rs`)

## Kept

- Redis comparison (`compare.rs` + `truth.rs`) — platform-agnostic, useful for remote validation
- Extraction profiles — still define prompts for CLI review
- `REDIS_URL` env var — optional, comparison works if Redis reachable
