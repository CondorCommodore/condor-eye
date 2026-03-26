# Hive Prompt: condor_audio Next Phase

## Context

condor_audio is a per-app audio capture system in condor-eye (Rust/Tauri on Windows). It taps Zoom and Discord audio via WASAPI, writes 10s WAV chunks, POSTs to whisper-server for transcription, and forwards transcripts to condor-intel for Ollama gate + Claude extraction.

The prototype works end-to-end but has five known problems that need fixing before it can run unattended daily. A plan exists at `condor-eye/docs/condor-audio-next-phase.md`.

## Current State (read these files)

| File | What it does | Key lines |
|---|---|---|
| `condor-eye/src-tauri/src/audio.rs` | Tap lifecycle, chunk plan, WASAPI capture worker, process_is_running | :899 powershell shelling, :713 called in capture loop |
| `condor-eye/src-tauri/src/audio_watcher.rs` | 5s poll loop: session discovery → auto-start/stop taps | :22 stale check, :42 dedup filter, :93 cleanup filter |
| `condor-eye/src-tauri/src/config.rs` | AppConfig with all audio env vars | full file |
| `condor-eye/docs/condor-audio-next-phase.md` | The plan being reviewed | full file |
| `condor-eye/docs/app-audio-tap-project.md` | Original spec (chunk strategy, transport, security) | :80-93 session-first activation, :95-116 chunk/stitch |

## Five Problems to Solve

### 1. Session discovery is process-based, not audio-session-based
`enumerate_audio_sessions()` currently shells out to `Get-Process` and matches by name. The spec says use `IAudioSessionManager2::GetSessionEnumerator()` → `GetProcessId()` → match by exe path. Idle Zoom/Discord processes without active audio should NOT start capture.

**Question**: Should we use `wasapi-rs` for session enumeration (same crate as capture), or call `IAudioSessionManager2` directly via `windows-rs`? What's the minimal COM surface needed?

### 2. Worker health conflated with transcript health
A failed whisper POST marks the tap `Error`. The watcher was spawning duplicates (fixed band-aid: dedup any non-Stopped tap). Real fix: tap stays `Running` while capture is live. Transcript errors tracked separately.

**Question**: What should the ActiveTap state model look like? Options:
- A) Add `transcript_errors: u32` + `last_error: Option<String>` alongside existing `TapStatus`
- B) Split into `CaptureStatus` (Running/Stopped) + `TranscriptStatus` (Ok/Degraded/Failed)
- C) Keep single status but add `degraded_since: Option<DateTime>` for transcript failures

### 3. PowerShell in the capture hot path
`process_is_running()` at audio.rs:899 spawns `powershell.exe` on every iteration of the capture loop (audio.rs:713). This adds ~50-100ms latency per check and is called every 10 seconds inside the worker.

**Question**: Replace with `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, pid)` via `windows-rs`? Or move the check entirely out of the worker into the watcher's 5s poll? The worker would only check `stop_flag`.

### 4. No operator consent/status UX
No toast or tray indicator when capture is active. No way to see whisper degradation without hitting the API.

**Question**: Tauri 2 system tray API vs Windows-native `Shell_NotifyIcon`? The app already has a Tauri window — can we add tray state without a visible window?

### 5. No unattended startup path
condor-eye and whisper-server need to survive nightly Aurora restarts. No systemd timer or equivalent exists.

**Question**: Should condor-eye auto-launch via Windows Task Scheduler (it's a Tauri app) or via a systemd service on WSL that calls into Windows? whisper-server is already Docker — just needs `restart: unless-stopped` and a healthcheck.

## Debate Focus

Agents should focus on **implementation choices**, not whether to do the work. The plan is approved. Specifically:

1. **COM surface for session enumeration** — wasapi-rs vs raw windows-rs vs hybrid
2. **ActiveTap state model** — options A/B/C above, with trade-offs
3. **Where process liveness lives** — worker vs watcher vs both
4. **Startup orchestration** — Task Scheduler vs systemd vs Docker wrapper
5. **Execution order** — which of the 5 items should ship first given they interact

Each agent MUST read the source files listed above before stating a position. No ungrounded opinions. Reference file:line.
