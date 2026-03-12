# Condor Eye — Design Spec

**Date**: 2026-03-12
**Status**: Draft
**Author**: Mike + Claude (Aurora session)

## Overview

Condor Eye is an AI vision tool that gives Claude agents the ability to see what's on screen. It combines a Windows desktop app (Tauri 2) with an MCP server, enabling any Claude Code agent to capture, describe, and analyze screen content on demand.

The name "Condor Eye" reflects the CondorCommodore namespace and positions it as a potentially open-sourceable project with a novel capability: structured AI extraction from screen content with optional comparison against live application state.

## Problem

Claude Code agents are blind. They can read files, query APIs, and run commands, but they cannot see what applications are rendering on screen. For a trading platform with multiple visualization tools (heatmaps, DOM ladders, charts), there's no way for an agent to verify that what's displayed matches what the data pipeline says should be there.

## Solution

A two-component system:

1. **Condor Eye App** — Tauri 2 desktop app (Rust + WebView2) that captures screen regions, sends them to the Anthropic API (Condor Vision), and exposes results via HTTP API
2. **Condor Eye MCP Server** — Node.js MCP server that wraps the HTTP API, making capture/locate/status available as tools in Claude Code

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ Claude Code (WSL)                                     │
│                                                       │
│   Agent calls: condor_eye_capture(prompt="...")        │
│         ↓                                             │
│   MCP Server (Node.js, stdio transport)               │
│         ↓ HTTP POST                                   │
└─────────┬────────────────────────────────────────────┘
          │ localhost:9050 (or Tailscale IP for remote)
          ↓
┌──────────────────────────────────────────────────────┐
│ Condor Eye App (Tauri 2, Windows-native)              │
│                                                       │
│   HTTP API:                                           │
│     POST /api/capture  → screenshot + Condor Vision   │
│     POST /api/locate   → full screen + find window    │
│     GET  /api/status   → app state + overlay position │
│                                                       │
│   Tauri IPC (for manual UI use):                      │
│     capture_and_compare, capture_free, list_profiles  │
│                                                       │
│   Overlay UI: transparent frame, toolbar, results     │
└──────────────────────────────────────────────────────┘
```

### Transport Decisions

| Layer | Transport | Rationale |
|-------|-----------|-----------|
| Claude Code ↔ MCP Server | stdio | Standard for Claude Code plugins, launched as child process |
| MCP Server ↔ Condor Eye App | HTTP (port 9050) | Works across WSL↔Windows, future Tailscale for remote machines |
| Condor Eye App ↔ Anthropic API | HTTPS | Direct from Rust, API key stays in one place |

### Key Principle

Condor Vision (AI analysis) calls happen **in the Tauri app**, not the MCP server. This keeps the API key in one place, avoids double-hop latency, and means the MCP server is a stateless protocol adapter.

## MCP Tool Surface

Three tools, all with optional `host` parameter (defaults to `localhost:9050`, accepts Tailscale IP for remote machines):

### `condor_eye_capture`

Primary tool. Screenshots a region and returns AI description.

**Parameters:**
- `prompt` (string, optional) — What to look for. Default: "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible."
- `region` (object, optional) — `{x, y, width, height}` in pixels. Region fallback: explicit `region` parameter > overlay window position > full screen capture.
- `host` (string, optional) — Target machine. Default: `localhost:9050`.
- `include_image` (boolean, optional) — Include base64 image in response. Default: false.

**Returns:**
```json
{
  "description": "AI analysis text...",
  "latency_ms": 3200,
  "region": {"x": 100, "y": 200, "width": 800, "height": 600},
  "cost_estimate_usd": 0.003,
  "model": "claude-haiku-4-5-20251001"
}
```

> The MCP server strips the base64 image by default to avoid bloating the agent context. The HTTP API always returns it. Pass `include_image: true` to the MCP tool to include it.

### `condor_eye_locate`

Finds a window or UI element on screen. Full-screen capture, asks Condor Vision to identify the target and return its bounding box.

**Parameters:**
- `target` (string, required) — Description of what to find. E.g., "Thinkorswim Active Trader window", "the ES heatmap", "Chrome browser showing Grafana".
- `host` (string, optional)

**Returns:**
```json
{
  "found": true,
  "bounds": {"x": 120, "y": 50, "width": 800, "height": 600},
  "confidence": "high",
  "description": "Found Thinkorswim Active Trader window showing /ES depth ladder"
}
```

**Not found:**
```json
{
  "found": false,
  "bounds": null,
  "confidence": "none",
  "description": "Could not find a Thinkorswim window on screen"
}
```

> Confidence levels: `high`, `medium`, `low`, `none`

### `condor_eye_status`

Health check and current state.

**Parameters:**
- `host` (string, optional)

**Returns:**
```json
{
  "running": true,
  "version": "0.2.0",
  "overlay": {"x": 100, "y": 200, "width": 400, "height": 700, "visible": true},
  "api_key_configured": true,
  "cost_estimate_usd": 0.003,
  "model": "claude-haiku-4-5-20251001"
}
```

## Multi-Step Capture Pattern

For detailed analysis of a specific window:

```
1. condor_eye_locate(target="TOS Active Trader for /ES")
   → bounds: {x: 120, y: 50, w: 800, h: 600}

2. condor_eye_capture(
     region: {x: 120, y: 50, width: 800, height: 600},
     prompt: "Read all price levels and volumes from this depth ladder"
   )
   → detailed structured description of the DOM/ladder
```

This pattern works across any machine running Condor Eye — just change the `host` parameter.

## HTTP API Spec (Tauri App)

### POST /api/capture

```json
// Request
{
  "prompt": "Describe what you see...",
  "region": {"x": 100, "y": 200, "width": 800, "height": 600}  // optional
}

// Response
{
  "image": "<base64 PNG>",
  "description": "...",
  "latency_ms": 3200,
  "region": {"x": 100, "y": 200, "width": 800, "height": 600},
  "cost_estimate_usd": 0.003
}
```

If `region` is omitted, captures the current overlay window position. If the overlay is hidden or minimized, captures full screen.

### POST /api/locate

```json
// Request
{
  "target": "Thinkorswim Active Trader window"
}

// Response
{
  "found": true,
  "bounds": {"x": 120, "y": 50, "width": 800, "height": 600},
  "confidence": "high",
  "description": "Found Thinkorswim Active Trader..."
}
```

Always captures full screen, sends to Condor Vision with a locate-specific prompt.

### GET /api/status

```json
{
  "running": true,
  "version": "0.2.0",
  "overlay": {"x": 100, "y": 200, "width": 400, "height": 700, "visible": true},
  "api_key_configured": true,
  "model": "claude-haiku-4-5-20251001"
}
```

## File Structure

```
~/code/dev-tools/visual-validator/     # Future: rename to condor-eye
├── app/                              # Tauri 2 app
│   ├── src/
│   │   ├── main.rs                   # Tauri setup + HTTP server
│   │   ├── capture.rs                # Screen capture (Win32 GDI via screenshots crate)
│   │   ├── vision.rs                 # Condor Vision API calls (Anthropic API)
│   │   ├── compare.rs                # Comparison engine (for manual UI use)
│   │   ├── config.rs                 # Config, profiles, tick sizes
│   │   ├── truth.rs                  # Redis truth snapshots (for manual UI use)
│   │   └── http_api.rs              # HTTP API: /api/capture, /api/locate, /api/status
│   │                                # Delegates to existing capture.rs and vision.rs — no duplication of capture/Vision logic
│   ├── Cargo.toml
│   └── .env                         # ANTHROPIC_API_KEY (gitignored)
├── ui/                               # Overlay frontend
│   ├── index.html
│   ├── style.css
│   └── app.js
├── mcp/                              # MCP server
│   ├── package.json
│   ├── index.js                      # Entry point (stdio transport)
│   └── tools.js                      # Tool definitions + HTTP client
├── profiles/                         # Extraction profiles (for manual UI use)
│   ├── depth.json
│   ├── candle.json
│   ├── quote.json
│   ├── heatmap.json
│   └── custom.json
├── CLAUDE.md                         # Project docs
├── .gitignore
└── README.md
```

## Implementation Phases

### Phase 1: Agent On-Demand (this spec)
- Add HTTP API to Tauri app (3 endpoints)
- Build MCP server (Node.js, ~150 lines)
- Register as Claude Code plugin
- Agent can call `condor_eye_capture` to see the screen

### Phase 2: Agent-Initiated Validation
- Agent calls capture during pipeline operations
- "Start L2 services, then verify the heatmap shows data"
- Uses existing compare/truth infrastructure through the UI or custom prompts

### Phase 3: Continuous Monitoring
- Add `condor_eye_watch` tool — periodic capture with change detection
- Only sends to Condor Vision when pixel content changes significantly
- Cost-controlled: configurable interval, pixel-diff gating

### Phase 4: Remote Viewing + Discord
- Expose HTTP API via Tailscale (Aurora: 100.70.34.55)
- Any agent on any lab machine can capture from any other machine
- Chain with Discord MCP: user says "show me ES heatmap" → capture → post to Discord
- Multi-machine fleet: each machine runs Condor Eye, agents coordinate via `host` parameter

## Security Considerations

- HTTP API binds to `127.0.0.1:9050` by default (localhost only)
- Remote access requires explicit config: `CONDOR_EYE_BIND=0.0.0.0` + Tailscale for auth
- API key stays in the Tauri app's `.env`, never exposed through MCP or HTTP
- MCP server runs as Claude Code child process — inherits session permissions
- No screenshot data is persisted by default (in-memory only)
- Phase 4 remote access requires a `CONDOR_EYE_API_TOKEN` bearer token checked by HTTP middleware. Localhost requests bypass auth.

## Timeouts

MCP server HTTP client: 60s. Tauri app Anthropic API: 30s (existing). Capture requests are serialized (one at a time) to prevent concurrent screen captures.

## Dependencies

### Tauri App (Rust)
- Existing: tauri 2, screenshots 0.8, image 0.25, reqwest, serde, tokio, redis, base64, dotenvy
- New: axum (HTTP server) or tiny-http (simpler)

### MCP Server (Node.js)
- `@modelcontextprotocol/sdk` — MCP protocol implementation. Uses Node.js built-in `fetch` (v22+).

## Open Source Strategy

**Public repo**: `CondorCommodore/condor-eye`
**License**: MIT

**What to open-source:**
- Tauri app framework (capture, HTTP API, overlay UI)
- MCP server (all tools)
- Generic extraction profiles
- README with setup instructions

**What stays private:**
- Trading-specific extraction prompts
- Redis stream schemas for market data
- Performance tuning for market data timing
- `.env` with API keys

## Success Criteria

1. From a Claude Code session, `condor_eye_capture()` returns a screenshot + description within 5 seconds
2. `condor_eye_locate("Chrome browser")` correctly identifies window bounds
3. Works from WSL calling Windows-side Tauri app
4. Works remotely via Tailscale IP (Phase 4)
5. MCP server installs as a Claude Code plugin with zero config beyond API key
