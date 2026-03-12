# Visual Validator — Design Spec

**Date**: 2026-03-11
**Location**: `~/code/dev-tools/visual-validator/`
**Runtime**: Tauri 2 (Rust backend + WebView frontend)
**Purpose**: Persistent screen capture tool that compares what a trading display *shows* against known ground truth data, using Claude vision for extraction.

---

## Problem

We have L2 market depth data flowing through a multi-stage pipeline (IBKRDepthObserver → Redis → ws-bridge → DOM viewer). We can verify data consistency programmatically at each stage, but we cannot verify that **what the user sees on screen** matches the data. A rendered price ladder might have the right data flowing in but render it incorrectly — wrong price, wrong volume, missing level, off-by-one in the ladder.

We also want to compare our DOM viewer against known-good references (Bookmap, Thinkorswim Active Trader) to validate our rendering matches professional tools.

## Why Tauri (not Electron)

| Concern | Electron | Tauri |
|---|---|---|
| WSL2 screen capture | Cannot capture Windows-side apps (desktopCapturer sees only X11/Wayland) | Runs natively on Windows — full access to Win32 screen capture APIs |
| Runtime size | ~200MB (ships Chromium) | ~10MB (uses system WebView2) |
| Backend language | Node.js | Rust — learning opportunity, fast, safe |
| Memory footprint | ~150MB+ | ~20-30MB |
| Security | Node process has full system access | Rust backend with explicit permission model |

The WSL2 limitation is the deciding factor — the core use case (capturing Bookmap/ToS running as Windows apps) is impossible from WSL. Tauri runs natively on Windows and connects to Redis over localhost (WSL2's `localhostForwarding` allows Windows apps to reach services running inside WSL2 via `localhost`).

## Solution

A Tauri app providing a draggable, always-on-top transparent capture frame. The user positions it over any trading display. Pressing a button captures the region underneath, sends it to Claude Haiku for structured data extraction, snapshots Redis ground truth simultaneously, and compares the two.

---

## Architecture

```
┌──────────────────────────────────────────┐
│         Tauri Rust Backend               │
│                                          │
│  ┌─────────────┐  ┌──────────────────┐   │
│  │  capture.rs  │  │   redis.rs       │   │
│  │  (Win32 GDI  │  │  (redis crate →  │   │
│  │   BitBlt or  │  │  market.depth    │   │
│  │  screenshots │  │   XREVRANGE)     │   │
│  │   crate)     │  │                  │   │
│  └──────┬──────┘  └───────┬──────────┘   │
│         │                 │              │
│         ▼                 ▼              │
│  ┌─────────────────────────────────────┐ │
│  │          compare.rs                 │ │
│  │  1. Send screenshot → Claude Haiku  │ │
│  │  2. Parse structured extraction     │ │
│  │  3. Diff against Redis snapshot     │ │
│  │  4. Return ComparisonReport         │ │
│  └──────────────┬──────────────────────┘ │
│                 │                        │
│         Tauri Commands (IPC)             │
└──────────────┬───────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────┐
│      WebView Frontend (HTML/CSS/JS)      │
│                                          │
│  ┌─────────────────────────────────────┐ │
│  │       Capture Frame Window          │ │
│  │  ┌─────────────────────────────┐    │ │
│  │  │   Transparent capture zone  │    │ │
│  │  │   (user positions over      │    │ │
│  │  │    target display)          │    │ │
│  │  └─────────────────────────────┘    │ │
│  │  [📷 Capture] [SPY ▼] [⚙]          │ │
│  └─────────────────────────────────────┘ │
│                                          │
│  ┌─────────────────────────────────────┐ │
│  │       Results Panel                 │ │
│  │  ✓ PASS  32/32 bids, 32/32 asks    │ │
│  │  Mismatches: none                   │ │
│  │  Latency: 1.2s  Cost: $0.003       │ │
│  └─────────────────────────────────────┘ │
└──────────────────────────────────────────┘
```

## Components

### 1. Capture Frame Window

**Tauri window config** (`tauri.conf.json`):
```json
{
  "windows": [{
    "title": "Visual Validator",
    "transparent": true,
    "decorations": false,
    "alwaysOnTop": true,
    "resizable": true,
    "shadow": false,
    "width": 400,
    "height": 700,
    "skipTaskbar": false
  }]
}
```

**UI elements:**
- Thin colored border (2-3px, cyan) defining the capture region
- Bottom toolbar (30px, semi-opaque dark background):
  - **Capture button** — triggers screen capture + comparison
  - **Symbol dropdown** — selects which symbol's ground truth to compare against (SPY, /ES, etc.)
  - **Mode toggle** — "Compare" (vs Redis truth) or "Extract Only" (no Redis, just show what's on screen)
  - **Settings gear** — opens config dialog
- Drag-to-move on the border area
- Resize handles on all edges and corners
- Global hotkey: `Ctrl+Shift+C` for quick capture (registered via Tauri's global shortcut API)

### 2. Screen Capture (capture.rs)

**Method**: The `screenshots` crate provides cross-platform screen capture. On Windows, it uses native Win32 APIs (GDI BitBlt) to capture any visible window content.

```rust
use screenshots::Screen;
use image::{DynamicImage, GenericImageView};

/// Capture the screen region under the validator frame.
/// The frame window is made fully transparent (opacity 0) before capture
/// to avoid capturing our own border.
pub fn capture_region(x: i32, y: i32, width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    let screens = Screen::all()?;

    // Find the screen containing the capture region
    let screen = screens.into_iter()
        .find(|s| {
            let di = s.display_info;
            x >= di.x && x < di.x + di.width as i32
                && y >= di.y && y < di.y + di.height as i32
        })
        .ok_or(CaptureError::NoScreen)?;

    // Capture full screen, then crop to our region
    let full = screen.capture()?;
    let di = screen.display_info;
    let crop_x = (x - di.x) as u32;
    let crop_y = (y - di.y) as u32;

    let img = DynamicImage::from(full);
    let cropped = img.crop_imm(crop_x, crop_y, width, height);

    // Encode to PNG
    let mut buf = Vec::new();
    cropped.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
    Ok(buf)
}
```

**Frame hiding strategy**: Use `window.set_opacity(0.0)` before capture, then `set_opacity(1.0)` after. This avoids the Z-order thrashing that `hide()`/`show()` causes, and prevents the frame border from appearing in the screenshot.

### 3. Redis Snapshot (redis.rs)

Snapshot the latest `DepthObserved` message for a given symbol from the `market.depth` stream.

```rust
use redis::{Client, Commands, streams::StreamRangeReply};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DepthLevel {
    pub price: f64,
    #[serde(alias = "totalVolume")]
    pub total_volume: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DepthSnapshot {
    pub stream_id: String,
    pub timestamp: u64,
    pub symbol: String,
    pub source: String,
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
}

/// Read latest DepthObserved for `symbol` from Redis market.depth stream.
pub fn snapshot_ground_truth(
    redis_url: &str,
    symbol: &str,
) -> Result<Option<DepthSnapshot>, RedisError> {
    let client = Client::open(redis_url)?;
    let mut conn = client.get_connection()?;

    // XREVRANGE market.depth + - COUNT 50
    let reply: StreamRangeReply = conn.xrevrange_count("market.depth", "+", "-", 50)?;

    for entry in reply.ids {
        // redis-rs 0.27+: Value::BulkString (renamed from Value::Data in 0.23)
        let data_str: Option<String> = entry.map.get("data")
            .and_then(|v| match v { redis::Value::BulkString(d) => String::from_utf8(d.clone()).ok(), _ => None });

        if let Some(data_json) = data_str {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&data_json) {
                if msg["symbol"].as_str() == Some(symbol) {
                    let bids: Vec<DepthLevel> = serde_json::from_value(
                        msg["bids"].clone()
                    ).unwrap_or_default();
                    let asks: Vec<DepthLevel> = serde_json::from_value(
                        msg["asks"].clone()
                    ).unwrap_or_default();

                    return Ok(Some(DepthSnapshot {
                        stream_id: entry.id.clone(),
                        timestamp: entry.id.split('-').next()
                            .and_then(|s| s.parse().ok()).unwrap_or(0),
                        symbol: symbol.to_string(),
                        source: msg["source"].as_str().unwrap_or("unknown").to_string(),
                        bids,
                        asks,
                    }));
                }
            }
        }
    }
    Ok(None)
}
```

**Redis connectivity**: The app runs on Windows. Redis runs in WSL2. WSL2's `localhostForwarding` (enabled by default in `.wslconfig`) allows Windows-side apps to connect to services inside WSL2 via `localhost`. So `redis://127.0.0.1:6379` works from the Tauri app on Windows.

### 4. Claude Vision Extraction (claude.rs)

Send the screenshot to Claude Haiku 4.5 via the Anthropic Messages API using `reqwest`.

```rust
use reqwest::Client;  // async client (not blocking — runs in Tauri's async runtime)
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractedLevel {
    pub price: f64,
    pub volume: Option<u64>,  // None if obscured
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub symbol: Option<String>,
    #[serde(rename = "displayType")]
    pub display_type: String,
    pub bids: Vec<ExtractedLevel>,
    pub asks: Vec<ExtractedLevel>,
    #[serde(rename = "bestBid")]
    pub best_bid: f64,
    #[serde(rename = "bestAsk")]
    pub best_ask: f64,
    pub spread: f64,
    #[serde(rename = "levelCount")]
    pub level_count: LevelCount,
    pub confidence: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LevelCount {
    pub bids: usize,
    pub asks: usize,
}

const EXTRACTION_PROMPT: &str = r#"Analyze this trading depth/order book display. Extract all visible price levels and their sizes.

Return a JSON object with this exact structure:
{
  "symbol": "detected symbol or null",
  "displayType": "dom_ladder|depth_chart|order_book|unknown",
  "bids": [{"price": 671.27, "volume": 480}, ...],
  "asks": [{"price": 671.30, "volume": 240}, ...],
  "bestBid": 671.27,
  "bestAsk": 671.30,
  "spread": 0.03,
  "levelCount": {"bids": 32, "asks": 32},
  "confidence": "high|medium|low",
  "notes": "any observations about the display"
}

Rules:
- List bids from best (highest price) to worst (lowest price)
- List asks from best (lowest price) to worst (highest price)
- Extract exact prices and volumes as shown on screen
- If a value is partially obscured, set volume to null
- Return ONLY the JSON object, no other text"#;

/// Async extraction — called from Tauri async command.
pub async fn extract_from_screenshot(
    api_key: &str,
    png_bytes: &[u8],
    model: &str,
) -> Result<ExtractionResult, ExtractionError> {
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
                    "text": EXTRACTION_PROMPT,
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
        // Retry once on rate limit
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            // Recursive retry (once)
            return Err(ExtractionError::RateLimit);
        }
        return Err(ExtractionError::Api(format!("{}: {}", status, text)));
    }

    let api_resp: serde_json::Value = resp.json().await?;
    let content_text = api_resp["content"][0]["text"]
        .as_str()
        .ok_or(ExtractionError::Parse("No text in response".into()))?;

    // Handle potential markdown code fences in response
    let json_str = content_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let result: ExtractionResult = serde_json::from_str(json_str)
        .map_err(|e| ExtractionError::Parse(format!("JSON parse: {} — raw: {}", e, json_str)))?;

    Ok(result)
}

/// Error types for extraction.
#[derive(Debug)]
pub enum ExtractionError {
    Api(String),
    Parse(String),
    RateLimit,
    Network(reqwest::Error),
}

impl From<reqwest::Error> for ExtractionError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() { ExtractionError::Api("Request timed out".into()) }
        else { ExtractionError::Network(e) }
    }
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(s) => write!(f, "API error: {}", s),
            Self::Parse(s) => write!(f, "Parse error: {}", s),
            Self::RateLimit => write!(f, "Rate limited — retry in a moment"),
            Self::Network(e) => write!(f, "Network error: {}", e),
        }
    }
}
```

**Error handling**: Wraps API errors, rate limits (429 → retry with backoff), timeouts, and malformed responses. Strips markdown code fences that Claude sometimes adds around JSON.

### 5. Comparison Engine (compare.rs)

Diffs the extracted data against Redis ground truth. Price tolerance is per-instrument.

```rust
use serde::Serialize;

/// Tick sizes for known instruments — determines price matching tolerance.
const TICK_SIZES: &[(&str, f64)] = &[
    ("SPY", 0.01), ("AAPL", 0.01), ("QQQ", 0.01),
    ("ES", 0.25), ("NQ", 0.25), ("MES", 0.25), ("MNQ", 0.25),
    ("YM", 1.0), ("RTY", 0.10), ("CL", 0.01), ("GC", 0.10),
];

fn tick_size(symbol: &str) -> f64 {
    let bare = symbol.trim_start_matches('/').to_uppercase();
    TICK_SIZES.iter()
        .find(|(s, _)| *s == bare)
        .map(|(_, t)| *t)
        .unwrap_or(0.01)
}

#[derive(Debug, Serialize)]
pub struct ComparisonReport {
    pub timestamp: u64,
    pub symbol: String,
    pub overall: Status,          // PASS | WARN | FAIL
    pub level_count: LevelCountReport,
    pub best_bid_ask: BestBidAskReport,
    pub mismatches: Vec<Mismatch>,
    pub missing: Vec<MissingLevel>,    // in truth but not displayed
    pub extra: Vec<ExtraLevel>,        // displayed but not in truth
    pub api_latency_ms: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Serialize)]
pub enum Status { PASS, WARN, FAIL, EXTRACT_ONLY }

#[derive(Debug, Serialize)]
pub struct Mismatch {
    pub side: String,
    pub price: f64,
    pub extracted_volume: u64,
    pub truth_volume: u64,
}

#[derive(Debug, Serialize)]
pub struct MissingLevel {
    pub side: String,
    pub price: f64,
    pub volume: u64,
}

#[derive(Debug, Serialize)]
pub struct ExtraLevel {
    pub side: String,
    pub price: f64,
    pub volume: Option<u64>,
}

pub fn compare_books(
    extracted: &ExtractionResult,
    truth: &DepthSnapshot,
) -> ComparisonReport {
    let tolerance = tick_size(&truth.symbol) * 0.5;
    let mut report = ComparisonReport { /* ... init fields ... */ };

    // Level count comparison
    report.level_count.match_ =
        extracted.bids.len() == truth.bids.len()
        && extracted.asks.len() == truth.asks.len();

    // Best bid/ask comparison
    report.best_bid_ask.bid_match =
        (extracted.best_bid - truth.bids[0].price).abs() < tolerance;
    report.best_bid_ask.ask_match =
        (extracted.best_ask - truth.asks[0].price).abs() < tolerance;

    // Forward pass: find truth levels missing from extraction
    for side_name in &["bids", "asks"] {
        let (t_levels, e_levels) = match *side_name {
            "bids" => (&truth.bids, &extracted.bids),
            _ => (&truth.asks, &extracted.asks),
        };

        for t in t_levels {
            let found = e_levels.iter().find(|e|
                (e.price - t.price).abs() < tolerance
            );
            match found {
                None => report.missing.push(MissingLevel {
                    side: side_name.to_string(),
                    price: t.price,
                    volume: t.total_volume,
                }),
                Some(e) if e.volume.is_some()
                    && e.volume.unwrap() != t.total_volume => {
                    report.mismatches.push(Mismatch {
                        side: side_name.to_string(),
                        price: t.price,
                        extracted_volume: e.volume.unwrap(),
                        truth_volume: t.total_volume,
                    });
                }
                _ => {} // match
            }
        }

        // Reverse pass: find extracted levels not in truth
        for e in e_levels {
            let in_truth = t_levels.iter().any(|t|
                (e.price - t.price).abs() < tolerance
            );
            if !in_truth {
                report.extra.push(ExtraLevel {
                    side: side_name.to_string(),
                    price: e.price,
                    volume: e.volume,
                });
            }
        }
    }

    // Set overall status
    report.overall = if !report.mismatches.is_empty() || report.missing.len() > 3 {
        Status::FAIL
    } else if !report.missing.is_empty() || !report.level_count.match_ {
        Status::WARN
    } else {
        Status::PASS
    };

    report
}
```

### 6. Tauri Commands (main.rs)

Tauri commands are the IPC bridge between the Rust backend and the WebView frontend.

```rust
use tauri::Manager;
use std::sync::Mutex;

/// Shared app state managed by Tauri.
pub struct AppState {
    pub config: Mutex<AppConfig>,
}

pub struct AppConfig {
    pub api_key: String,
    pub redis_url: String,
    pub model: String,   // e.g. "claude-haiku-4-5-20251001"
}

#[tauri::command]
async fn capture_and_compare(
    window: tauri::Window,
    symbol: String,
    mode: String,   // "compare" or "extract_only"
    state: tauri::State<'_, AppState>,
) -> Result<ComparisonReport, String> {
    let config = state.config.lock().unwrap().clone();

    // 1. Hide frame (set opacity to 0 — avoids Z-order thrashing vs hide/show)
    window.set_opacity(0.0).map_err(|e| e.to_string())?;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 2. Get window position/size, apply DPI scale factor.
    //    Tauri reports logical pixels; screen capture APIs use physical pixels.
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;

    let phys_x = (pos.x as f64 * scale) as i32;
    let phys_y = (pos.y as f64 * scale) as i32;
    let phys_w = (size.width as f64 * scale) as u32;
    let phys_h = (size.height as f64 * scale) as u32;

    // 3. Capture screen region (blocking I/O — run on blocking thread)
    let png = tokio::task::spawn_blocking(move || {
        capture::capture_region(phys_x, phys_y, phys_w, phys_h)
    }).await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(|e| format!("Capture failed: {}", e))?;

    // 4. Restore frame
    window.set_opacity(1.0).map_err(|e| e.to_string())?;

    // 5. Send to Claude API (async)
    let start = std::time::Instant::now();
    let extracted = claude::extract_from_screenshot(
        &config.api_key, &png, &config.model
    ).await
        .map_err(|e| format!("Extraction failed: {}", e))?;
    let api_latency = start.elapsed().as_millis() as u64;

    // 6. If extract-only mode, return extraction report without comparison
    if mode == "extract_only" {
        return Ok(ComparisonReport {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
            symbol,
            overall: Status::EXTRACT_ONLY,
            level_count: LevelCountReport {
                extracted: LevelCount { bids: extracted.bids.len(), asks: extracted.asks.len() },
                truth: LevelCount { bids: 0, asks: 0 },
                match_: false,
            },
            best_bid_ask: BestBidAskReport {
                extracted: BidAsk { bid: extracted.best_bid, ask: extracted.best_ask },
                truth: BidAsk { bid: 0.0, ask: 0.0 },
                bid_match: false, ask_match: false,
            },
            mismatches: vec![], missing: vec![], extra: vec![],
            api_latency_ms: api_latency,
            estimated_cost_usd: estimate_cost(&png, &config.model),
            extraction: Some(extracted),  // include raw extraction for display
        });
    }

    // 7. Snapshot Redis ground truth (blocking I/O — run on blocking thread)
    let redis_url = config.redis_url.clone();
    let sym = symbol.clone();
    let truth = tokio::task::spawn_blocking(move || {
        redis::snapshot_ground_truth(&redis_url, &sym)
    }).await
        .map_err(|e| format!("Task join failed: {}", e))?
        .map_err(|e| format!("Redis failed: {}", e))?
        .ok_or("No depth data found for symbol in Redis")?;

    // 8. Compare
    let mut report = compare::compare_books(&extracted, &truth);
    report.api_latency_ms = api_latency;
    report.estimated_cost_usd = estimate_cost(&png, &config.model);

    Ok(report)
}

/// Tauri app setup.
fn main() {
    let config = AppConfig {
        api_key: std::env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY must be set"),
        redis_url: std::env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
        model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string()),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::init())
        .manage(AppState { config: Mutex::new(config) })
        .invoke_handler(tauri::generate_handler![capture_and_compare])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn estimate_cost(png: &[u8], model: &str) -> f64 {
    // Image tokens ≈ (w * h) / 750, output ≈ 500 tokens
    // Haiku: $1/M input, $5/M output
    let input_tokens = 3000.0; // typical for a cropped screenshot
    let output_tokens = 500.0;
    match model {
        m if m.contains("haiku") => (input_tokens / 1_000_000.0) * 1.0 + (output_tokens / 1_000_000.0) * 5.0,
        m if m.contains("sonnet") => (input_tokens / 1_000_000.0) * 3.0 + (output_tokens / 1_000_000.0) * 15.0,
        _ => 0.01, // conservative estimate
    }
}
```

### 7. Frontend (WebView)

**Critical CSS requirement**: WebView2 transparency requires the HTML body to be transparent. Without this, the window appears opaque white despite the Tauri config.

```css
/* style.css — required for transparent window */
html, body {
  background: transparent;
  margin: 0;
  padding: 0;
  overflow: hidden;
  font-family: 'Segoe UI', system-ui, sans-serif;
  color: #e0e0e0;
}
```

**Tauri IPC invoke pattern** (how frontend calls Rust commands):

```javascript
// app.js
const { invoke } = window.__TAURI__.core;

document.getElementById('capture-btn').addEventListener('click', async () => {
  const symbol = document.getElementById('symbol-select').value;
  const mode = document.getElementById('mode-select').value;

  setStatus('capturing...');
  try {
    const report = await invoke('capture_and_compare', { symbol, mode });
    renderResults(report);
  } catch (err) {
    setStatus(`Error: ${err}`);
  }
});
```

**Results panel** displays below the capture zone:

- **Status badge**: PASS (green) / WARN (yellow) / FAIL (red) / EXTRACT_ONLY (blue)
- **Summary**: "32/32 bids, 32/32 asks — all prices match"
- **Mismatch table** (if any): price, side, extracted vs truth volume
- **Missing levels**: prices in truth but not visible on screen
- **Extra levels**: prices displayed but not in truth (indicates stale display data)
- **Metadata**: capture timestamp, API latency, estimated cost, Redis stream ID

---

## Data Contract: DepthObserved

The `market.depth` Redis stream contains `DepthObserved` messages with this structure:

```json
{
  "kind": "DepthObserved",
  "v": 1,
  "symbol": "SPY",
  "source": "ibkr",
  "exchange": "SMART",
  "ingestTs": 1773275334540,
  "bookTime": 1773275334540,
  "bids": [
    {
      "price": 671.27,
      "totalVolume": 480,
      "numOrders": 1,
      "orders": [{ "exchange": "ARCA", "volume": 480, "sequence": 0 }]
    }
  ],
  "asks": [ /* same shape */ ]
}
```

**Field mapping** (Claude extraction → ground truth):
| Extracted field | Truth field | Notes |
|---|---|---|
| `bids[].price` | `bids[].price` | Compared with per-instrument tolerance |
| `bids[].volume` | `bids[].totalVolume` | Explicit mapping in comparator |
| `levelCount.bids` | `bids.length` | Count comparison |

---

## File Structure

```
visual-validator/
├── src-tauri/
│   ├── Cargo.toml          # Rust dependencies
│   ├── tauri.conf.json      # Tauri window config
│   ├── src/
│   │   ├── main.rs          # Tauri setup, commands, state
│   │   ├── capture.rs       # Screen capture (screenshots crate)
│   │   ├── truth.rs         # Pluggable truth sources (Redis, API, file)
│   │   ├── claude.rs        # Claude API vision extraction
│   │   ├── compare.rs       # Diff engine
│   │   └── config.rs        # Settings, profiles, API key loading
│   └── icons/               # App icons
├── src/                     # WebView frontend
│   ├── index.html           # Capture frame + results UI
│   ├── style.css            # Transparent frame, toolbar, results
│   └── app.js               # Frontend logic (invoke Tauri commands)
├── profiles/                # Extraction profiles (JSON)
│   ├── depth.json           # L2 depth ladder (default)
│   ├── candle.json          # Candlestick chart OHLC
│   ├── quote.json           # Quote screen bid/ask/last
│   └── custom.json          # User-editable template
├── CLAUDE.md                # Project-specific instructions
└── docs/
    └── superpowers/specs/
        └── 2026-03-11-visual-validator-design.md
```

## Rust Dependencies (Cargo.toml)

```toml
[dependencies]
tauri = { version = "2", features = ["transparent"] }
tauri-plugin-global-shortcut = "2"        # global hotkey (separate plugin in Tauri 2)
serde = { version = "1", features = ["derive"] }
serde_json = "1"
redis = "0.27"
reqwest = { version = "0.12", features = ["json"] }  # async (not blocking)
tokio = { version = "1", features = ["rt", "time"] }
base64 = "0.22"
image = "0.25"
screenshots = "0.8"
```

## Configuration

API key loaded from environment (set in shell profile or via 1Password):

```bash
# Load from 1Password (recommended)
export ANTHROPIC_API_KEY=$(op.exe read "op://Development/Anthropic API Key/credential")

# Or set directly
export ANTHROPIC_API_KEY=sk-ant-...

# Redis (WSL2 mirrors to Windows localhost)
export REDIS_URL=redis://127.0.0.1:6379
```

The app reads from environment at startup. No `.env` file with plaintext secrets.

## Build & Run

```bash
# Prerequisites (one-time)
# Install Rust: https://rustup.rs/
# Install Tauri CLI: cargo install tauri-cli

cd ~/code/dev-tools/visual-validator

# Development
cargo tauri dev

# Build release
cargo tauri build
# Produces: src-tauri/target/release/visual-validator.exe
```

## Usage

1. Launch `visual-validator.exe` (or `cargo tauri dev`)
2. Position the transparent frame over Bookmap, ToS, or our DOM viewer
3. Select symbol (SPY, /ES) from dropdown
4. Click **Capture** or press `Ctrl+Shift+C`
5. Wait ~1-2s for Claude extraction + comparison
6. Review results in the panel below

**Extract Only mode**: When no Redis ground truth is available (e.g., comparing two third-party tools), use "Extract Only" mode to just extract visible data from the screenshot without comparison.

## Error Handling

| Error | Behavior |
|---|---|
| Claude API timeout (>30s) | Show "Extraction timed out — try again" in results |
| Claude API rate limit (429) | Retry once after 2s, then show error |
| Claude returns malformed JSON | Show raw response with "Could not parse extraction" |
| Redis not reachable | Show "Ground truth unavailable" — extraction still works in Extract Only mode |
| No matching symbol in Redis | Show "No depth data for {symbol}" — suggest checking pipeline |
| Screen capture fails | Show "Capture failed" — may need to run as admin for some apps |

## Limitations & Mitigations

| Limitation | Mitigation |
|---|---|
| Claude vision imperfect with small numbers | Structured prompt, confidence field, tolerance-based matching |
| Frame opacity flash during capture | 50ms at opacity 0 — imperceptible |
| Redis snapshot vs capture timing delta | Capture Redis first (faster), then screen — <100ms delta |
| After-hours crossed books | Notes field captures anomalies; tolerance handles it |
| Price tolerance varies by instrument | Per-instrument tick size table in comparator |

## Generic Design — Not Stove-Piped

The tool is designed as a **general-purpose visual data validator**, not just an L2 depth checker. The architecture supports multiple use cases through pluggable extraction prompts and truth sources.

### Extraction Profiles

The extraction prompt is a configurable template, not hardcoded. Different profiles handle different display types:

| Profile | Captures From | Extracts | Truth Source |
|---|---|---|---|
| `depth` (default) | DOM ladders, Bookmap depth | Prices, volumes, bid/ask levels | Redis `market.depth` |
| `candle` | Candlestick charts | OHLC values, timestamps | Redis `candle.update` or API |
| `quote` | Quote screens | Bid/ask, last price, volume | Redis or tt-exec `/api/quotes` |
| `heatmap` | Our L2 surface heatmap | Intensity bands, price range | Redis `cf.tick` |
| `custom` | Any display | User-defined prompt | User-defined or none |

Profiles are JSON files in a `profiles/` directory:

```json
{
  "name": "depth",
  "prompt": "Analyze this trading depth/order book display...",
  "truthSource": {
    "type": "redis_stream",
    "stream": "market.depth",
    "matchField": "symbol"
  },
  "comparison": {
    "priceToleranceMode": "tick_size",
    "volumeField": { "extracted": "volume", "truth": "totalVolume" }
  }
}
```

### Use Cases Beyond L2

1. **Candle chart validation**: Capture a candle chart, extract OHLC values, compare against known candle data
2. **Quote screen verification**: Capture a broker quote panel, extract bid/ask/last, verify against our data feed
3. **Heatmap intensity check**: Capture our L2 surface, extract price bands and intensity, compare against raw CF_TICK bins
4. **Cross-tool comparison**: Extract from Bookmap AND our DOM (two captures, no Redis), diff the extractions against each other
5. **Arbitrary screen reading**: Use "custom" profile with a free-form prompt to extract any structured data from any screen

### Custom Prompts

The settings panel allows editing the extraction prompt directly for one-off analysis. This makes the tool useful for *any* situation where you need AI to read structured data from a screen and compare it against something you know.

## Future Extensions

- **History**: Save capture + truth + report to SQLite for regression tracking
- **Auto-capture**: Timer-based periodic captures for drift monitoring
- **Multi-symbol**: Compare multiple symbols in one capture
- **Diff overlay**: Highlight mismatched areas on the captured screenshot
- **Headless mode**: CLI invocation for CI (provide screenshot path)
- **Cross-capture comparison**: Compare two Extract Only captures against each other (our DOM vs Bookmap)
