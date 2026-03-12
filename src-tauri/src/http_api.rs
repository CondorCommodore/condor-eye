use axum::{
    extract::{Query, State},
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
    /// Optional HWND — if set, brings window to foreground before capture.
    pub hwnd: Option<u64>,
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

    // Bring target window to foreground if hwnd provided
    if let Some(hwnd) = req.hwnd {
        tokio::task::spawn_blocking(move || {
            crate::windows::focus_window(hwnd);
        })
        .await
        .map_err(|e| api_error(format!("Focus join: {}", e)))?;
        // Brief delay for window manager to repaint
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

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

async fn handle_windows(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let windows = if let Some(query) = params.get("query") {
        let q = query.clone();
        tokio::task::spawn_blocking(move || crate::windows::find_windows(&q))
            .await
            .unwrap_or_default()
    } else {
        tokio::task::spawn_blocking(crate::windows::list_windows)
            .await
            .unwrap_or_default()
    };

    Json(serde_json::json!({
        "windows": windows,
        "count": windows.len(),
    }))
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
        .route("/api/windows", get(handle_windows))
        .with_state(state);

    let addr = format!("{}:{}", bind_addr, port);
    eprintln!("[CE] HTTP API starting on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[CE] Warning: failed to bind to {}: {}. HTTP API disabled.", addr, e);
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[CE] HTTP server error: {}", e);
    }
}
