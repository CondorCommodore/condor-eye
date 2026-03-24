# App Audio Tap → Knowledge Capture Pipeline

## Problem Statement

I have two audio channels (Zoom calls, Discord voice) that carry important information throughout the trading day — market commentary, design discussions, coordination calls. I'm not always at my desk. When I come back, that knowledge is gone.

I need a pipeline that:
1. **Captures audio** from Zoom and Discord independently (not whole-system loopback)
2. **Converts to text** — real-time or near-real-time transcription
3. **Extracts actionable insights** — LLM parses transcripts for decisions, action items, questions
4. **Routes to the right place** — brainstorm-ui nodes, coord tasks, memory files, Discord summaries

**Step 1 (this project):** Capture the audio and convert to text. Everything else depends on this working.

## Why Custom Capture (Not Zoom AI / Craig Bot)

Hive investigated "buy first" alternatives. Evidence round killed them:

| Alternative | Why it doesn't work | Source |
|---|---|---|
| Zoom AI Notes API | **No programmatic access** — returns error 3322, webhook doesn't fire, UI-only download | [Zoom Dev Forum](https://devforum.zoom.us/t/api-access-for-zoom-ai-companion-custom-ai-notetaker-transcripts-summaries-integration/135692) |
| Zoom Cloud Recording VTT | Post-meeting only, **~2x meeting duration** processing lag — violates 30s criterion | [Recall.ai analysis](https://www.recall.ai/blog/zoom-transcript-api) |
| Craig bot for Discord | **No download API** — DM link only. Can't record DMs/group calls. Known reliability issues. | [craig.chat/faq](https://craig.chat/faq/) |
| Recall.ai-style meeting bot | Works (~500ms latency) but adds external dependency, per-meeting cost, requires bot-as-participant | [recall.ai docs](https://docs.recall.ai/docs/zoom-overview) |

Custom WASAPI per-process capture is the right path. The `wasapi-rs` Rust crate makes it tractable.

## Why This Lives In Condor Eye

`condor-eye` already owns:
- Native Windows execution (Tauri 2 / Rust)
- A long-running local service boundary
- An HTTP API (axum, port 9050) and MCP-friendly control plane
- Process/window discovery code

---

# Technical Spec

## Capture Layer: WASAPI Per-Process Loopback

### API

Windows `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS` via `ActivateAudioInterfaceAsync`.
- Available since **Windows 10 Build 20348** (Win11 / Server 2022)
- Captures audio from a specific process tree without affecting other apps
- `ProcessLoopbackMode::IncludeTargetProcessTree` handles child processes (Zoom spawns audio in subprocesses)

Ref: [Microsoft Learn — AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)

### Rust Crate

`wasapi` (crate by HEnquist) — wraps windows-rs for WASAPI.

```rust
use wasapi::AudioClient;

// Initialize COM on capture thread
wasapi::initialize_mta().unwrap();

// Open per-process loopback capture (include child process tree)
let client = AudioClient::new_application_loopback_client(zoom_pid, true)?;
```

- Crate: [crates.io/crates/wasapi](https://crates.io/crates/wasapi)
- Source: [github.com/HEnquist/wasapi-rs](https://github.com/HEnquist/wasapi-rs)
- Working example: [wasapi-rs/examples/loopback.rs](https://github.com/HEnquist/wasapi-rs/blob/master/examples/loopback.rs)
- Reference impl: [masonasons/AudioCapture](https://github.com/masonasons/AudioCapture) (open-source per-app capture tool using same API)

Note: `cpal` does NOT support per-process loopback ([cpal issue #476](https://github.com/RustAudio/cpal/issues/476)).

### Cargo.toml Addition

```toml
[dependencies]
wasapi = "0.15"
hound = "3.5"    # WAV file writing
```

### Tap Activation (audio-session-first, not process-first)

The source of truth for "should we be capturing?" is **active audio sessions**, not running processes. Zoom and Discord both spawn background/helper processes that exist without active call audio. Process names are a candidate filter only.

Background watcher task, every 5 seconds:
1. `IAudioSessionManager2::GetSessionEnumerator()` → enumerate all active audio render sessions
2. For each session: `IAudioSessionControl2::GetProcessId()` → get owning PID
3. `OpenProcess(pid)` + `QueryFullProcessImageName()` → get exe path
4. Match exe path against target patterns (e.g., path contains `zoom`, `discord`)
5. Filter: only sessions in `AudioSessionStateActive` (skip expired/inactive sessions)
6. If a matching **active audio session** is found and no tap exists → start tap with that PID
7. If an existing tap's session transitions to inactive or the process exits → stop tap, finalize last chunk

This prevents false-positive taps from idle Zoom/Discord background processes. The `include_tree: true` loopback param captures child processes of the matched PID (handles `CptHost.exe`, `ZoomAudioService`, Discord `discord_voice`).

### Chunk Strategy

**10-second timestamped WAV files**, not ring buffer.

Why 10s, not 30s: The acceptance criterion is "searchable within 30 seconds of being spoken." Worst-case latency = chunk fill time + transcription time. With whisper.cpp on `base.en`, a 10s chunk transcribes in ~1-2s. Worst case: word spoken at chunk start → 10s fill + 2s transcribe = **12s**. A 30s chunk would be 30s + 2s = 32s, violating the SLA for words spoken early in a chunk.

- Pattern: `{app}_{YYYYMMDD}T{HHMMSS}.wav` (e.g., `zoom_20260324T143000.wav`)
- Location: configurable, default `%LOCALAPPDATA%/condor_audio/audio-taps/`
- Retention: keep last N chunks per tap (default 360 = 1 hour per app)
- Rotation: delete oldest chunk when limit exceeded
- Size: ~940 KB per 10s chunk at 48kHz/16-bit/mono

**Format conversion**: WASAPI shared-mode mix format is typically 32-bit float stereo. The capture thread downmixes to mono (average L+R) and converts to 16-bit PCM before writing. Transcription consumes the normalized 16-bit mono WAV, not the native device format. This keeps whisper-server input consistent regardless of the system's audio device configuration.

Use `hound` crate for WAV writing (single-purpose, well-maintained).

### State Model

```rust
struct TapRegistry {
    taps: HashMap<String, ActiveTap>,  // keyed by tap_id
}

struct ActiveTap {
    tap_id: String,
    app_name: String,           // "zoom" | "discord"
    target_pid: u32,
    include_tree: bool,
    started_at: DateTime<Utc>,
    chunks_written: u64,
    bytes_captured: u64,
    output_dir: PathBuf,
    status: TapStatus,          // Running | Paused | Stopped | Error(String)
}
```

Add `Arc<Mutex<TapRegistry>>` to `HttpState` in `http_api.rs`.

## Transcription Layer: whisper.cpp Server Sidecar

### Why whisper.cpp

| | Deepgram Nova-2 | whisper.cpp server | Groq Whisper Turbo |
|---|---|---|---|
| Latency | <300ms streaming | ~2-3s per 30s chunk | <1s batch |
| Cost | $0.0058/min | Free | $0.00067/min |
| Privacy | Cloud (SOC2, zero-retention) | **Fully local** | Cloud |
| Streaming | Yes | Sliding window (500ms) | Batch only |
| Diarization | Yes (streaming) | No | No |

**Phase 1: whisper.cpp server** — fully local, no cloud, no cost, good enough accuracy.

**Phase 1.5 upgrade path**: Swap to Deepgram streaming if accuracy or diarization is needed. Deepgram supports on-prem deployment for privacy-sensitive calls.

### Deployment

```bash
# Docker (WSL2)
docker run -d --name whisper-server \
  -p 8080:8080 \
  -v /data/whisper-models:/models \
  ghcr.io/ggerganov/whisper.cpp:main \
  --host 0.0.0.0 --port 8080 -m /models/ggml-base.en.bin

# Or systemd (matching swarm-coordinator pattern)
whisper-server --host 127.0.0.1 --port 8080 -m ./models/ggml-base.en.bin
```

Endpoint: `POST /inference` with multipart audio file.

Ref: [whisper.cpp server README](https://github.com/ggml-org/whisper.cpp/blob/master/examples/server/README.md)

### Transcription Pipeline

```
condor-eye (Windows)           whisper-server (WSL2)
┌──────────────┐               ┌──────────────┐
│ WASAPI tap   │──10s WAV──→   │ POST         │
│ per-process  │   chunks      │ /inference   │──→ transcript text
│ capture      │               │              │
└──────────────┘               └──────────────┘
                                      │
                                      ▼
                               /data/transcripts/{app}_{ts}.txt
```

### Transport Modes (two distinct architectures)

**Primary: Synchronous HTTP POST** (default, meets 30s SLA)

Condor-eye POSTs each completed 10s WAV chunk to `http://{WSL-IP}:8080/inference`. Whisper-server returns transcript text synchronously (~1-2s). Condor-eye stores the `.txt` alongside the WAV. Latency: 10s fill + 2s transcribe = 12s worst case.

Requires: stable WSL2 IP reachable from Windows. Use `host.docker.internal` if whisper runs in Docker, or the WSL2 bridge IP.

**Fallback: Async file-drop via Syncthing** (different latency, different failure semantics)

If WSL IP instability causes HTTP failures, switch to file-based: condor-eye writes WAV chunks to a Syncthing-synced folder. A watcher process on WSL picks up new WAVs, POSTs to whisper-server locally, writes `.txt` back. Syncthing is already deployed in home-lab docker-compose for `/data/` sync.

This mode changes the architecture from request/response to eventual consistency. Latency adds Syncthing propagation delay (~1-5s) on top of chunk + transcribe time. Acceptable as a degraded mode, not the primary path. The mode is configured via `AUDIO_TRANSPORT=http|file` env var, not auto-detected.

## HTTP API

All audio routes require `Authorization: Bearer {CAPTURE_TOKEN}` (same token gate as existing `/api/capture`).

```
GET  /api/condor_audio/status          — capability report + active taps summary
GET  /api/condor_audio/sessions        — enumerate Windows audio sessions with owning PIDs
POST /api/condor_audio/taps            — start a tap { app: "zoom"|"discord", pid?: number }
DELETE /api/condor_audio/taps/:id      — stop a tap
GET  /api/condor_audio/taps/:id        — tap status (bytes_captured, chunks_written, last_chunk_ts)
GET  /api/condor_audio/taps/:id/latest — download latest WAV chunk (binary)
GET  /api/condor_audio/transcripts     — list transcripts { since?: ISO8601, app?: string }
GET  /api/condor_audio/transcripts/:id — get transcript text
```

## Security Requirements (blockers — must ship with Phase 1)

1. **Auth**: Bearer token on all `/api/condor_audio/*` routes (reuse `CAPTURE_TOKEN` from existing capture endpoint)
2. **Separate listener for audio routes**: The existing condor-eye axum server binds `0.0.0.0:9050` (required for WSL2 access to capture routes). Axum does not support per-route bind addresses. Audio routes run on a **second axum listener** on `127.0.0.1:9051` (localhost-only by default). This is a separate `tokio::spawn` in `main.rs`, not a sidecar process — same binary, two listeners. Widen to `0.0.0.0` only via explicit `CONDOR_AUDIO_BIND` env var. The existing `:9050` server does NOT serve audio routes.
3. **Consent**: System tray notification + toast when any tap is active ("Recording Zoom audio")
4. **Encryption at rest**: Optional — encrypt WAV files with key from 1Password if `AUDIO_ENCRYPT=true`
5. **Auto-cleanup**: WAV chunks deleted after confirmed transcription + retention window

## MCP Tools (condor-eye MCP server addition)

```
condor_audio_status    — list active taps and capabilities
condor_audio_start     — start a tap for a target app
condor_audio_stop      — stop a tap
condor_audio_latest    — get latest transcript text from a tap
```

---

# Implementation Plan

## Phase 1: Capture + Transcribe (Step 1)

### 1.1 — Add audio module to condor-eye backend
- Add `wasapi` and `hound` to `Cargo.toml`
- Create `src-tauri/src/audio.rs` — replace current stubs with:
  - `enumerate_audio_sessions()` → list active Windows audio sessions with PIDs via `IAudioSessionManager2`
  - `start_tap(pid, include_tree, output_dir)` → spawn capture thread, return tap handle
  - `stop_tap(tap_id)` → signal capture thread to stop, finalize last chunk
  - Format conversion: downmix stereo float → mono 16-bit PCM in capture thread
- Create `src-tauri/src/audio_watcher.rs` — background task:
  - Poll active audio sessions every 5s (session-first, not process-first)
  - Match session PIDs to target apps via exe path
  - Auto-start tap only when matching **active audio session** found
  - Auto-stop when session goes inactive or process exits
- **Acceptance**: `cargo test` passes, audio sessions enumerable on Windows

### 1.2 — WAV chunk writing
- Capture thread writes 10s WAV chunks using `hound` (16-bit PCM, 48kHz, mono after downmix)
- Chunk rotation (delete oldest when > N per app)
- Expose `bytes_captured` counter for health monitoring
- **Acceptance**: WAV files appear in output dir when Zoom/Discord has an active audio session (not just a running process)

### 1.3 — HTTP API for audio control
- Add `TapRegistry` to `HttpState`
- Spawn **second axum listener** on `127.0.0.1:9051` for audio routes (separate from existing `:9050`)
- Wire routes: status, sessions, start, stop, latest, transcripts
- Apply `CAPTURE_TOKEN` auth to all audio routes
- **Acceptance**: `curl -H "Authorization: Bearer $TOKEN" localhost:9051/api/condor_audio/status` returns capability report

### 1.4 — System tray consent notification
- Show Windows toast notification when tap starts ("Recording Zoom audio")
- Tray icon indicator while any tap is active
- **Acceptance**: Visible notification appears when tap starts

### 1.5 — whisper-server sidecar
- Add whisper-server to home-lab docker-compose (or systemd)
- Download `ggml-base.en.bin` model (~140MB)
- Test: POST a WAV file, get transcript back
- **Acceptance**: `curl -F file=@test.wav localhost:8080/inference` returns transcript text

### 1.6 — Transcription pipeline wiring
- After each 10s chunk completes, condor-eye POSTs to whisper-server (synchronous HTTP mode)
- Store transcript as `{app}_{ts}.txt` alongside WAV
- Expose via `GET /api/condor_audio/transcripts` on `:9051`
- Implement `AUDIO_TRANSPORT=file` fallback mode (Syncthing async, different latency guarantees)
- **Acceptance**: Spoken words in a Zoom call become searchable text within 12 seconds worst-case (10s chunk fill + 2s transcribe)

### 1.7 — MCP tools
- Add `condor_audio_*` tools to `mcp/index.js`
- **Acceptance**: Claude Code can query "what was said in the last Zoom call?"

## Phase 2: Insight Extraction (future)

- Feed transcript chunks to Ollama for triage (is this actionable?)
- High-confidence items → Claude API for structured extraction
- Route to: brainstorm-ui nodes, coord messages, memory files

## Phase 3: Knowledge Management (future)

- Searchable transcript archive (TimescaleDB hypertable)
- Agent summaries ("what did I miss?")
- Project-mention detection → link to brainstorm trees

---

# Hive Provenance

Converged in 2 rounds + 1 evidence round. 5 agents (architect, skeptic, infra, domain, security).

Evidence round invalidated Round 2 "buy first" decision:
- Zoom AI Notes: no API ([Zoom Dev Forum](https://devforum.zoom.us/t/api-access-for-zoom-ai-companion-custom-ai-notetaker-transcripts-summaries-integration/135692))
- Craig bot: no download API ([craig.chat/faq](https://craig.chat/faq/))
- WASAPI via wasapi-rs: simpler than expected ([HEnquist/wasapi-rs](https://github.com/HEnquist/wasapi-rs))

Key references:
- [AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS — Microsoft Learn](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params)
- [Microsoft ApplicationLoopback sample](https://github.com/microsoft/windows-classic-samples/tree/main/Samples/ApplicationLoopback)
- [wasapi-rs loopback example](https://github.com/HEnquist/wasapi-rs/blob/master/examples/loopback.rs)
- [whisper.cpp server](https://github.com/ggml-org/whisper.cpp/blob/master/examples/server/README.md)
- [Deepgram pricing](https://deepgram.com/pricing) (Phase 1.5 upgrade path)
- [Deepgram self-hosted](https://developers.deepgram.com/docs/self-hosted-introduction) (privacy option)
