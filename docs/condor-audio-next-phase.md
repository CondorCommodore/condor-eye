# Condor Audio Next Phase

## Goal

Move `condor_audio` from the current process-discovery prototype to a reliable
Windows audio-session-driven service that is ready for unattended daily use.

## Implementation Decisions (hive-resolved 2026-03-25)

| Question | Decision | Rationale |
|---|---|---|
| Session enumeration API | `windows-rs` direct (`IAudioSessionManager2`) | `wasapi-rs` wraps v1 manager only — v2 needed for global session enumeration |
| ActiveTap state model | Split: `CaptureStatus` + `TranscriptStatus` | Two independent failure modes need independent state machines |
| Process liveness | Native `OpenProcess` + `GetExitCodeProcess`, watcher-only | Removes PowerShell from hot path (~0.1ms vs 50-100ms). Worker checks `stop_flag` only. |
| Consent/tray UX | Tauri 2 built-in `tray-icon` + `tauri-plugin-notification` | No window needed for tray-only mode. Dynamic icon swap for active/idle. |
| Startup orchestration | `tauri-plugin-autostart` (HKCU Run) + Docker `restart: unless-stopped` | No NSSM or manual Task Scheduler. Desktop session required for tray. |

## Next Phase Work

### 1. Replace process discovery with true audio-session discovery

Use `windows-rs` directly for COM session enumeration. Add to `Cargo.toml`:

```toml
windows = { version = "0.62", features = [
    "Win32_Foundation",
    "Win32_Media_Audio",
    "Win32_System_Com",
    "Win32_System_Threading",
] }
```

Implementation (~50 lines in `audio.rs`):
1. `CoCreateInstance` → `IMMDeviceEnumerator` → `GetDefaultAudioEndpoint(eRender)`
2. `device.Activate::<IAudioSessionManager2>()`
3. `manager.GetSessionEnumerator()` → iterate sessions
4. For each: `IAudioSessionControl::QueryInterface::<IAudioSessionControl2>()` → `GetProcessId()`, `GetState()`
5. Filter to `AudioSessionStateActive` only
6. `OpenProcess(pid)` + `QueryFullProcessImageNameW()` → match exe path against target patterns

Keep `wasapi` crate only for the loopback capture path (where it already works).

**Files**: `audio.rs` (replace `enumerate_audio_sessions` stub), `audio_watcher.rs` (consumes sessions)
**Acceptance**: idle Zoom/Discord background processes do not trigger capture. Active audio sessions do.

### 2. Separate worker health from transcript health

Replace single `TapStatus` with split model:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptStatus {
    Ok,
    Degraded(u32),  // consecutive error count
    Failed,
}
```

Update `ActiveTap`:
```rust
pub capture_status: CaptureStatus,
pub transcript_status: TranscriptStatus,
pub last_transcript_error: Option<String>,
pub last_successful_transcript: Option<String>,
```

Watcher rules:
- Dedup on `capture_status == Running` (not transcript_status)
- Cleanup: stop tap when `capture_status == Stopped` or app disappears
- `/api/condor_audio/status` serializes both fields — consumers see `"capture_status": "running", "transcript_status": { "degraded": 3 }`

**Files**: `audio.rs` (types + worker), `audio_watcher.rs` (dedup/cleanup logic), `http_api.rs` (status response)
**Acceptance**: failed whisper does not create duplicate taps. Status API shows "capture alive, whisper degraded."

### 3. Remove shell-based liveness checks from the hot path

Replace `process_is_running()` at `audio.rs:899`:

```rust
#[cfg(target_os = "windows")]
pub(crate) fn process_is_running(pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        OpenProcess, GetExitCodeProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        if GetExitCodeProcess(handle, &mut code).is_ok() {
            code == 259 // STILL_ACTIVE
        } else {
            false
        }
    }
}
```

Remove the `process_is_running` call from the capture worker loop (`audio.rs:713`). Worker only checks `stop_flag.load(Ordering::Relaxed)`. Watcher owns all process-exit detection.

**Files**: `audio.rs` (replace fn + remove call from worker loop)
**Acceptance**: no `powershell.exe` spawns during capture. Worker focuses on audio + chunking.

### 4. Add operator-visible consent and status

Use Tauri 2 built-in tray support:

- Enable `tray-icon` feature in `tauri.conf.json`
- Add `tauri-plugin-notification` for toast notifications
- `TrayIconBuilder::new().icon(gray_icon).build(app)` on startup
- On tap start: `tray.set_icon(green_icon)` + toast "Recording Zoom audio"
- On all taps stopped: `tray.set_icon(gray_icon)`
- On transcript degraded: `tray.set_icon(yellow_icon)` + tooltip shows error count

Tray works without a visible window (confirmed: Tauri 2 supports tray-only apps).

**Files**: `main.rs` (tray setup), new `tray.rs` (icon state management), `audio_watcher.rs` (emit tray events)
**Acceptance**: visible notification on tap start. Tray icon reflects capture state. No window required.

### 5. Finish the unattended nightly path

Startup chain:
```
Windows boot → LxssManager starts WSL2 → dockerd → whisper-server (restart: unless-stopped)
User logon   → tauri-plugin-autostart (HKCU\...\Run) → condor-eye → tray visible
```

Implementation:
- Add `tauri-plugin-autostart` to condor-eye — writes to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
- whisper-server already has `restart: unless-stopped` in docker-compose
- Verify WSL2 auto-starts dockerd (check `/etc/wsl.conf` for `[boot] command=` or Docker Desktop auto-start)
- Add smoke test script: `scripts/test-nightly-restart.ps1` — reboot, verify tray + whisper + capture within 60s

**Files**: `Cargo.toml` (plugin dep), `main.rs` (plugin init), `scripts/test-nightly-restart.ps1` (new)
**Acceptance**: Aurora survives nightly restart. condor-eye + whisper-server running within 60s of user logon.

## Execution Order

Ship in this order — each step is independently useful and later steps depend on earlier ones:

1. **Step 3 first** (PowerShell removal) — smallest change, unblocks worker performance, no type changes
2. **Step 2** (state model split) — required before step 1 to handle session transitions cleanly
3. **Step 1** (session enumeration) — the core feature, depends on correct state model
4. **Step 4** (tray UX) — depends on capture status being reliable (steps 1-3)
5. **Step 5** (startup) — ship last, depends on everything else being stable

## Acceptance (end-to-end)

- Starting Zoom or Discord with active audio causes a tap to appear without manual API calls.
- Idle/background Zoom or Discord processes do not start capture.
- A failed whisper request does not create duplicate taps. Status shows "capture alive, whisper degraded."
- A spoken phrase becomes visible in transcript text within 12 seconds (10s chunk + 2s transcribe).
- No `powershell.exe` spawns during capture.
- System tray shows capture state. Toast on tap start.
- Aurora survives nightly restart without manual intervention.
