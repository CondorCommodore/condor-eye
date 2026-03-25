# Aurora Handoff

This project has to be continued on the Windows Aurora machine.

The new audio surface is named `condor_audio`. The desktop app is still `condor-eye`.

## Current State

What is wired:

- `condor-eye` starts two HTTP listeners:
  - `CONDOR_EYE_BIND:CONDOR_EYE_PORT` for the existing screen API, default `0.0.0.0:9050`
  - `CONDOR_AUDIO_BIND:CONDOR_AUDIO_PORT` for the new audio API, default `127.0.0.1:9051`
- MCP exposes:
  - `condor_audio_status`
  - `condor_audio_start`
  - `condor_audio_stop`
  - `condor_audio_latest`
- Audio directories default to `%LOCALAPPDATA%\\condor_audio\\audio-taps\\`
- The watcher task and audio API are present
- Manual tap mode is the default operator path
- Watcher auto-start is optional via `CONDOR_AUDIO_AUTO_WATCH=true`
- A standalone browser UI lives under `audio-mini-ui/`
- Manual taps can resolve a running Zoom or Discord process and start a loopback capture worker
- Completed WAV chunks are POSTed to whisper and persisted as sibling `.txt` transcripts
- `condor-eye` can optionally POST successful transcripts to `condor-intel`

What is still incomplete / not fully verified:

- True Windows audio-session enumeration; current session listing is a process-discovery fallback
- consent notification / tray indicator
- Live Windows runtime verification on Aurora against real Zoom and Discord calls

Important consequence:

- `GET /api/condor_audio/status` should work once the app is running
- `GET /api/condor_audio/sessions` should return running Zoom/Discord processes
- `POST /api/condor_audio/taps` should start a manual tap for a running process
- the realistic v1 test path is manual tap start through the mini UI or direct API calls

## Repo

Working tree on Forge/Linux:

- repo: `condor-eye`
- key files:
  - `src-tauri/src/main.rs`
  - `src-tauri/src/http_api.rs`
  - `src-tauri/src/audio.rs`
  - `src-tauri/src/audio_watcher.rs`
  - `src-tauri/src/config.rs`
  - `mcp/index.js`
  - `docs/app-audio-tap-project.md`

## Aurora Prereqs

Install or verify:

- Rust toolchain on Windows
- Node.js 18+
- WebView2 runtime
- 1Password CLI if using `CAPTURE_TOKEN` retrieval via `op.exe`
- Optional for later transcription:
  - Docker Desktop or a local `whisper-server` binary

## Env Setup

Place a `.env` file in one of the app lookup paths:

1. repo root `.env`
2. parent of `src-tauri`
3. `%APPDATA%\\Condor Eye\\.env`
4. next to the built executable

Minimum useful values:

```env
ANTHROPIC_API_KEY=...
CAPTURE_TOKEN=...

CONDOR_EYE_BIND=0.0.0.0
CONDOR_EYE_PORT=9050

CONDOR_AUDIO_BIND=127.0.0.1
CONDOR_AUDIO_PORT=9051
CONDOR_AUDIO_OUTPUT_DIR=%LOCALAPPDATA%\\condor_audio\\audio-taps

AUDIO_TRANSPORT=http
CONDOR_AUDIO_AUTO_WATCH=false
WHISPER_URL=http://localhost:8080/inference
CONDOR_INTEL_URL=http://localhost:8791
```

Compatibility aliases still work if older env is present:

- `CONDOR_EYE_AUDIO_BIND`
- `CONDOR_EYE_AUDIO_PORT`
- `CONDOR_EYE_AUDIO_OUTPUT_DIR`

## Start In Dev Mode

From Windows PowerShell in the repo root:

```powershell
cargo tauri dev
```

If launching from WSL against the Windows toolchain:

```bash
cargo.exe tauri dev
```

## Expected Startup Behavior

On successful startup:

- Tauri window opens
- existing Condor Eye API binds on `:9050`
- audio API binds on `:9051`
- audio output directories are created under `%LOCALAPPDATA%\\condor_audio\\audio-taps`

Expected audio log lines:

```text
[condor_audio] CAPTURE_TOKEN set — audio API authorized
[condor_audio] HTTP API starting on 127.0.0.1:9051
```

Expected current behavior with default manual mode:

```text
[condor_audio] auto-watch disabled; manual tap mode is active
```

If watcher mode is explicitly enabled:

```text
[condor_audio] watcher starting: bind=127.0.0.1 port=9051 transport=http
```

## Smoke Test

From PowerShell:

```powershell
curl http://localhost:9050/api/status
```

```powershell
curl -H "Authorization: Bearer $env:CAPTURE_TOKEN" http://localhost:9051/api/condor_audio/status
```

Or open `audio-mini-ui/index.html` and point it at `http://127.0.0.1:9051`.

This should succeed now:

- `/api/status`
- `/api/condor_audio/status`

Process-discovery fallback should now work:

```powershell
curl -H "Authorization: Bearer $env:CAPTURE_TOKEN" http://localhost:9051/api/condor_audio/sessions
```

Manual tap start should now work if Zoom or Discord is already running:

```powershell
curl -X POST `
  -H "Authorization: Bearer $env:CAPTURE_TOKEN" `
  -H "Content-Type: application/json" `
  -d '{"app":"zoom"}' `
  http://localhost:9051/api/condor_audio/taps
```

Latest transcript should work after the first completed chunk and a healthy whisper response:

```powershell
curl -H "Authorization: Bearer $env:CAPTURE_TOKEN" http://localhost:9051/api/condor_audio/taps/zoom-REPLACE_ME/latest-transcript
```

## MCP Check

From the repo root:

```powershell
node mcp/index.js
```

If registering in Claude Code:

```powershell
claude mcp add --scope user condor-eye -- node C:\path\to\condor-eye\mcp\index.js
```

The new audio tools are:

- `condor_audio_status`
- `condor_audio_start`
- `condor_audio_stop`
- `condor_audio_latest`

## Startup Troubleshooting

### `cargo` not found

Install Rust with `rustup-init.exe`, then reopen PowerShell.

Verify:

```powershell
cargo --version
rustc --version
```

### WebView2 / Tauri startup failure

Symptoms:

- window never opens
- immediate Tauri crash

Check:

- WebView2 runtime installed
- running from normal Windows PowerShell, not a restricted shell

### `CAPTURE_TOKEN` missing

Symptoms:

- `/api/capture` and `/api/condor_audio/*` return `403`

Fix:

```powershell
$env:CAPTURE_TOKEN = (op.exe read 'op://Dev/condor-eye-capture/token').Trim()
```

Or place `CAPTURE_TOKEN=...` in `.env`.

### Port bind failure on `9050` or `9051`

Symptoms:

- log says failed to bind

Check:

```powershell
netstat -ano | findstr :9050
netstat -ano | findstr :9051
```

Fix:

- stop the conflicting process, or
- move ports via `.env`

Example:

```env
CONDOR_EYE_PORT=9060
CONDOR_AUDIO_PORT=9061
```

### Audio API up, but taps do not start

Check:

- Zoom or Discord is actually running on Windows
- `POST /api/condor_audio/sessions` shows a matching process
- whisper is reachable if the tap starts but no transcript appears

If tap start still fails, the likely failure point is in `src-tauri/src/audio.rs` inside the Windows loopback worker path.

### Whisper server unreachable

The app should still start and write WAV chunks, but transcript `.txt` files will not appear.

Verify:

```powershell
curl http://localhost:8080
```

If using Docker:

```powershell
docker ps
```

## Next Step On Aurora

Resume implementation in this order:

1. Replace the stub in `src-tauri/src/audio.rs` with real Windows session enumeration.
2. Add real per-process loopback capture and 10-second WAV chunk writing.
3. Wire transcript POSTs to `WHISPER_URL`.
4. Add a consent indicator once taps can actually start.

Do not spend time debugging transcription until `enumerate_audio_sessions()` works on Aurora.
