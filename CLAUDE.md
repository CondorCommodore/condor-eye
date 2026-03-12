# CLAUDE.md

## Project

Visual Validator — a Tauri 2 app (Rust + WebView2) for comparing what a trading display shows on screen against known ground truth data. Uses Claude Vision API (Haiku 4.5) to extract structured data from screenshots.

## Build and Run

**Prerequisite**: Rust toolchain installed on Windows (not WSL). The app runs natively on Windows.

    # Development (from WSL — calls Windows Rust toolchain)
    cd ~/code/dev-tools/visual-validator
    cargo.exe tauri dev

    # Build release
    cargo.exe tauri build

    # Run tests (pure logic — no display needed)
    cd src-tauri && cargo.exe test

## Architecture

    capture.rs  — screen capture (screenshots crate, Win32 GDI)
    claude.rs   — Claude Vision API extraction (async reqwest)
    truth.rs    — Redis ground truth snapshots (market.depth stream)
    compare.rs  — Diff engine: extracted vs truth -> ComparisonReport
    config.rs   — AppConfig, profiles, tick sizes, cost estimation
    main.rs     — Tauri setup, IPC commands, AppState

Frontend is vanilla HTML/CSS/JS in src/. Transparent frameless always-on-top window.

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
