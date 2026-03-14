# CLAUDE.md

## Project

Condor Eye — a Tauri 2 app (Rust + WebView2) that gives AI agents the ability to see what's on screen. Captures screen regions, sends to Anthropic API for visual analysis, and optionally compares against Redis ground truth data. Exposes an HTTP API for programmatic access and an MCP server for Claude Code integration.

## Build and Run

**Prerequisite**: Rust toolchain installed on Windows. The app runs natively on Windows.

    # Development
    cargo tauri dev

    # Build release
    cargo tauri build

    # Run tests (pure logic — no display needed)
    cd src-tauri && cargo test

## Architecture

    capture.rs   — screen capture (screenshots crate, Win32 GDI) + capture_full_screen()
    claude.rs    — Anthropic Vision API: extract_from_screenshot() (JSON) + describe_screenshot() (text)
    http_api.rs  — axum HTTP server: /api/capture, /api/locate, /api/status (port 9050)
    truth.rs     — Redis ground truth snapshots
    compare.rs   — Diff engine: extracted vs truth -> ComparisonReport
    config.rs    — AppConfig, profiles, tick sizes, cost estimation
    main.rs      — Tauri setup, IPC commands, AppState, HTTP server launch
    mcp/index.js — Node.js MCP server (stdio transport, wraps HTTP API)

Frontend is vanilla HTML/CSS/JS in src/. Transparent frameless always-on-top window.

## HTTP API (port 9050)

The Tauri app embeds an axum HTTP server for programmatic access. Starts automatically with the app.

    POST /api/capture  — screenshot + AI description
    POST /api/locate   — full screen capture, find a window/element, return bounds
    GET  /api/status   — health check + config

Captures are serialized (one at a time) via tokio::sync::Mutex.

## MCP Server

Node.js MCP server in `mcp/` wraps the HTTP API as Claude Code tools:
- `condor_eye_capture` — capture + describe screen content
- `condor_eye_locate` — find a window/element on screen
- `condor_eye_windows` — list visible windows
- `condor_eye_status` — health check

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

| Variable | Required | Default |
|---|---|---|
| ANTHROPIC_API_KEY | Yes | (none) |
| REDIS_URL | No | redis://127.0.0.1:6379 |
| CLAUDE_MODEL | No | claude-haiku-4-5-20251001 |
| CONDOR_EYE_BIND | No | 0.0.0.0 |
| CONDOR_EYE_PORT | No | 9050 |

## Key Patterns

- DPI scaling: outer_position()/outer_size() return physical pixels already. Do NOT multiply by scale_factor().
- Blocking I/O: Screen capture and Redis use tokio::task::spawn_blocking. Claude API is async.
- Frame hiding: Use window.set_opacity(0.0) before capture, set_opacity(1.0) after. Avoids Z-order thrashing vs hide()/show().
- Corner resize: WebView2's startResizeDragging rejects compound corner directions. Corners use manual pointer-tracked resize; edges use native Tauri resize.

## Testing

    # All tests
    cd src-tauri && cargo test

    # Specific module
    cargo test compare::tests
    cargo test config::tests
    cargo test claude::tests
