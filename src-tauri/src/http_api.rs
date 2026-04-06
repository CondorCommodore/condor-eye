use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::audio::{self, SharedTapRegistry};
use crate::capture::{self, Region};
use crate::config::AppConfig;

/// Shared state for the HTTP API server.
pub struct HttpState {
    pub config: AppConfig,
    pub capture_lock: Mutex<()>,
    /// Token from 1Password that authorizes paid AI capture calls.
    /// If set, /api/capture requires `Authorization: Bearer <token>` header.
    /// If empty, /api/capture is disabled entirely (returns 403).
    pub capture_token: String,
    pub audio_registry: Option<SharedTapRegistry>,
}

// ── Request/Response types ──

#[derive(Deserialize)]
pub struct CaptureRequest {
    pub prompt: Option<String>,
    pub region: Option<Region>,
    /// Optional HWND — if set, brings window to foreground before capture.
    pub hwnd: Option<u64>,
    /// Optional key combos to send after focus (e.g. ["ctrl+3"] to switch to tab 3).
    pub keys: Option<Vec<String>>,
    /// If true, capture the window's region without stealing focus.
    /// Requires `hwnd` to resolve window bounds. Ignored if `hwnd` is not set.
    #[serde(default)]
    pub no_focus: bool,
    /// Review provider: "claude", "codex", or "none" (default).
    /// When "none", capture returns image only with no AI description.
    #[serde(default)]
    pub review: crate::review::ReviewProvider,
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
pub struct AudioStatusResponse {
    pub running: bool,
    pub project: audio::AudioProjectStatus,
    pub active_taps: Vec<audio::ActiveTap>,
    pub audio_bind: String,
    pub audio_port: u16,
    pub audio_transport: String,
    pub audio_output_dir: String,
    pub whisper_url: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Deserialize)]
pub struct AudioTapStartRequest {
    pub app: String,
    pub pid: Option<u32>,
}

#[derive(Deserialize)]
pub struct TranscriptQuery {
    pub since: Option<String>,
    pub app: Option<String>,
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

async fn handle_audio_status(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<AudioStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let snapshot = audio::status_snapshot(&state.config, registry).await;
    Ok(Json(AudioStatusResponse {
        running: snapshot.running,
        project: snapshot.project,
        active_taps: snapshot.active_taps,
        audio_bind: snapshot.audio_bind,
        audio_port: snapshot.audio_port,
        audio_transport: snapshot.audio_transport,
        audio_output_dir: snapshot.audio_output_dir,
        whisper_url: snapshot.whisper_url,
    }))
}

async fn handle_audio_sessions(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    match audio::enumerate_audio_sessions() {
        Ok(sessions) => Ok(Json(serde_json::json!({
            "ok": true,
            "count": sessions.len(),
            "sessions": sessions,
        }))),
        Err(error) => Err(api_error(error)),
    }
}

async fn handle_audio_tap_start(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<AudioTapStartRequest>,
) -> Result<Json<audio::ActiveTap>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let pid = req.pid.unwrap_or(0);
    let tap = audio::start_tap(registry, &state.config, &req.app, pid, true)
        .await
        .map_err(api_error)?;
    Ok(Json(tap))
}

async fn handle_audio_tap_stop(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    AxumPath(tap_id): AxumPath<String>,
) -> Result<Json<audio::ActiveTap>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let tap = audio::stop_tap(registry, &tap_id)
        .await
        .map_err(api_error)?;
    Ok(Json(tap))
}

async fn handle_audio_tap_get(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    AxumPath(tap_id): AxumPath<String>,
) -> Result<Json<audio::ActiveTap>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let tap = audio::get_tap(registry, &tap_id)
        .await
        .ok_or_else(|| api_error(format!("Tap not found: {}", tap_id)))?;
    Ok(Json(tap))
}

async fn handle_audio_tap_latest_chunk(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    AxumPath(tap_id): AxumPath<String>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let tap = audio::get_tap(registry, &tap_id)
        .await
        .ok_or_else(|| api_error(format!("Tap not found: {}", tap_id)))?;
    let bytes = audio::latest_chunk_bytes(&tap).map_err(api_error)?;
    Ok(([(axum::http::header::CONTENT_TYPE, "audio/wav")], bytes).into_response())
}

async fn handle_audio_transcripts(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    Query(query): Query<TranscriptQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let items =
        audio::list_transcripts(&state.config, query.app.as_deref(), query.since.as_deref())
            .map_err(api_error)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "count": items.len(),
        "transcripts": items,
    })))
}

async fn handle_audio_transcript_get(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    AxumPath(transcript_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let text = audio::read_transcript(&state.config, &transcript_id).map_err(api_error)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "id": transcript_id,
        "text": text,
    })))
}

async fn handle_audio_latest_transcript(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    AxumPath(tap_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;
    let registry = state
        .audio_registry
        .as_ref()
        .ok_or_else(|| api_error("Audio registry not configured".to_string()))?;
    let tap = audio::get_tap(registry, &tap_id)
        .await
        .ok_or_else(|| api_error(format!("Tap not found: {}", tap_id)))?;
    let text = audio::latest_transcript_text(&tap).map_err(api_error)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "tap_id": tap_id,
        "text": text,
    })))
}

async fn handle_capture(
    State(state): State<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CaptureRequest>,
) -> Result<Json<CaptureResponse>, (StatusCode, Json<ErrorResponse>)> {
    require_capture_token(&state, &headers)?;

    let prompt = req.prompt.unwrap_or_else(|| {
        "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible.".to_string()
    });

    // Serialize captures — one at a time
    let _guard = state.capture_lock.lock().await;

    // Bring target window to foreground if hwnd provided (unless no_focus)
    if let Some(hwnd) = req.hwnd {
        if !req.no_focus {
            tokio::task::spawn_blocking(move || {
                crate::windows::focus_window(hwnd);
            })
            .await
            .map_err(|e| api_error(format!("Focus join: {}", e)))?;
            // Brief delay for window manager to repaint
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }

    // Send key combos if requested (e.g. switch browser tab)
    if let Some(keys) = req.keys {
        for combo in &keys {
            let c = combo.clone();
            tokio::task::spawn_blocking(move || {
                crate::windows::send_key_combo(&c);
            })
            .await
            .map_err(|e| api_error(format!("Keys join: {}", e)))?;
        }
        // Wait for tab switch / UI to settle
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // Capture screen
    let (png, region) = if let Some(r) = req.region {
        let rx = r.x;
        let ry = r.y;
        let rw = r.width;
        let rh = r.height;
        let png = tokio::task::spawn_blocking(move || capture::capture_region(rx, ry, rw, rh))
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

    // Opt-in review via local CLI
    let start = std::time::Instant::now();
    let description = crate::review::review_screenshot(req.review, &png, &prompt)
        .await
        .map_err(|e| api_error(format!("Review: {}", e)))?;
    let latency_ms = start.elapsed().as_millis() as u64;

    let image = base64::engine::general_purpose::STANDARD.encode(&png);

    eprintln!(
        "[CE] capture response: {}ms, {} chars",
        latency_ms,
        description.len()
    );

    Ok(Json(CaptureResponse {
        image,
        description,
        latency_ms,
        region,
        cost_estimate_usd: 0.0,
    }))
}

async fn handle_locate(
    State(_state): State<Arc<HttpState>>,
    Json(req): Json<LocateRequest>,
) -> Result<Json<LocateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Find by window title (free, instant, no AI)
    let target = req.target.clone();
    let windows = tokio::task::spawn_blocking(move || {
        crate::windows::find_windows(&target)
    })
    .await
    .map_err(|e| api_error(format!("Task join: {}", e)))?;

    if let Some(w) = windows.first() {
        Ok(Json(LocateResponse {
            found: true,
            bounds: Some(Region {
                x: w.x,
                y: w.y,
                width: w.width,
                height: w.height,
            }),
            confidence: "high".to_string(),
            description: format!("Found window: {} (pid {})", w.title, w.pid),
        }))
    } else {
        Ok(Json(LocateResponse {
            found: false,
            bounds: None,
            confidence: "none".to_string(),
            description: format!("No window matching '{}' found", req.target),
        }))
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

// ── Raw screenshot — capture without AI analysis (free, no Haiku calls) ──

#[derive(Deserialize)]
pub struct ScreenshotRequest {
    pub region: Option<Region>,
}

async fn handle_screenshot(
    State(state): State<Arc<HttpState>>,
    Json(req): Json<ScreenshotRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let _guard = state.capture_lock.lock().await;

    let (png, region) = if let Some(r) = req.region {
        let (rx, ry, rw, rh) = (r.x, r.y, r.width, r.height);
        let png = tokio::task::spawn_blocking(move || capture::capture_region(rx, ry, rw, rh))
            .await
            .map_err(|e| api_error(format!("Task join: {}", e)))?
            .map_err(|e| api_error(format!("Capture: {}", e)))?;
        (png, r)
    } else {
        tokio::task::spawn_blocking(capture::capture_full_screen)
            .await
            .map_err(|e| api_error(format!("Task join: {}", e)))?
            .map_err(|e| api_error(format!("Capture: {}", e)))?
    };

    let image = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(Json(serde_json::json!({
        "image": image,
        "region": { "x": region.x, "y": region.y, "width": region.width, "height": region.height },
        "size_bytes": png.len(),
    })))
}

// ── Grid config persistence — survives WebView2 cache clears ──

pub(crate) fn grid_config_path() -> std::path::PathBuf {
    if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("Condor Eye").join("grid.json")
    } else {
        std::path::PathBuf::from("grid.json")
    }
}

async fn handle_grid_save(
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let path = grid_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
    .map_err(|e| api_error(format!("Save grid: {}", e)))?;
    eprintln!("[CE] grid config saved to {}", path.display());
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn handle_grid_load() -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    let path = grid_config_path();
    let data =
        std::fs::read_to_string(&path).map_err(|e| api_error(format!("Load grid: {}", e)))?;
    let json: serde_json::Value =
        serde_json::from_str(&data).map_err(|e| api_error(format!("Parse grid: {}", e)))?;
    Ok(Json(json))
}

// ── Vision proxy — forwards to local vision server so JS stays same-origin ──

async fn handle_vision_proxy() -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)>
{
    let url = std::env::var("VISION_URL")
        .unwrap_or_else(|_| "http://localhost:8090/vision/latest".to_string());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| api_error(format!("HTTP client: {}", e)))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| api_error(format!("Vision server unreachable: {}", e)))?;
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| api_error(format!("Vision parse: {}", e)))?;
    Ok(Json(body))
}

// ── Server startup ──

pub async fn start_server(config: AppConfig, bind_addr: String, port: u16) {
    let capture_token = std::env::var("CAPTURE_TOKEN").unwrap_or_default();
    if capture_token.is_empty() {
        eprintln!("[CE] WARNING: CAPTURE_TOKEN not set — /api/capture is DISABLED (403)");
        eprintln!("[CE]   To enable: export CAPTURE_TOKEN=$(op.exe read 'op://Dev/condor-eye-capture/token')");
    } else {
        eprintln!("[CE] CAPTURE_TOKEN set — /api/capture is authorized");
    }
    let state = Arc::new(HttpState {
        config,
        capture_lock: Mutex::new(()),
        capture_token,
        audio_registry: None,
    });

    let app = Router::new()
        .route("/api/status", get(handle_status))
        .route("/api/capture", post(handle_capture))
        .route("/api/locate", post(handle_locate))
        .route("/api/windows", get(handle_windows))
        .route("/api/vision", get(handle_vision_proxy))
        .route("/api/screenshot", post(handle_screenshot))
        .route("/api/grid", get(handle_grid_load))
        .route("/api/grid", post(handle_grid_save))
        .with_state(state);

    let addr = format!("{}:{}", bind_addr, port);
    eprintln!("[CE] HTTP API starting on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[CE] Warning: failed to bind to {}: {}. HTTP API disabled.",
                addr, e
            );
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[CE] HTTP server error: {}", e);
    }
}

pub async fn start_audio_server(
    config: AppConfig,
    bind_addr: String,
    port: u16,
    registry: SharedTapRegistry,
) {
    let capture_token = std::env::var("CAPTURE_TOKEN").unwrap_or_default();
    if capture_token.is_empty() {
        eprintln!("[condor_audio] WARNING: CAPTURE_TOKEN not set — audio API is DISABLED (403)");
    } else {
        eprintln!("[condor_audio] CAPTURE_TOKEN set — audio API authorized");
    }
    let state = Arc::new(HttpState {
        config: config.clone(),
        capture_lock: Mutex::new(()),
        capture_token,
        audio_registry: Some(registry),
    });

    if let Err(err) = audio::ensure_audio_dirs(&config) {
        eprintln!("[condor_audio] warning: {}", err);
    }

    let app = Router::new()
        .route("/", get(serve_audio_ui))
        .route("/api/condor_audio/status", get(handle_audio_status))
        .route("/api/condor_audio/sessions", get(handle_audio_sessions))
        .route("/api/condor_audio/taps", post(handle_audio_tap_start))
        .route(
            "/api/condor_audio/taps/{tap_id}",
            get(handle_audio_tap_get).delete(handle_audio_tap_stop),
        )
        .route(
            "/api/condor_audio/taps/{tap_id}/latest",
            get(handle_audio_tap_latest_chunk),
        )
        .route(
            "/api/condor_audio/taps/{tap_id}/latest-transcript",
            get(handle_audio_latest_transcript),
        )
        .route(
            "/api/condor_audio/transcripts",
            get(handle_audio_transcripts),
        )
        .route(
            "/api/condor_audio/transcripts/{transcript_id}",
            get(handle_audio_transcript_get),
        )
        .with_state(state);

    let addr = format!("{}:{}", bind_addr, port);
    eprintln!("[condor_audio] HTTP API starting on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[condor_audio] Warning: failed to bind to {}: {}. Audio API disabled.",
                addr, e
            );
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[condor_audio] HTTP server error: {}", e);
    }
}

async fn serve_audio_ui() -> impl IntoResponse {
    Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .body(include_str!("audio_ui.html").to_string())
        .unwrap()
}

fn require_capture_token(
    state: &HttpState,
    headers: &axum::http::HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if state.capture_token.is_empty() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: "Capture disabled: no CAPTURE_TOKEN set. Run: op.exe read 'op://Dev/condor-eye-capture/token' to authorize.".to_string(),
            }),
        ));
    }
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected = format!("Bearer {}", state.capture_token);
    if auth != expected {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Invalid or missing Authorization header. Use: Authorization: Bearer <token from op>".to_string(),
            }),
        ));
    }
    Ok(())
}
