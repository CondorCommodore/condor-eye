# Condor Eye

**Give your AI agents the ability to see what's on screen.**

Condor Eye is a lightweight, transparent overlay app that captures screen regions and sends them to the Anthropic API for visual analysis. It exposes an HTTP API and an MCP server so Claude Code (or any agent) can take screenshots, describe UI content, locate windows, and annotate with a drawing pen — all programmatically.

Built with [Tauri 2](https://v2.tauri.app/) (Rust + WebView2). Runs natively on Windows.

---

## Features

- **Transparent overlay** — frameless, always-on-top window with a resizable capture frame. Whatever's visible through the frame is what gets captured.
- **AI-powered vision** — sends screenshots to the Anthropic API (Claude) and returns structured descriptions or JSON extractions.
- **HTTP API** — `POST /api/capture`, `POST /api/locate`, `GET /api/status` on port 9050. Any tool that can make HTTP requests can use Condor Eye.
- **MCP server** — exposes `condor_eye_capture`, `condor_eye_locate`, `condor_eye_windows`, and `condor_eye_status` as Claude Code tools via the [Model Context Protocol](https://modelcontextprotocol.io/).
- **Focus box** — draggable/resizable highlight region for focusing the AI's attention on a specific area within the capture frame.
- **Drawing pen** — freehand annotations powered by [perfect-freehand](https://github.com/steveruizok/perfect-freehand). Five color presets, pressure simulation, included in screenshots sent to the API.
- **Extraction profiles** — configurable JSON profiles for different capture scenarios (depth ladders, candlestick charts, quote screens, heatmaps).
- **Ground truth comparison** — optional Redis integration to compare AI extractions against known-good data.
- **Global hotkey** — `Ctrl+Shift+C` triggers capture from anywhere.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  WebView2 Frontend (src/)                           │
│  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐ │
│  │ Capture  │ │ Focus    │ │ Draw   │ │ Results  │ │
│  │ Frame    │ │ Box      │ │ Canvas │ │ Panel    │ │
│  └──────────┘ └──────────┘ └────────┘ └──────────┘ │
├─────────────────────────────────────────────────────┤
│  Rust Backend (src-tauri/)                          │
│  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐ │
│  │ capture  │ │ claude   │ │ http   │ │ compare  │ │
│  │ .rs      │ │ .rs      │ │ _api   │ │ .rs      │ │
│  │ Win32 GDI│ │ Anthropic│ │ axum   │ │ Diff     │ │
│  └──────────┘ └──────────┘ └────────┘ └──────────┘ │
├─────────────────────────────────────────────────────┤
│  MCP Server (mcp/)                                  │
│  Node.js stdio transport — wraps HTTP API           │
└─────────────────────────────────────────────────────┘
```

## Getting Started

### Prerequisites

- **Rust toolchain** on Windows (via [rustup](https://rustup.rs/))
- **Node.js** 18+ (for the MCP server)
- **Anthropic API key** — set `ANTHROPIC_API_KEY` in your environment or a `.env` file

### Build & Run

```bash
# Development mode
cargo tauri dev

# Production build
cargo tauri build

# Run tests
cd src-tauri && cargo test
```

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `ANTHROPIC_API_KEY` | Yes | — | Your Anthropic API key |
| `CLAUDE_MODEL` | No | `claude-haiku-4-5-20251001` | Model for vision calls |
| `CONDOR_EYE_PORT` | No | `9050` | HTTP API port |
| `CONDOR_EYE_BIND` | No | `0.0.0.0` | HTTP API bind address |
| `REDIS_URL` | No | `redis://127.0.0.1:6379` | Redis for ground truth comparison |

The app looks for `.env` files in several locations (first found wins):
1. Current working directory
2. Parent directory (for dev mode)
3. `%APPDATA%/Condor Eye/.env` (installed app)
4. Next to the executable

### WSL Users

If developing from WSL, use `cargo.exe` to invoke the Windows Rust toolchain:

```bash
cargo.exe tauri dev
cargo.exe tauri build
```

The HTTP API binds to `0.0.0.0` by default, so you can reach it from WSL at the Windows host IP (usually the WSL gateway).

## HTTP API

The app starts an [axum](https://github.com/tokio-rs/axum) HTTP server on port 9050.

### `POST /api/capture`

Capture a screen region and get an AI description.

```bash
curl -X POST http://localhost:9050/api/capture \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Describe what you see.", "include_image": true}'
```

**Parameters:**
- `prompt` — what to ask the AI about the screenshot
- `region` — `{x, y, width, height}` in pixels (omit to use the app's frame)
- `hwnd` — window handle to bring to foreground before capture
- `keys` — key combos to send before capture (e.g., `["ctrl+3"]` to switch tabs)
- `include_image` — return base64 PNG in response
- `profile` — extraction profile name (default: `depth`)

### `POST /api/locate`

Full-screen capture to find a window or UI element.

```bash
curl -X POST http://localhost:9050/api/locate \
  -H "Content-Type: application/json" \
  -d '{"query": "the Chrome browser window"}'
```

### `GET /api/status`

Health check and configuration.

```bash
curl http://localhost:9050/api/status
```

## MCP Server

The MCP server in `mcp/` wraps the HTTP API for Claude Code integration.

### Register globally

```bash
claude mcp add --scope user condor-eye -- node /path/to/mcp/index.js
```

### Available tools

| Tool | Description |
|------|-------------|
| `condor_eye_capture` | Capture + AI description of screen content |
| `condor_eye_locate` | Find a window or UI element on screen |
| `condor_eye_windows` | List visible windows (free, no API call) |
| `condor_eye_status` | Health check |

### `/screen` command

The repo includes a Claude Code [custom command](https://docs.anthropic.com/en/docs/claude-code/custom-commands) at `.claude/commands/screen.md`. After cloning, just type `/screen` in any Claude Code session:

| Command | Action |
|---------|--------|
| `/screen` | Capture the overlay frame region |
| `/screen firefox` | Find Firefox, bring to front, capture |
| `/screen chrome tab 3` | Focus Chrome, switch to tab 3, capture |
| `/screen full` | Capture the entire screen |

## Extraction Profiles

JSON files in `profiles/` define extraction behavior:

| Profile | Use case |
|---------|----------|
| `depth.json` | L2 depth / DOM ladder |
| `candle.json` | Candlestick chart OHLC |
| `quote.json` | Quote screen bid/ask/last |
| `heatmap.json` | Heatmap intensity |
| `custom.json` | User-editable template |

Profiles specify the extraction prompt, optional truth source for comparison, and comparison configuration.

## Cost

Condor Eye uses `claude-haiku-4-5-20251001` by default — the fastest and cheapest Claude model. A typical capture costs ~$0.003-0.005 in API usage. You can switch to a more capable model via the `CLAUDE_MODEL` environment variable.

## License

[MIT](LICENSE)
