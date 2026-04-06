# Condor Eye macOS Port — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port Condor Eye from Windows-only to macOS with vision + window management + click-through overlay, removing direct Anthropic API calls in favor of opt-in local CLI review.

**Architecture:** Single codebase with `#[cfg(target_os)]` gates. Platform-specific code in `windows.rs` (already split), `main.rs` (click-through, hotkeys, audio spawn, .env paths), and new `review.rs`. All shared modules (`capture.rs`, `compare.rs`, `config.rs`, `truth.rs`, `http_api.rs`) remain unchanged except path helpers.

**Tech Stack:** Tauri 2, Rust, objc2/objc2-app-kit (macOS NSWindow), core-graphics (CGWindowList), dirs crate (cross-platform paths), screenshots crate (already cross-platform).

**Spec:** `docs/superpowers/specs/2026-04-06-condor-eye-macos-port-design.md`

---

### Task 1: Add `dirs` crate and fix path helpers

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/config.rs:89-103`
- Modify: `src-tauri/src/http_api.rs:490-498`
- Modify: `src-tauri/src/main.rs:462-475`

- [ ] **Step 1: Add `dirs` dependency to Cargo.toml**

In `src-tauri/Cargo.toml`, add to `[dependencies]`:

```toml
dirs = "5"
```

- [ ] **Step 2: Replace `grid_config_path()` in http_api.rs**

Replace the function at `http_api.rs:490-498`:

```rust
pub(crate) fn grid_config_path() -> std::path::PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("Condor Eye").join("grid.json")
    } else {
        std::path::PathBuf::from("grid.json")
    }
}
```

- [ ] **Step 3: Replace `default_audio_output_dir()` in config.rs**

Replace the function at `config.rs:89-103`:

```rust
fn default_audio_output_dir() -> String {
    if let Some(data_dir) = dirs::data_local_dir() {
        return data_dir
            .join("condor_audio")
            .join("audio-taps")
            .to_string_lossy()
            .into_owned();
    }
    std::env::temp_dir()
        .join("condor_audio")
        .join("audio-taps")
        .to_string_lossy()
        .into_owned()
}
```

- [ ] **Step 4: Replace APPDATA .env loading in main.rs**

Replace the `APPDATA` block at `main.rs:463-469`:

```rust
if let Some(config_dir) = dirs::config_dir() {
    let _ = dotenvy::from_path(config_dir.join("Condor Eye").join(".env"));
}
```

- [ ] **Step 5: Run existing tests**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test`
Expected: All existing tests pass (tick_size, config, profile loading, etc.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/config.rs src-tauri/src/http_api.rs src-tauri/src/main.rs
git commit -m "refactor: replace APPDATA/LOCALAPPDATA with dirs crate for cross-platform paths"
```

---

### Task 2: Gate `set_click_through` and audio spawn for macOS

**Files:**
- Modify: `src-tauri/src/main.rs:382-410` (click-through)
- Modify: `src-tauri/src/main.rs:588-597` (audio server + watcher spawn)

- [ ] **Step 1: Add macOS deps to Cargo.toml**

In `src-tauri/Cargo.toml`, add:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSWindow", "NSRunningApplication", "NSApplication"] }
objc2-foundation = "0.3"
core-graphics = "0.24"
```

- [ ] **Step 2: Replace `set_click_through` with cfg-gated versions**

Replace the entire `set_click_through` function at `main.rs:382-410`:

```rust
/// Set click-through on the window (transparent areas pass clicks to windows behind).
#[tauri::command]
async fn set_click_through(window: tauri::Window, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        #[allow(non_snake_case)]
        mod win32 {
            extern "system" {
                pub fn GetWindowLongW(hWnd: isize, nIndex: i32) -> i32;
                pub fn SetWindowLongW(hWnd: isize, nIndex: i32, dwNewLong: i32) -> i32;
            }
            pub const GWL_EXSTYLE: i32 = -20;
            pub const WS_EX_TRANSPARENT: i32 = 0x00000020;
            pub const WS_EX_LAYERED: i32 = 0x00080000;
        }

        let hwnd = window.hwnd().map_err(|e| format!("No HWND: {}", e))?;
        let hwnd_val = hwnd.0 as isize;

        unsafe {
            let style = win32::GetWindowLongW(hwnd_val, win32::GWL_EXSTYLE);
            let new_style = if enabled {
                style | win32::WS_EX_TRANSPARENT | win32::WS_EX_LAYERED
            } else {
                style & !(win32::WS_EX_TRANSPARENT)
            };
            win32::SetWindowLongW(hwnd_val, win32::GWL_EXSTYLE, new_style);
        }
    }

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWindow;
        let ns_ptr = window.ns_window().map_err(|e| format!("No NSWindow: {}", e))?;
        unsafe {
            let ns_window = &*(ns_ptr as *const NSWindow);
            ns_window.setIgnoresMouseEvents(enabled);
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (window, enabled);
    }

    eprintln!("[CE] click-through: {}", if enabled { "ON" } else { "OFF" });
    Ok(())
}
```

- [ ] **Step 3: Gate audio server and watcher spawns**

Wrap the audio server and watcher spawns at `main.rs:588-597` with cfg:

```rust
#[cfg(target_os = "windows")]
{
    tauri::async_runtime::spawn(http_api::start_audio_server(
        ce_config.clone(),
        ce_config.audio_bind.clone(),
        ce_config.audio_port,
        audio_registry.clone(),
    ));
    tauri::async_runtime::spawn(audio_watcher::run_watcher(
        ce_config,
        audio_registry.clone(),
    ));
}
```

- [ ] **Step 4: Run tests**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test`
Expected: All tests pass.

- [ ] **Step 5: Verify it compiles on macOS**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo check`
Expected: Compiles without errors. (The `objc2` imports only activate on macOS, so this checks no regressions on the shared code.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/main.rs
git commit -m "feat: cfg-gate click-through and audio for macOS compilation"
```

---

### Task 3: Platform hotkey modifiers

**Files:**
- Modify: `src-tauri/src/main.rs:537-579` (shortcut registration)

- [ ] **Step 1: Replace hardcoded Ctrl+Shift with platform-aware modifiers**

Replace the three shortcut registration blocks at `main.rs:539-579`. Before the first shortcut, add a platform modifier:

```rust
#[cfg(target_os = "macos")]
let platform_mod = Modifiers::META | Modifiers::SHIFT;
#[cfg(not(target_os = "macos"))]
let platform_mod = Modifiers::CONTROL | Modifiers::SHIFT;
```

Then change each `Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyC)` to `Shortcut::new(Some(platform_mod), Code::KeyC)` — same for KeyM and KeyT.

- [ ] **Step 2: Run tests + compile check**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test && cargo check`
Expected: Pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat: use Cmd+Shift on macOS, Ctrl+Shift on Windows for global hotkeys"
```

---

### Task 4: macOS window management in `windows.rs`

**Files:**
- Modify: `src-tauri/src/windows.rs:295-308` (replace empty stubs with macOS implementation)

- [ ] **Step 1: Write test for macOS window listing**

At bottom of `windows.rs` tests module, add:

```rust
#[test]
fn list_windows_returns_vec() {
    // On macOS this should return actual windows (if display exists)
    // On non-display CI, it may be empty but must not panic
    let windows = list_windows();
    // Just verify it returns without error
    let _ = windows;
}
```

- [ ] **Step 2: Run test to verify it works with current stubs**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test windows::tests::list_windows_returns_vec`
Expected: PASS (empty vec from stub).

- [ ] **Step 3: Replace the non-Windows stub with a macOS implementation**

Replace `windows.rs:295-308` (the `#[cfg(not(target_os = "windows"))] mod platform` block):

```rust
#[cfg(target_os = "macos")]
mod platform {
    use super::WindowInfo;
    use core_graphics::display::{
        kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
        CGWindowListCopyWindowInfo,
    };
    use core_graphics::window::{
        kCGWindowBounds, kCGWindowLayer, kCGWindowName, kCGWindowNumber,
        kCGWindowOwnerName, kCGWindowOwnerPID,
    };

    pub fn list_windows() -> Vec<WindowInfo> {
        let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let window_list = unsafe { CGWindowListCopyWindowInfo(options, kCGNullWindowID) };
        let Some(window_list) = window_list else {
            return Vec::new();
        };

        let count = unsafe { core_foundation::array::CFArrayGetCount(window_list as _) };
        let mut results = Vec::new();

        for i in 0..count {
            let dict = unsafe {
                let ptr = core_foundation::array::CFArrayGetValueAtIndex(window_list as _, i);
                &*(ptr as *const core_foundation::dictionary::CFDictionary<
                    core_foundation::string::CFString,
                    core_foundation::base::CFType,
                >)
            };

            // Skip non-normal windows (layer != 0 means menu bar, dock, etc.)
            let layer = get_i32(dict, unsafe { kCGWindowLayer });
            if layer != 0 {
                continue;
            }

            let window_id = get_i32(dict, unsafe { kCGWindowNumber }) as u64;
            let pid = get_i32(dict, unsafe { kCGWindowOwnerPID }) as u32;
            let title = get_string(dict, unsafe { kCGWindowName }).unwrap_or_default();
            let owner = get_string(dict, unsafe { kCGWindowOwnerName }).unwrap_or_default();

            // Skip windows with no title
            if title.is_empty() {
                continue;
            }

            // Parse bounds
            let (x, y, width, height) = get_bounds(dict);
            if width == 0 && height == 0 {
                continue;
            }

            results.push(WindowInfo {
                hwnd: window_id,
                title,
                pid,
                x,
                y,
                width,
                height,
                class_name: owner, // macOS has no window class — use owner app name
            });
        }

        results
    }

    fn get_i32(
        dict: &core_foundation::dictionary::CFDictionary<
            core_foundation::string::CFString,
            core_foundation::base::CFType,
        >,
        key: core_foundation::string::CFStringRef,
    ) -> i32 {
        use core_foundation::base::TCFType;
        use core_foundation::string::CFString;
        let key = unsafe { CFString::wrap_under_get_rule(key) };
        dict.find(key)
            .and_then(|val| {
                let num = unsafe {
                    &*(val.as_CFTypeRef() as *const core_foundation::number::CFNumber)
                };
                num.to_i32()
            })
            .unwrap_or(0)
    }

    fn get_string(
        dict: &core_foundation::dictionary::CFDictionary<
            core_foundation::string::CFString,
            core_foundation::base::CFType,
        >,
        key: core_foundation::string::CFStringRef,
    ) -> Option<String> {
        use core_foundation::base::TCFType;
        use core_foundation::string::CFString;
        let key = unsafe { CFString::wrap_under_get_rule(key) };
        dict.find(key).map(|val| {
            let s = unsafe { CFString::wrap_under_get_rule(val.as_CFTypeRef() as _) };
            s.to_string()
        })
    }

    fn get_bounds(
        dict: &core_foundation::dictionary::CFDictionary<
            core_foundation::string::CFString,
            core_foundation::base::CFType,
        >,
    ) -> (i32, i32, u32, u32) {
        use core_foundation::base::TCFType;
        use core_foundation::string::CFString;
        let key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
        let Some(bounds_val) = dict.find(key) else {
            return (0, 0, 0, 0);
        };
        let bounds_dict = unsafe {
            &*(bounds_val.as_CFTypeRef()
                as *const core_foundation::dictionary::CFDictionary<
                    core_foundation::string::CFString,
                    core_foundation::base::CFType,
                >)
        };

        let x_key = core_foundation::string::CFString::new("X");
        let y_key = core_foundation::string::CFString::new("Y");
        let w_key = core_foundation::string::CFString::new("Width");
        let h_key = core_foundation::string::CFString::new("Height");

        let get_f64 = |k: &core_foundation::string::CFString| -> f64 {
            bounds_dict
                .find(*k)
                .and_then(|v| {
                    let num = unsafe {
                        &*(v.as_CFTypeRef() as *const core_foundation::number::CFNumber)
                    };
                    num.to_f64()
                })
                .unwrap_or(0.0)
        };

        (
            get_f64(&x_key) as i32,
            get_f64(&y_key) as i32,
            get_f64(&w_key) as u32,
            get_f64(&h_key) as u32,
        )
    }

    pub fn focus_window(hwnd: u64) -> bool {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        use objc2_foundation::NSArray;

        // Look up the PID from the window list, then activate the app
        let options =
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
        let window_list = unsafe { CGWindowListCopyWindowInfo(options, kCGNullWindowID) };
        let Some(window_list) = window_list else {
            return false;
        };

        let count = unsafe { core_foundation::array::CFArrayGetCount(window_list as _) };
        let mut target_pid: Option<i32> = None;
        for i in 0..count {
            let dict = unsafe {
                let ptr = core_foundation::array::CFArrayGetValueAtIndex(window_list as _, i);
                &*(ptr as *const core_foundation::dictionary::CFDictionary<
                    core_foundation::string::CFString,
                    core_foundation::base::CFType,
                >)
            };
            let wid = get_i32(dict, unsafe { kCGWindowNumber }) as u64;
            if wid == hwnd {
                target_pid = Some(get_i32(dict, unsafe { kCGWindowOwnerPID }));
                break;
            }
        }

        let Some(pid) = target_pid else {
            eprintln!("[CE] focus_window: window {} not found", hwnd);
            return false;
        };

        unsafe {
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
            if let Some(app) = app {
                app.activateWithOptions(
                    NSApplicationActivationOptions::NSApplicationActivateIgnoringOtherApps,
                );
                true
            } else {
                eprintln!("[CE] focus_window: no app for pid {}", pid);
                false
            }
        }
    }

    pub fn send_key_combo(combo: &str) {
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

        let lower = combo.to_lowercase();
        let parts: Vec<&str> = lower.split('+').collect();

        let mut flags = CGEventFlags::empty();
        let mut key_code: Option<CGKeyCode> = None;

        for part in &parts {
            match part.trim() {
                "cmd" | "command" | "meta" => flags |= CGEventFlags::CGEventFlagCommand,
                "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
                "alt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
                "shift" => flags |= CGEventFlags::CGEventFlagShift,
                "tab" => key_code = Some(48),
                s if s.len() == 1 => {
                    if let Some(c) = s.chars().next() {
                        key_code = Some(match c {
                            '1' => 18, '2' => 19, '3' => 20, '4' => 21, '5' => 23,
                            '6' => 22, '7' => 26, '8' => 28, '9' => 25, '0' => 29,
                            'a' => 0, 'b' => 11, 'c' => 8, 'd' => 2, 'e' => 14,
                            'f' => 3, 'g' => 5, 'h' => 4, 'i' => 34, 'j' => 38,
                            'k' => 40, 'l' => 37, 'm' => 46, 'n' => 45, 'o' => 31,
                            'p' => 35, 'q' => 12, 'r' => 15, 's' => 1, 't' => 17,
                            'u' => 32, 'v' => 9, 'w' => 13, 'x' => 7, 'y' => 16,
                            'z' => 6,
                            _ => return,
                        });
                    }
                }
                _ => {}
            }
        }

        if let Some(kc) = key_code {
            let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState);
            if let Ok(source) = source {
                if let Ok(key_down) = CGEvent::new_keyboard_event(source.clone(), kc, true) {
                    key_down.set_flags(flags);
                    key_down.post(CGEventTapLocation::HID);
                }
                if let Ok(key_up) = CGEvent::new_keyboard_event(source, kc, false) {
                    key_up.set_flags(flags);
                    key_up.post(CGEventTapLocation::HID);
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::WindowInfo;

    pub fn list_windows() -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn focus_window(_hwnd: u64) -> bool {
        false
    }

    pub fn send_key_combo(_combo: &str) {}
}
```

Also add `core-foundation` to macOS deps in Cargo.toml:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSWindow", "NSRunningApplication", "NSApplication"] }
objc2-foundation = "0.3"
core-graphics = "0.24"
core-foundation = "0.10"
```

- [ ] **Step 4: Run tests + compile check**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test && cargo check`
Expected: Tests pass, compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/windows.rs
git commit -m "feat: macOS window management via CGWindowList, NSRunningApplication, CGEvent"
```

---

### Task 5: Create `review.rs` — local CLI review module

**Files:**
- Create: `src-tauri/src/review.rs`
- Modify: `src-tauri/src/main.rs` (add `mod review;`, remove `mod claude;` usage from `capture_free`)

- [ ] **Step 1: Write test for review module**

Create `src-tauri/src/review.rs`:

```rust
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewProvider {
    Claude,
    Codex,
    None,
}

impl Default for ReviewProvider {
    fn default() -> Self {
        Self::None
    }
}

/// Review a screenshot using a local CLI tool.
///
/// Saves the PNG to a temp file, spawns the CLI, returns stdout.
/// Returns empty string for ReviewProvider::None.
pub async fn review_screenshot(
    provider: ReviewProvider,
    png_bytes: &[u8],
    prompt: &str,
) -> Result<String, String> {
    match provider {
        ReviewProvider::None => Ok(String::new()),
        ReviewProvider::Claude => run_cli_review("claude", png_bytes, prompt).await,
        ReviewProvider::Codex => run_cli_review("codex", png_bytes, prompt).await,
    }
}

async fn run_cli_review(
    tool: &str,
    png_bytes: &[u8],
    prompt: &str,
) -> Result<String, String> {
    // Save screenshot to temp file
    let tmp_dir = std::env::temp_dir().join("condor-eye-review");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let img_path = tmp_dir.join(format!("capture-{}.png", ts));
    std::fs::write(&img_path, png_bytes)
        .map_err(|e| format!("Failed to write temp image: {}", e))?;

    let img_path_str = img_path.to_string_lossy().to_string();
    let prompt_owned = prompt.to_string();
    let tool_owned = tool.to_string();

    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(&tool_owned);
        match tool_owned.as_str() {
            "claude" => {
                cmd.arg("-p")
                    .arg(&prompt_owned)
                    .arg("--allowedTools")
                    .arg("none")
                    .env("CLAUDE_IMAGE", &img_path_str);
            }
            "codex" => {
                cmd.arg("-q")
                    .arg(format!("{}\n\nImage: {}", prompt_owned, img_path_str));
            }
            _ => {
                cmd.arg(&prompt_owned);
            }
        }
        cmd.output()
    })
    .await
    .map_err(|e| format!("Task join: {}", e))?
    .map_err(|e| format!("{} not found or failed to run: {}", tool, e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&img_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{} failed: {}", tool, stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_provider_default_is_none() {
        assert_eq!(ReviewProvider::default(), ReviewProvider::None);
    }

    #[test]
    fn review_provider_deserialize() {
        let p: ReviewProvider = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(p, ReviewProvider::Claude);
        let p: ReviewProvider = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(p, ReviewProvider::Codex);
        let p: ReviewProvider = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(p, ReviewProvider::None);
    }

    #[tokio::test]
    async fn review_none_returns_empty() {
        let result = review_screenshot(ReviewProvider::None, b"fake png", "describe").await;
        assert_eq!(result.unwrap(), "");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test review::tests`
Expected: 3 tests pass.

- [ ] **Step 3: Register the module in main.rs**

Add `mod review;` after the existing `mod claude;` line in `main.rs:8`.

- [ ] **Step 4: Run full test suite**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/review.rs src-tauri/src/main.rs
git commit -m "feat: add review.rs — local CLI review via claude/codex"
```

---

### Task 6: Replace API calls in `http_api.rs` with opt-in review

**Files:**
- Modify: `src-tauri/src/http_api.rs` (handle_capture, handle_locate, CaptureRequest, CaptureResponse, imports)

- [ ] **Step 1: Update CaptureRequest to include review provider**

In `http_api.rs`, add to `CaptureRequest` struct (after `no_focus` field):

```rust
    /// Review provider: "claude", "codex", or "none" (default).
    /// When "none", capture returns image only with no AI description.
    #[serde(default)]
    pub review: crate::review::ReviewProvider,
```

- [ ] **Step 2: Replace handle_capture to use review instead of direct API**

Replace the handle_capture function body at `http_api.rs:270-356`. The capture logic stays the same, but replace the claude::describe_screenshot call with review:

```rust
async fn handle_capture(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CaptureRequest>,
) -> Result<Json<CaptureResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;

    let prompt = req.prompt.unwrap_or_else(|| {
        "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible.".to_string()
    });

    // Serialize captures — one at a time
    let _guard = state.capture_lock.lock().await;

    // Bring target window to foreground if hwnd provided (unless no_focus)
    if let Some(hwnd) = req.hwnd {
        if !req.no_focus {
            tokio::task::spawn_blocking(move || {
                crate::windows::focus_window(hwnd);
            })
            .await
            .map_err(|e| api_error(format!("Focus join: {}", e)))?;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }

    // Send key combos if requested
    if let Some(keys) = req.keys {
        for combo in &keys {
            let c = combo.clone();
            tokio::task::spawn_blocking(move || {
                crate::windows::send_key_combo(&c);
            })
            .await
            .map_err(|e| api_error(format!("Keys join: {}", e)))?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // Capture screen
    let (png, region) = if let Some(r) = req.region {
        let rx = r.x;
        let ry = r.y;
        let rw = r.width;
        let rh = r.height;
        let png = tokio::task::spawn_blocking(move || capture::capture_region(rx, ry, rw, rh))
            .await
            .map_err(|e| api_error(format!("Task join: {}", e)))?
            .map_err(|e| api_error(format!("Capture: {}", e)))?;
        (png, r)
    } else {
        tokio::task::spawn_blocking(capture::capture_full_screen)
            .await
            .map_err(|e| api_error(format!("Task join: {}", e)))?
            .map_err(|e| api_error(format!("Capture: {}", e)))?
    };

    eprintln!("[CE] captured {} bytes, region: {:?}", png.len(), region);

    // Opt-in review via local CLI
    let start = std::time::Instant::now();
    let description = crate::review::review_screenshot(req.review, &png, &prompt)
        .await
        .map_err(|e| api_error(format!("Review: {}", e)))?;
    let latency_ms = start.elapsed().as_millis() as u64;

    let image = base64::engine::general_purpose::STANDARD.encode(&png);

    eprintln!(
        "[CE] capture response: {}ms, {} chars",
        latency_ms,
        description.len()
    );

    Ok(Json(CaptureResponse {
        image,
        description,
        latency_ms,
        region,
        cost_estimate_usd: 0.0,
    }))
}
```

- [ ] **Step 3: Update handle_locate to work without API calls**

Replace handle_locate to return structured output from local window data instead of calling Claude:

```rust
async fn handle_locate(
    State(state): State<Arc<HttpState>>,
    Json(req): Json<LocateRequest>,
) -> Result<Json<LocateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // First try to find by window title (free, instant)
    let target = req.target.clone();
    let windows = tokio::task::spawn_blocking(move || {
        crate::windows::find_windows(&target)
    })
    .await
    .map_err(|e| api_error(format!("Task join: {}", e)))?;

    if let Some(w) = windows.first() {
        Ok(Json(LocateResponse {
            found: true,
            bounds: Some(Region {
                x: w.x,
                y: w.y,
                width: w.width,
                height: w.height,
            }),
            confidence: "high".to_string(),
            description: format!("Found window: {} (pid {})", w.title, w.pid),
        }))
    } else {
        Ok(Json(LocateResponse {
            found: false,
            bounds: None,
            confidence: "none".to_string(),
            description: format!("No window matching '{}' found", req.target),
        }))
    }
}
```

- [ ] **Step 4: Remove unused `claude` import from http_api.rs**

Remove or comment out `use crate::claude;` at the top of http_api.rs (line 17).

- [ ] **Step 5: Run tests + compile check**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test && cargo check`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/http_api.rs
git commit -m "feat: replace direct Anthropic API calls with opt-in local CLI review"
```

---

### Task 7: Update `capture_free` in main.rs to use review

**Files:**
- Modify: `src-tauri/src/main.rs:179-271` (capture_free function)

- [ ] **Step 1: Replace capture_free to remove direct API call**

Replace the `capture_free` function:

```rust
/// Free-mode capture — captures screenshot and optionally reviews via local CLI.
#[tauri::command]
async fn capture_free(
    window: tauri::Window,
    prompt: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let user_prompt = if prompt.is_empty() {
        "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible.".to_string()
    } else {
        prompt
    };

    eprintln!(
        "[VV] free capture: prompt={}",
        &user_prompt[..user_prompt.len().min(100)]
    );

    // Hide, capture, show
    let _ = window.hide();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let (cx, cy, cw, ch) = (pos.x, pos.y, size.width, size.height);

    let png = tokio::task::spawn_blocking(move || capture::capture_region(cx, cy, cw, ch))
        .await
        .map_err(|e| format!("Task join: {}", e))?
        .map_err(|e| format!("Capture: {}", e))?;

    let _ = window.show();
    eprintln!("[VV] free: captured {} bytes", png.len());

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);

    // Store for share commands
    *state.last_capture.lock().unwrap() = Some(LastCapture {
        description: String::new(),
        image_b64: b64.clone(),
    });

    // Return the base64 image — review happens separately if requested
    Ok(format!("Captured {} bytes. Image stored for sharing.", png.len()))
}
```

- [ ] **Step 2: Run tests**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test`
Expected: All pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "refactor: capture_free no longer calls Anthropic API directly"
```

---

### Task 8: Bundle config for macOS

**Files:**
- Modify: `src-tauri/tauri.conf.json`
- Create: `src-tauri/icons/icon.icns` (generated from icon.png)
- Modify: `src-tauri/capabilities/default.json` (no changes needed — permissions are cross-platform)

- [ ] **Step 1: Generate macOS icon**

```bash
cd /Users/mikebook/code/condor-eye/src-tauri
mkdir -p icon.iconset
sips -z 16 16   icons/icon.png --out icon.iconset/icon_16x16.png
sips -z 32 32   icons/icon.png --out icon.iconset/icon_16x16@2x.png
sips -z 32 32   icons/icon.png --out icon.iconset/icon_32x32.png
sips -z 64 64   icons/icon.png --out icon.iconset/icon_32x32@2x.png
sips -z 128 128 icons/icon.png --out icon.iconset/icon_128x128.png
sips -z 256 256 icons/icon.png --out icon.iconset/icon_128x128@2x.png
sips -z 256 256 icons/icon.png --out icon.iconset/icon_256x256.png
sips -z 512 512 icons/icon.png --out icon.iconset/icon_256x256@2x.png
sips -z 512 512 icons/icon.png --out icon.iconset/icon_512x512.png
sips -z 1024 1024 icons/icon.png --out icon.iconset/icon_512x512@2x.png
iconutil -c icns icon.iconset -o icons/icon.icns
rm -rf icon.iconset
```

- [ ] **Step 2: Update tauri.conf.json for macOS**

Replace the bundle section in `tauri.conf.json`:

```json
{
  "$schema": "https://raw.githubusercontent.com/nicedoc/tauri/tauri-v2/crates/tauri-cli/schema.json",
  "productName": "Condor Eye",
  "version": "0.1.0",
  "identifier": "com.condor.condor-eye",
  "build": {
    "frontendDist": "../src"
  },
  "app": {
    "withGlobalTauri": true,
    "security": {
      "csp": "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self' http://localhost:* http://127.0.0.1:*; img-src 'self' data:"
    },
    "windows": [
      {
        "title": "Condor Eye",
        "transparent": true,
        "decorations": false,
        "alwaysOnTop": true,
        "resizable": true,
        "shadow": false,
        "width": 400,
        "height": 700,
        "skipTaskbar": false
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": ["nsis", "app"],
    "icon": [
      "icons/icon.ico",
      "icons/icon.icns",
      "icons/icon.png"
    ],
    "windows": {
      "nsis": {
        "installMode": "currentUser"
      }
    },
    "macOS": {
      "minimumSystemVersion": "13.0"
    }
  }
}
```

- [ ] **Step 3: Add Info.plist entries for permissions**

Create `src-tauri/Info.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>NSScreenCaptureUsageDescription</key>
    <string>Condor Eye needs screen recording access to capture screen regions for analysis.</string>
    <key>NSAccessibilityUsageDescription</key>
    <string>Condor Eye needs accessibility access for global hotkeys and window management.</string>
</dict>
</plist>
```

- [ ] **Step 4: Run compile check**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo check`
Expected: Compiles.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/icons/icon.icns src-tauri/Info.plist
git commit -m "feat: macOS bundle config — .app target, icns icon, plist permissions"
```

---

### Task 9: Startup permission check

**Files:**
- Modify: `src-tauri/src/main.rs` (in the `setup` closure)

- [ ] **Step 1: Add macOS permission check after Tauri setup**

In `main.rs`, inside the `.setup(move |app| {` closure, after the HTTP server spawn block, add:

```rust
// macOS: check screen recording permission on startup
#[cfg(target_os = "macos")]
{
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        // Brief delay to let window initialize
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        match tokio::task::spawn_blocking(|| {
            capture::capture_region(0, 0, 1, 1)
        }).await {
            Ok(Ok(_)) => {
                eprintln!("[CE] Screen recording permission: OK");
            }
            _ => {
                eprintln!("[CE] WARNING: Screen recording permission not granted");
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.emit("permission-needed", "screen-recording");
                }
            }
        }
    });
}
```

- [ ] **Step 2: Add frontend handler for permission warning**

This is handled by the existing frontend event system — the JS just needs to listen:

Add to `src/app.js` near the top (after the click-through listener):

```javascript
// macOS: listen for permission warnings
appWindow.listen('permission-needed', (event) => {
  const msg = event.payload === 'screen-recording'
    ? 'Screen Recording permission needed. Go to System Settings → Privacy & Security → Screen Recording and enable Condor Eye.'
    : `Permission needed: ${event.payload}`;
  // Show in the results panel
  const results = document.getElementById('results');
  if (results) {
    results.textContent = msg;
    results.style.color = '#ff6b6b';
  }
  console.warn(`[CE] ${msg}`);
});
```

- [ ] **Step 3: Run compile check**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo check`
Expected: Compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/main.rs src/app.js
git commit -m "feat: macOS startup permission check for screen recording"
```

---

### Task 10: Build and smoke test

**Files:** None (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cd /Users/mikebook/code/condor-eye/src-tauri && cargo test`
Expected: All tests pass.

- [ ] **Step 2: Build the macOS .app**

Run: `cd /Users/mikebook/code/condor-eye && cargo tauri build`
Expected: Produces a `.app` bundle in `src-tauri/target/release/bundle/macos/`.

- [ ] **Step 3: Run the dev version**

Run: `cd /Users/mikebook/code/condor-eye && cargo tauri dev`
Expected: Window appears, transparent overlay visible. Check the console for:
- `[CE] Screen recording permission: OK` (or permission-needed warning)
- `[CE] HTTP API starting on 0.0.0.0:9050`
- No audio server startup (gated out)

- [ ] **Step 4: Test HTTP API**

```bash
curl http://localhost:9050/api/status
curl http://localhost:9050/api/windows
```
Expected: Status returns JSON with `running: true`. Windows returns a list of visible macOS windows.

- [ ] **Step 5: Final commit**

```bash
git add -A
git commit -m "chore: verify macOS build and smoke test"
```
