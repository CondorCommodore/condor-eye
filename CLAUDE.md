# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Condor Eye — a Tauri 2 app (Rust + WebView2) that gives AI agents the ability to see what's on screen and hear application audio. Captures screen regions, sends to Anthropic API for visual analysis, and optionally compares against Redis ground truth data. Includes a WASAPI-based audio tap system for capturing per-application audio and routing it to Whisper for transcription. Exposes HTTP APIs for programmatic access and an MCP server for Claude Code integration.

## Build and Run

**Prerequisite**: Rust toolchain + Windows. Audio tap requires WASAPI (Windows only).

    # Development
    cargo tauri dev

    # Build release
    cargo tauri build

    # Run tests (pure logic — no display needed)
    cd src-tauri && cargo test

    # Windows batch (sets up VS env automatically)
    build.bat

    # MCP server dependencies
    cd mcp && npm install

## Architecture

### Rust backend (src-tauri/src/)

    main.rs          — Tauri setup, IPC commands, AppState, launches both HTTP servers
    capture.rs       — screen capture (screenshots crate, Win32 GDI) + capture_full_screen()
    claude.rs        — Anthropic Vision API: extract_from_screenshot() (JSON) + describe_screenshot() (text)
    http_api.rs      — axum HTTP servers: vision API (port 9050) + audio API (port 9051)
    windows.rs       — Win32 FFI: enumerate visible windows, WindowInfo struct
    audio.rs         — WASAPI audio tap: per-app loopback capture, WAV chunking, Whisper transcription
    audio_watcher.rs — background session monitor, auto-discovers audio sessions
    truth.rs         — Redis ground truth snapshots
    compare.rs       — diff engine: extracted vs truth -> ComparisonReport
    config.rs        — AppConfig from env vars, profiles, tick sizes, cost estimation

### Frontend

    src/             — main UI: vanilla HTML/CSS/JS, transparent frameless always-on-top window
    audio-mini-ui/   — standalone audio tap control panel (served by audio HTTP server at /)

### Other

    mcp/index.js     — Node.js MCP server (stdio transport, wraps both HTTP APIs)
    profiles/        — extraction profile JSON files (depth, candle, quote, heatmap, custom)
    docs/            — design docs and specs for the audio subsystem

## Vision HTTP API (port 9050)

Axum server embedded in the Tauri app. Starts automatically.

    GET  /api/status      — health check + config
    POST /api/capture     — screenshot + AI description
    POST /api/locate      — full screen capture, find a window/element, return bounds
    GET  /api/windows     — list visible top-level windows (optional ?query= filter)
    GET  /api/vision      — proxy pass-through to external vision service
    POST /api/screenshot  — raw screenshot capture
    GET  /api/grid        — load saved grid layout
    POST /api/grid        — save grid layout

Captures are serialized (one at a time) via tokio::sync::Mutex.

## Audio HTTP API (port 9051)

Separate axum server for audio tap management. Requires CAPTURE_TOKEN auth header.

    GET    /api/condor_audio/status                          — audio subsystem health
    GET    /api/condor_audio/sessions                        — list active audio sessions
    POST   /api/condor_audio/taps                            — start a new audio tap
    GET    /api/condor_audio/taps/{tap_id}                   — get tap info
    DELETE /api/condor_audio/taps/{tap_id}                   — stop a tap
    GET    /api/condor_audio/taps/{tap_id}/latest            — latest audio chunk
    GET    /api/condor_audio/taps/{tap_id}/latest-transcript — latest transcript
    GET    /api/condor_audio/transcripts                     — list all transcripts
    GET    /api/condor_audio/transcripts/{transcript_id}     — get specific transcript

## MCP Server

Node.js MCP server in `mcp/` wraps both HTTP APIs as Claude Code tools:

**Vision tools:**
- `condor_eye_capture` — capture + describe screen content
- `condor_eye_locate` — find a window/element on screen
- `condor_eye_windows` — list visible windows
- `condor_eye_status` — health check

**Audio tools:**
- `condor_audio_status` — audio subsystem health + active taps
- `condor_audio_start` — start an audio tap for a target app
- `condor_audio_stop` — stop an active tap
- `condor_audio_latest` — fetch latest transcript for a tap

Register globally: `claude mcp add --scope user condor-eye -- node /path/to/mcp/index.js`

## Extraction Profiles

JSON files in profiles/ define extraction behavior:
- depth.json — L2 depth/DOM ladder (default)
- candle.json — Candlestick chart OHLC
- quote.json — Quote screen bid/ask/last
- heatmap.json — L2 surface heatmap intensity
- custom.json — User-editable template

Profiles specify: extraction prompt, truth source, comparison config.

## Environment Variables

### Vision (required)

| Variable | Required | Default |
|---|---|---|
| ANTHROPIC_API_KEY | Yes (if vision enabled) | (none) |
| CONDOR_VISION_ENABLED | No | false — captures skip the AI call to save costs |
| REDIS_URL | No | redis://127.0.0.1:6379 |
| CLAUDE_MODEL | No | claude-haiku-4-5-20251001 |
| CONDOR_EYE_BIND | No | 0.0.0.0 |
| CONDOR_EYE_PORT | No | 9050 |

### Audio

| Variable | Default |
|---|---|
| CONDOR_AUDIO_BIND | 127.0.0.1 |
| CONDOR_AUDIO_PORT | 9051 |
| CONDOR_AUDIO_OUTPUT_DIR | %LOCALAPPDATA%/condor_audio/audio-taps |
| CONDOR_AUDIO_CHUNK_SECONDS | 10 |
| CONDOR_AUDIO_STITCH_MS | 1500 |
| CONDOR_AUDIO_AUTO_WATCH | false |
| CONDOR_AUDIO_ARCHIVE | true |
| AUDIO_TRANSPORT | http |
| CAPTURE_TOKEN | (none — audio API returns 403 if unset) |
| WHISPER_URL | http://localhost:8080/inference |

### Integration

| Variable | Default |
|---|---|
| DISCORD_BRIDGE_URL | (none, optional) |
| COORD_API_URL | http://localhost:8800 |
| COORD_API_TOKEN | (none) |
| CONDOR_INTEL_URL | http://localhost:8791 |

## Key Patterns

- DPI scaling: outer_position()/outer_size() return physical pixels already. Do NOT multiply by scale_factor().
- Blocking I/O: Screen capture and Redis use tokio::task::spawn_blocking. Claude API is async.
- Frame hiding: Use window.set_opacity(0.0) before capture, set_opacity(1.0) after. Avoids Z-order thrashing vs hide()/show().
- Corner resize: WebView2's startResizeDragging rejects compound corner directions. Corners use manual pointer-tracked resize; edges use native Tauri resize.
- Audio tap auth: The audio API requires a CAPTURE_TOKEN header. If unset, all audio endpoints return 403.

## Testing

    # All tests
    cd src-tauri && cargo test

    # Specific module
    cargo test compare::tests
    cargo test config::tests
    cargo test claude::tests
