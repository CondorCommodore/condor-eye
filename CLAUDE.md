# CLAUDE.md

## Project

Condor Eye (formerly Visual Validator) — a Tauri 2 app (Rust + WebView2) that gives Claude agents the ability to see what's on screen. Captures screen regions, sends to Anthropic API (Condor Vision) for analysis, and optionally compares against Redis ground truth data. Exposes an HTTP API for programmatic access and an MCP server for Claude Code integration.

## Build and Run

**Prerequisite**: Rust toolchain installed on Windows (not WSL). The app runs natively on Windows.

    # Development (from WSL — calls Windows Rust toolchain)
    cd ~/code/dev-tools/condor-eye
    cargo.exe tauri dev

    # Build release
    cargo.exe tauri build

    # Run tests (pure logic — no display needed)
    cd src-tauri && cargo.exe test

## Architecture

    capture.rs   — screen capture (screenshots crate, Win32 GDI) + capture_full_screen()
    claude.rs    — Condor Vision API: extract_from_screenshot() (JSON) + describe_screenshot() (text)
    http_api.rs  — axum HTTP server: /api/capture, /api/locate, /api/status (port 9050)
    truth.rs     — Redis ground truth snapshots (market.depth stream)
    compare.rs   — Diff engine: extracted vs truth -> ComparisonReport
    config.rs    — AppConfig, profiles, tick sizes, cost estimation
    main.rs      — Tauri setup, IPC commands, AppState, HTTP server launch
    mcp/index.js — Node.js MCP server (stdio transport, wraps HTTP API)

Frontend is vanilla HTML/CSS/JS in src/. Transparent frameless always-on-top window.

## Condor Eye HTTP API (port 9050)

The Tauri app embeds an axum HTTP server for programmatic access. Starts automatically with the app.

    POST /api/capture  — screenshot + Condor Vision description
    POST /api/locate   — full screen capture, find a window/element, return bounds
    GET  /api/status   — health check + config

Captures are serialized (one at a time) via tokio::sync::Mutex.

## MCP Server

Node.js MCP server in `mcp/` wraps the HTTP API as 3 Claude Code tools:
- `condor_eye_capture` — capture + describe screen content
- `condor_eye_locate` — find a window/element on screen
- `condor_eye_status` — health check

Registered globally: `claude mcp add --scope user condor-eye -- node /path/to/mcp/index.js`

The MCP server auto-detects WSL gateway IP for Windows host routing. Requires Windows Firewall rule for port 9050 (one-time setup).

## Extraction Profiles

JSON files in profiles/ define extraction behavior:
- depth.json — L2 depth/DOM ladder (default)
- candle.json — Candlestick chart OHLC
- quote.json — Quote screen bid/ask/last
- heatmap.json — L2 surface heatmap intensity
- custom.json — User-editable template

Profiles specify: extraction prompt, truth source (Redis stream, file, none), comparison config.

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
- Redis connectivity: App runs on Windows, connects to Redis in WSL2 via localhostForwarding.

## Testing

    # All tests
    cd src-tauri && cargo.exe test

    # Specific module
    cargo.exe test compare::tests
    cargo.exe test config::tests
    cargo.exe test claude::tests

    # Integration test (requires Redis with live data)
    REDIS_URL=redis://127.0.0.1:6379 cargo.exe test truth::tests::snapshot_real -- --ignored
