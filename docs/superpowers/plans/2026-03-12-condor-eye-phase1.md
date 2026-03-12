# Condor Eye Phase 1 — Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an HTTP API to the Tauri app and build a Node.js MCP server so Claude Code agents can capture and analyze screen content on demand.

**Architecture:** The Tauri app gets an embedded axum HTTP server (port 9050) exposing 3 endpoints (`/api/capture`, `/api/locate`, `/api/status`). A Node.js MCP server (stdio transport) wraps these endpoints as MCP tools. The MCP server is a stateless protocol adapter — all AI vision calls happen in the Tauri app.

**Tech Stack:** Rust (axum for HTTP), Node.js 22 (`@modelcontextprotocol/sdk`), existing Tauri 2 app infrastructure.

**Spec:** `docs/superpowers/specs/2026-03-12-condor-eye-design.md`

---

## File Structure

### Files to create:
- `src-tauri/src/http_api.rs` — axum HTTP server with 3 endpoints, shared state, capture serialization
- `mcp/package.json` — MCP server dependencies
- `mcp/index.js` — MCP server entry point (stdio transport, tool handlers)

### Files to modify:
- `src-tauri/Cargo.toml` — Add `axum` dependency
- `src-tauri/src/main.rs` — Add `mod http_api`, launch HTTP server in `setup()`
- `src-tauri/src/capture.rs` — Add `capture_full_screen()` function
- `src-tauri/src/claude.rs` — Add `describe_screenshot()` function (free-form text, no JSON parsing)
- `~/.claude/settings.json` — Register MCP server
- `~/code/ports.json` — Register port 9050

### Files unchanged:
- `src-tauri/src/compare.rs` — Not needed for Phase 1 HTTP API
- `src-tauri/src/truth.rs` — Not needed for Phase 1 HTTP API
- `src-tauri/src/config.rs` — Read-only (used for `estimate_cost`, `AppConfig`)
- `src/index.html`, `src/app.js`, `src/style.css` — Overlay UI unchanged

---

## Chunk 1: Tauri App HTTP API

### Task 1: Add axum dependency and Region type

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/capture.rs`

- [ ] **Step 1: Add axum to Cargo.toml**

In `src-tauri/Cargo.toml`, add `axum` to `[dependencies]`:

```toml
axum = "0.8"
```

- [ ] **Step 2: Add Region struct and capture_full_screen to capture.rs**

Add after the existing `CaptureError` Display impl, before `capture_region`:

```rust
/// Screen region in physical pixels.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Capture the entire primary screen as PNG bytes.
/// Returns (png_bytes, region) where region is the full screen dimensions.
pub fn capture_full_screen() -> Result<(Vec<u8>, Region), CaptureError> {
    let screens = Screen::all().map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;
    let screen = screens.into_iter().next().ok_or(CaptureError::NoScreen)?;
    let di = screen.display_info;
    let region = Region {
        x: di.x,
        y: di.y,
        width: di.width,
        height: di.height,
    };

    let full = screen
        .capture()
        .map_err(|e| CaptureError::ScreenshotFailed(e.to_string()))?;

    let w = full.width();
    let h = full.height();
    let rgba_data = full.into_raw();
    let rgba_img = RgbaImage::from_raw(w, h, rgba_data)
        .ok_or(CaptureError::EncodeFailed("Failed to create RGBA image".into()))?;
    let img = DynamicImage::ImageRgba8(rgba_img);

    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| CaptureError::EncodeFailed(e.to_string()))?;

    Ok((buf, region))
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd ~/code/dev-tools/visual-validator && cargo.exe check 2>&1 | tail -5`
Expected: compiles without errors (warnings OK)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/capture.rs
git commit -m "feat(condor-eye): add axum dep and capture_full_screen"
```

---

### Task 2: Add describe_screenshot to claude.rs

**Files:**
- Modify: `src-tauri/src/claude.rs`

The existing `extract_from_screenshot` returns a parsed `ExtractionResult` (structured JSON). The HTTP API needs a free-form text description. Add a simpler function.

- [ ] **Step 1: Add describe_screenshot function**

Add after `extract_from_screenshot`, before `parse_extraction`:

```rust
/// Send a screenshot to Claude for free-form description.
///
/// Unlike `extract_from_screenshot`, this returns raw text — no JSON parsing.
/// Used by the HTTP API for generic capture requests.
pub async fn describe_screenshot(
    api_key: &str,
    png_bytes: &[u8],
    model: &str,
    prompt: &str,
) -> Result<String, ExtractionError> {
    let client = Client::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2000,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": b64,
                    }
                },
                {
                    "type": "text",
                    "text": prompt,
                }
            ]
        }]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ExtractionError::RateLimit);
        }
        return Err(ExtractionError::Api(format!("{}: {}", status, text)));
    }

    let api_resp: serde_json::Value = resp.json().await?;
    let content = api_resp["content"][0]["text"]
        .as_str()
        .ok_or(ExtractionError::Parse("No text in response".into()))?
        .to_string();

    Ok(content)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd ~/code/dev-tools/visual-validator && cargo.exe check 2>&1 | tail -5`
Expected: compiles without errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/claude.rs
git commit -m "feat(condor-eye): add describe_screenshot for free-form vision"
```

---

### Task 3: Create http_api.rs — shared state + /api/status

**Files:**
- Create: `src-tauri/src/http_api.rs`
- Modify: `src-tauri/src/main.rs` (add `mod http_api`)

- [ ] **Step 1: Create http_api.rs with status endpoint**

Create `src-tauri/src/http_api.rs`:

```rust
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::capture::{self, Region};
use crate::claude;
use crate::config::{self, AppConfig};

/// Shared state for the HTTP API server.
pub struct HttpState {
    pub config: AppConfig,
    pub capture_lock: Mutex<()>,
}

// ── Request/Response types ──

#[derive(Deserialize)]
pub struct CaptureRequest {
    pub prompt: Option<String>,
    pub region: Option<Region>,
}

#[derive(Serialize)]
pub struct CaptureResponse {
    pub image: String,
    pub description: String,
    pub latency_ms: u64,
    pub region: Region,
    pub cost_estimate_usd: f64,
}

#[derive(Deserialize)]
pub struct LocateRequest {
    pub target: String,
}

#[derive(Serialize, Deserialize)]
pub struct LocateResponse {
    pub found: bool,
    pub bounds: Option<Region>,
    pub confidence: String,
    pub description: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub running: bool,
    pub version: String,
    pub api_key_configured: bool,
    pub model: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// ── Handlers ──

async fn handle_status(State(state): State<Arc<HttpState>>) -> Json<StatusResponse> {
    Json(StatusResponse {
        running: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        api_key_configured: !state.config.api_key.is_empty(),
        model: state.config.model.clone(),
    })
}

async fn handle_capture(
    State(state): State<Arc<HttpState>>,
    Json(req): Json<CaptureRequest>,
) -> Result<Json<CaptureResponse>, (StatusCode, Json<ErrorResponse>)> {
    let prompt = req.prompt.unwrap_or_else(|| {
        "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible.".to_string()
    });

    // Serialize captures — one at a time
    let _guard = state.capture_lock.lock().await;

    // Capture screen
    let (png, region) = if let Some(r) = req.region {
        let rx = r.x;
        let ry = r.y;
        let rw = r.width;
        let rh = r.height;
        let png = tokio::task::spawn_blocking(move || {
            capture::capture_region(rx, ry, rw, rh)
        })
        .await
        .map_err(|e| api_error(format!("Task join: {}", e)))?
        .map_err(|e| api_error(format!("Capture: {}", e)))?;
        (png, r)
    } else {
        // Full screen capture
        tokio::task::spawn_blocking(capture::capture_full_screen)
            .await
            .map_err(|e| api_error(format!("Task join: {}", e)))?
            .map_err(|e| api_error(format!("Capture: {}", e)))?
    };

    eprintln!("[CE] captured {} bytes, region: {:?}", png.len(), region);

    // Send to Condor Vision
    let start = std::time::Instant::now();
    let description = claude::describe_screenshot(
        &state.config.api_key,
        &png,
        &state.config.model,
        &prompt,
    )
    .await
    .map_err(|e| api_error(format!("Vision: {}", e)))?;
    let latency_ms = start.elapsed().as_millis() as u64;

    let cost = config::estimate_cost(region.width, region.height, &state.config.model);
    let image = base64::engine::general_purpose::STANDARD.encode(&png);

    eprintln!("[CE] capture response: {}ms, {} chars", latency_ms, description.len());

    Ok(Json(CaptureResponse {
        image,
        description,
        latency_ms,
        region,
        cost_estimate_usd: cost,
    }))
}

async fn handle_locate(
    State(state): State<Arc<HttpState>>,
    Json(req): Json<LocateRequest>,
) -> Result<Json<LocateResponse>, (StatusCode, Json<ErrorResponse>)> {
    let _guard = state.capture_lock.lock().await;

    // Always full-screen for locate
    let (png, _screen_region) = tokio::task::spawn_blocking(capture::capture_full_screen)
        .await
        .map_err(|e| api_error(format!("Task join: {}", e)))?
        .map_err(|e| api_error(format!("Capture: {}", e)))?;

    eprintln!("[CE] locate: full screen captured, {} bytes, target: {}", png.len(), req.target);

    let prompt = format!(
        "You are a screen analysis assistant. Look at this screenshot and find: {}\n\n\
         Return ONLY a JSON object (no markdown fences) with these fields:\n\
         - \"found\": boolean — whether you found the target\n\
         - \"bounds\": object with {{\"x\", \"y\", \"width\", \"height\"}} in pixels, or null if not found\n\
         - \"confidence\": one of \"high\", \"medium\", \"low\", \"none\"\n\
         - \"description\": brief description of what you found or why you couldn't find it\n\n\
         Estimate pixel coordinates based on the image dimensions. Be as accurate as possible.",
        req.target
    );

    let start = std::time::Instant::now();
    let raw = claude::describe_screenshot(
        &state.config.api_key,
        &png,
        &state.config.model,
        &prompt,
    )
    .await
    .map_err(|e| api_error(format!("Vision: {}", e)))?;
    let latency_ms = start.elapsed().as_millis() as u64;

    eprintln!("[CE] locate response ({}ms): {}", latency_ms, &raw[..raw.len().min(200)]);

    // Parse the JSON response
    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    match serde_json::from_str::<LocateResponse>(cleaned) {
        Ok(resp) => Ok(Json(resp)),
        Err(_) => {
            // If parsing fails, return a best-effort response
            Ok(Json(LocateResponse {
                found: false,
                bounds: None,
                confidence: "none".to_string(),
                description: format!("Failed to parse locate response. Raw: {}", &raw[..raw.len().min(300)]),
            }))
        }
    }
}

fn api_error(msg: String) -> (StatusCode, Json<ErrorResponse>) {
    eprintln!("[CE] ERROR: {}", msg);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: msg }),
    )
}

// ── Server startup ──

pub async fn start_server(config: AppConfig, bind_addr: String, port: u16) {
    let state = Arc::new(HttpState {
        config,
        capture_lock: Mutex::new(()),
    });

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/capture", post(handle_capture))
        .route("/api/locate", post(handle_locate))
        .with_state(state);

    let addr = format!("{}:{}", bind_addr, port);
    eprintln!("[CE] HTTP API starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect(&format!("Failed to bind to {}", addr));

    axum::serve(listener, app)
        .await
        .expect("HTTP server error");
}
```

- [ ] **Step 2: Add mod http_api to main.rs**

In `src-tauri/src/main.rs`, add after the existing `mod truth;` line:

```rust
mod http_api;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd ~/code/dev-tools/visual-validator && cargo.exe check 2>&1 | tail -5`
Expected: compiles without errors

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/http_api.rs src-tauri/src/main.rs
git commit -m "feat(condor-eye): add HTTP API module with status/capture/locate endpoints"
```

---

### Task 4: Wire HTTP server into Tauri startup

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Launch HTTP server in setup()**

In `main.rs`, inside the `.setup(|app| { ... })` closure, add **before** the `Ok(())` line:

```rust
            // Start Condor Eye HTTP API server
            let ce_config = app.state::<AppState>().config.lock().unwrap().clone();
            let ce_bind = std::env::var("CONDOR_EYE_BIND")
                .unwrap_or_else(|_| "127.0.0.1".to_string());
            let ce_port = std::env::var("CONDOR_EYE_PORT")
                .unwrap_or_else(|_| "9050".to_string())
                .parse::<u16>()
                .unwrap_or(9050);
            tauri::async_runtime::spawn(http_api::start_server(ce_config, ce_bind, ce_port));
```

- [ ] **Step 2: Verify it compiles**

Run: `cd ~/code/dev-tools/visual-validator && cargo.exe check 2>&1 | tail -5`
Expected: compiles without errors

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/main.rs
git commit -m "feat(condor-eye): launch HTTP API server on Tauri startup"
```

---

### Task 5: Build and manual smoke test

- [ ] **Step 1: Build the app**

Run: `cd ~/code/dev-tools/visual-validator && cargo.exe tauri dev 2>&1 | head -20`

Expected: App launches, console shows `[CE] HTTP API starting on 127.0.0.1:9050`

- [ ] **Step 2: Test status endpoint from WSL**

Run: `curl -s http://localhost:9050/api/status | jq .`

Expected:
```json
{
  "running": true,
  "version": "0.1.0",
  "api_key_configured": true,
  "model": "claude-haiku-4-5-20251001"
}
```

Note: WSL can reach Windows localhost via localhost forwarding. If this fails, try the Windows IP or `$(hostname).local:9050`.

- [ ] **Step 3: Test capture endpoint (full screen)**

Run:
```bash
curl -s -X POST http://localhost:9050/api/capture \
  -H "Content-Type: application/json" \
  -d '{"prompt": "What applications are visible on this screen?"}' \
  | jq '{description: .description, latency_ms: .latency_ms, region: .region, cost: .cost_estimate_usd}'
```

Expected: JSON with description of screen content, latency in ms, full screen region, cost estimate.

- [ ] **Step 4: Test locate endpoint**

Run:
```bash
curl -s -X POST http://localhost:9050/api/locate \
  -H "Content-Type: application/json" \
  -d '{"target": "any browser window"}' \
  | jq .
```

Expected: JSON with `found`, `bounds`, `confidence`, `description`.

- [ ] **Step 5: Commit (no changes expected, just checkpoint)**

If any fixes were needed during smoke test, commit them:
```bash
git add -A && git commit -m "fix(condor-eye): smoke test fixes for HTTP API"
```

---

## Chunk 2: MCP Server

### Task 6: Create MCP server package

**Files:**
- Create: `mcp/package.json`

- [ ] **Step 1: Create mcp directory and package.json**

Create `mcp/package.json`:

```json
{
  "name": "condor-eye-mcp",
  "version": "0.1.0",
  "description": "MCP server for Condor Eye — gives Claude agents the ability to see the screen",
  "type": "module",
  "main": "index.js",
  "scripts": {
    "start": "node index.js"
  },
  "dependencies": {
    "@modelcontextprotocol/sdk": "^1.12.1"
  }
}
```

- [ ] **Step 2: Install dependencies**

Run: `cd ~/code/dev-tools/visual-validator/mcp && npm install`

Expected: `node_modules/` created, `package-lock.json` generated.

- [ ] **Step 3: Add node_modules to .gitignore**

Append to the project `.gitignore`:

```
mcp/node_modules/
```

- [ ] **Step 4: Commit**

```bash
cd ~/code/dev-tools/visual-validator
git add mcp/package.json mcp/package-lock.json .gitignore
git commit -m "feat(condor-eye): add MCP server package scaffolding"
```

---

### Task 7: Create MCP server

**Files:**
- Create: `mcp/index.js`

- [ ] **Step 1: Create index.js with all 3 tools**

Create `mcp/index.js`:

```javascript
#!/usr/bin/env node

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

const DEFAULT_HOST = "localhost:9050";
const HTTP_TIMEOUT_MS = 60_000;

// ── Tool definitions ──

const TOOLS = [
  {
    name: "condor_eye_capture",
    description:
      "Capture a screen region and get an AI description of what's visible. " +
      "If no region is specified, captures the full screen. " +
      "Returns a text description of the screen content.",
    inputSchema: {
      type: "object",
      properties: {
        prompt: {
          type: "string",
          description:
            "What to look for or describe. Default: general description of visible content.",
        },
        region: {
          type: "object",
          description: "Screen region to capture in pixels. Omit for full screen.",
          properties: {
            x: { type: "integer" },
            y: { type: "integer" },
            width: { type: "integer" },
            height: { type: "integer" },
          },
          required: ["x", "y", "width", "height"],
        },
        host: {
          type: "string",
          description: `Condor Eye app host. Default: ${DEFAULT_HOST}`,
        },
        include_image: {
          type: "boolean",
          description:
            "Include base64 image in response. Default: false (saves context space).",
        },
      },
    },
  },
  {
    name: "condor_eye_locate",
    description:
      "Find a window or UI element on screen. Captures the full screen and uses AI " +
      "to identify the target and return its bounding box. Use this before capture " +
      "to find the region of interest.",
    inputSchema: {
      type: "object",
      properties: {
        target: {
          type: "string",
          description:
            'Description of what to find. E.g., "Chrome browser", "the terminal window", "VS Code editor".',
        },
        host: {
          type: "string",
          description: `Condor Eye app host. Default: ${DEFAULT_HOST}`,
        },
      },
      required: ["target"],
    },
  },
  {
    name: "condor_eye_status",
    description:
      "Check if the Condor Eye app is running and get its current configuration.",
    inputSchema: {
      type: "object",
      properties: {
        host: {
          type: "string",
          description: `Condor Eye app host. Default: ${DEFAULT_HOST}`,
        },
      },
    },
  },
];

// ── HTTP client ──

async function callApi(host, method, path, body) {
  const url = `http://${host}${path}`;
  const options = {
    method,
    headers: { "Content-Type": "application/json" },
    signal: AbortSignal.timeout(HTTP_TIMEOUT_MS),
  };
  if (body) {
    options.body = JSON.stringify(body);
  }

  const resp = await fetch(url, options);
  const data = await resp.json();

  if (!resp.ok) {
    throw new Error(data.error || `HTTP ${resp.status}`);
  }
  return data;
}

// ── Tool handlers ──

async function handleCapture(args) {
  const host = args.host || DEFAULT_HOST;
  const includeImage = args.include_image || false;

  const body = {};
  if (args.prompt) body.prompt = args.prompt;
  if (args.region) body.region = args.region;

  const result = await callApi(host, "POST", "/api/capture", body);

  // Build response — strip base64 image by default to save context space
  const response = {
    description: result.description,
    latency_ms: result.latency_ms,
    region: result.region,
    cost_estimate_usd: result.cost_estimate_usd,
  };

  if (includeImage) {
    response.image = result.image;
  }

  return response;
}

async function handleLocate(args) {
  const host = args.host || DEFAULT_HOST;
  return callApi(host, "POST", "/api/locate", { target: args.target });
}

async function handleStatus(args) {
  const host = args.host || DEFAULT_HOST;
  return callApi(host, "GET", "/api/status");
}

// ── MCP server ──

const server = new Server(
  { name: "condor-eye", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: TOOLS,
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    let result;
    switch (name) {
      case "condor_eye_capture":
        result = await handleCapture(args || {});
        break;
      case "condor_eye_locate":
        result = await handleLocate(args || {});
        break;
      case "condor_eye_status":
        result = await handleStatus(args || {});
        break;
      default:
        return {
          content: [{ type: "text", text: `Unknown tool: ${name}` }],
          isError: true,
        };
    }

    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  } catch (error) {
    return {
      content: [
        {
          type: "text",
          text: `Condor Eye error: ${error.message}`,
        },
      ],
      isError: true,
    };
  }
});

// Start
const transport = new StdioServerTransport();
await server.connect(transport);
```

- [ ] **Step 2: Verify MCP server starts without errors**

Run: `echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | timeout 3 node ~/code/dev-tools/visual-validator/mcp/index.js 2>/dev/null || true`

Expected: JSON response with server info (or clean timeout — the server waits for stdio input). No crash or missing module errors.

- [ ] **Step 3: Commit**

```bash
cd ~/code/dev-tools/visual-validator
git add mcp/index.js
git commit -m "feat(condor-eye): add MCP server with capture/locate/status tools"
```

---

## Chunk 3: Plugin Registration & Integration

### Task 8: Register port and MCP server

**Files:**
- Modify: `~/code/ports.json`
- Modify: `~/.claude/settings.json`

- [ ] **Step 1: Register port 9050 in ports.json**

Add entry for condor-eye in `~/code/ports.json`. Find the appropriate location and add:

```json
{
  "port": 9050,
  "repo": "dev-tools/visual-validator",
  "service": "condor-eye-http-api",
  "notes": "Condor Eye HTTP API — screen capture + AI vision for MCP"
}
```

- [ ] **Step 2: Register MCP server in Claude Code settings**

In `~/.claude/settings.json`, add `mcpServers` key (at the top level, alongside existing keys):

```json
"mcpServers": {
  "condor-eye": {
    "command": "node",
    "args": ["/home/mikem/code/dev-tools/visual-validator/mcp/index.js"]
  }
}
```

- [ ] **Step 3: Commit ports.json**

```bash
cd ~/code
git add ports.json
git commit -m "feat: register condor-eye HTTP API on port 9050"
```

---

### Task 9: End-to-end integration test

This task verifies the full pipeline: Claude Code → MCP server → HTTP API → screen capture → Condor Vision → response.

**Prerequisites:**
- Tauri app running (`cargo.exe tauri dev` in a separate terminal)
- Claude Code restarted (to pick up MCP server config)

- [ ] **Step 1: Verify MCP server is registered**

Start a new Claude Code session. The MCP server should appear in available tools.
Check: `condor_eye_status`, `condor_eye_capture`, `condor_eye_locate` should be available.

- [ ] **Step 2: Test status from Claude Code**

From Claude Code, call: `condor_eye_status()`

Expected: JSON showing `running: true`, `api_key_configured: true`, `model: "claude-haiku-4-5-20251001"`.

- [ ] **Step 3: Test capture from Claude Code**

From Claude Code, call: `condor_eye_capture({ prompt: "What is visible on screen?" })`

Expected: JSON with `description` (text describing screen), `latency_ms`, `region`, `cost_estimate_usd`. Should complete within 5 seconds.

- [ ] **Step 4: Test locate from Claude Code**

From Claude Code, call: `condor_eye_locate({ target: "any browser window" })`

Expected: JSON with `found` (boolean), `bounds` (coordinates or null), `confidence`, `description`.

- [ ] **Step 5: Test multi-step pattern (locate then capture)**

```
1. condor_eye_locate({ target: "the terminal or command prompt" })
   → note the bounds
2. condor_eye_capture({ region: <bounds from step 1>, prompt: "Read the text in this terminal" })
   → detailed description of terminal content
```

---

### Task 10: Update CLAUDE.md

**Files:**
- Modify: `~/code/dev-tools/visual-validator/CLAUDE.md`

- [ ] **Step 1: Add Condor Eye HTTP API and MCP server documentation**

Add a new section to the project CLAUDE.md covering:
- HTTP API endpoints and port
- MCP server location and registration
- How to start (Tauri app must be running for HTTP API)
- Environment variables (`CONDOR_EYE_BIND`, `CONDOR_EYE_PORT`)

- [ ] **Step 2: Commit**

```bash
cd ~/code/dev-tools/visual-validator
git add CLAUDE.md
git commit -m "docs: add Condor Eye HTTP API and MCP server to CLAUDE.md"
```
