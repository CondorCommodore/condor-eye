#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod audio_watcher;
mod capture;
mod claude;
mod compare;
mod config;
mod http_api;
mod truth;
mod windows;

use std::sync::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use compare::{ComparisonReport, Status};
use config::{AppConfig, ExtractionProfile};

/// Last capture result — stored for sharing to Discord/coord.
pub struct LastCapture {
    pub description: String,
    pub image_b64: String,
}

/// Shared app state managed by Tauri.
pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub profiles: Mutex<Vec<ExtractionProfile>>,
    pub last_capture: Mutex<Option<LastCapture>>,
    pub audio_registry: audio::SharedTapRegistry,
}

/// Main capture-and-compare command.
///
/// Orchestrates: hide frame → capture → restore → extract → truth → compare.
#[tauri::command]
async fn capture_and_compare(
    window: tauri::Window,
    symbol: String,
    mode: String,
    profile_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<ComparisonReport, String> {
    let cfg = state.config.lock().unwrap().clone();
    let profiles = state.profiles.lock().unwrap().clone();

    eprintln!("[VV] capture_and_compare: symbol={}, mode={}, profile={}", symbol, mode, profile_name);

    // Find the requested profile
    let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .cloned()
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    // 1. Hide frame to prevent capturing our own border
    let _ = window.hide();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 2. Get window position/size.
    //    Tauri 2's outer_position() returns PhysicalPosition and
    //    outer_size() returns PhysicalSize — already in physical pixels.
    //    Do NOT multiply by scale_factor (that would double-scale).
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;

    let cap_x = pos.x;
    let cap_y = pos.y;
    let cap_w = size.width;
    let cap_h = size.height;

    // 3. Capture screen region (blocking I/O)
    eprintln!("[VV] capturing region: x={}, y={}, w={}, h={}", cap_x, cap_y, cap_w, cap_h);
    let png = tokio::task::spawn_blocking(move || {
        capture::capture_region(cap_x, cap_y, cap_w, cap_h)
    })
    .await
    .map_err(|e| { eprintln!("[VV] ERROR capture join: {}", e); format!("Task join: {}", e) })?
    .map_err(|e| { eprintln!("[VV] ERROR capture: {}", e); format!("Capture: {}", e) })?;
    eprintln!("[VV] captured {} bytes PNG", png.len());

    // 4. Restore frame
    let _ = window.show();

    // 5. Send to Claude API (async — profile provides the prompt)
    let start = std::time::Instant::now();
    let extracted = claude::extract_from_screenshot(
        &cfg.api_key,
        &png,
        &cfg.model,
        &profile.prompt,
    )
    .await
    .map_err(|e| { eprintln!("[VV] ERROR extraction: {}", e); format!("Extraction: {}", e) })?;
    let api_latency = start.elapsed().as_millis() as u64;
    eprintln!("[VV] extraction done in {}ms: {} bids, {} asks", api_latency, extracted.bids.len(), extracted.asks.len());

    let cost = config::estimate_cost(cap_w, cap_h, &cfg.model);

    // 6. Extract-only mode — return extraction without comparison
    if mode == "extract_only" || profile.truth_source.source_type == "none" {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        return Ok(ComparisonReport {
            timestamp: now,
            symbol,
            overall: Status::ExtractOnly,
            extracted_bids: extracted.bids.len(),
            extracted_asks: extracted.asks.len(),
            truth_bids: 0,
            truth_asks: 0,
            best_bid_match: false,
            best_ask_match: false,
            mismatches: vec![],
            missing: vec![],
            extra: vec![],
            api_latency_ms: api_latency,
            estimated_cost_usd: cost,
            extraction: Some(extracted),
        });
    }

    // 7. Snapshot Redis ground truth (blocking I/O)
    let redis_url = cfg.redis_url.clone();
    let stream = profile
        .truth_source
        .stream
        .clone()
        .unwrap_or_else(|| "market.depth".to_string());
    let sym = symbol.clone();
    let truth_result = tokio::task::spawn_blocking(move || {
        truth::snapshot_depth(&redis_url, &stream, &sym)
    })
    .await
    .map_err(|e| format!("Task join: {}", e))?
    .map_err(|e| { eprintln!("[VV] ERROR truth: {}", e); format!("Truth: {}", e) })?;
    eprintln!("[VV] truth snapshot: {} bids, {} asks", truth_result.bids.len(), truth_result.asks.len());

    // 8. Compare
    let mut report = compare::compare_books(&extracted, &truth_result);
    report.api_latency_ms = api_latency;
    report.estimated_cost_usd = cost;
    eprintln!("[VV] result: {:?} | mismatches={}, missing={}, extra={}",
        report.overall, report.mismatches.len(), report.missing.len(), report.extra.len());

    Ok(report)
}

/// Free-mode capture — sends screenshot to Claude with a simple prompt, returns raw text.
#[tauri::command]
async fn capture_free(
    window: tauri::Window,
    prompt: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    let user_prompt = if prompt.is_empty() {
        "Describe what you see in this screenshot. Be specific about any data, numbers, charts, or UI elements visible.".to_string()
    } else {
        prompt
    };

    eprintln!("[VV] free capture: prompt={}", &user_prompt[..user_prompt.len().min(100)]);

    // Hide, capture, show
    let _ = window.hide();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let (cx, cy, cw, ch) = (pos.x, pos.y, size.width, size.height);

    let png = tokio::task::spawn_blocking(move || {
        capture::capture_region(cx, cy, cw, ch)
    })
    .await
    .map_err(|e| format!("Task join: {}", e))?
    .map_err(|e| format!("Capture: {}", e))?;

    let _ = window.show();
    eprintln!("[VV] free: captured {} bytes", png.len());

    // Send to Claude — raw text response, no JSON parsing
    let client = reqwest::Client::new();
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);

    let body = serde_json::json!({
        "model": cfg.model,
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
                    "text": user_prompt,
                }
            ]
        }]
    });

    let start = std::time::Instant::now();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Network: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error: {}", text));
    }

    let api_resp: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let content = api_resp["content"][0]["text"]
        .as_str()
        .unwrap_or("(no response)")
        .to_string();

    let ms = start.elapsed().as_millis();
    eprintln!("[VV] free response ({}ms):\n{}", ms, content);

    // Store for share commands
    *state.last_capture.lock().unwrap() = Some(LastCapture {
        description: content.clone(),
        image_b64: b64,
    });

    Ok(content)
}

/// Start window drag.
#[tauri::command]
fn start_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

/// List available profiles.
#[tauri::command]
fn list_profiles(state: tauri::State<'_, AppState>) -> Vec<String> {
    state
        .profiles
        .lock()
        .unwrap()
        .iter()
        .map(|p| p.name.clone())
        .collect()
}

/// Share last capture to Discord via the discord-mcp HTTP bridge.
#[tauri::command]
async fn share_discord(
    channel: Option<String>,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();

    let (description, image_b64) = {
        let guard = state.last_capture.lock().unwrap();
        let c = guard.as_ref().ok_or("No capture to share")?;
        (c.description.clone(), c.image_b64.clone())
    };

    let channel = channel.unwrap_or_else(|| "fleet".to_string());
    let bridge_url = cfg.discord_bridge_url
        .unwrap_or_else(|| "http://localhost:8770".to_string());

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "channel": channel,
        "message": format!("**Condor Eye Capture**\n{}", &description[..description.len().min(1900)]),
        "attachment": image_b64,
        "filename": "capture.png",
    });

    let resp = client
        .post(format!("{}/discord/post", bridge_url))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Discord bridge: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Discord error: {}", text));
    }

    Ok(format!("Sent to #{}", channel))
}

/// Share last capture to a coord agent as a proposed task.
#[tauri::command]
async fn share_coord(
    agent_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let cfg = state.config.lock().unwrap().clone();
    if cfg.coord_api_token.is_empty() {
        return Err("COORD_API_TOKEN not configured".to_string());
    }

    let description = {
        let guard = state.last_capture.lock().unwrap();
        let c = guard.as_ref().ok_or("No capture to share")?;
        c.description.clone()
    };

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "id": format!("ce-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()),
        "summary": format!("[Condor Eye] {}", &description[..description.len().min(200)]),
        "context": description,
        "to_agent": agent_id,
        "tags": ["condor-eye", "capture"],
    });

    let resp = client
        .post(format!("{}/tasks", cfg.coord_api_url))
        .header("Authorization", format!("Bearer {}", cfg.coord_api_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Coord: {}", e))?;

    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Coord error: {}", text));
    }

    Ok(format!("Sent to {}", agent_id))
}

/// Fetch vision overlay data from the local vision server via IPC (bypasses CSP).
#[tauri::command]
async fn fetch_vision(client: tauri::State<'_, reqwest::Client>) -> Result<serde_json::Value, String> {
    let url = std::env::var("VISION_URL")
        .unwrap_or_else(|_| "http://localhost:8090/vision/latest".to_string());
    let resp = client.get(&url).send().await
        .map_err(|e| format!("Vision unreachable: {}", e))?;
    let body: serde_json::Value = resp.json().await
        .map_err(|e| format!("Vision parse: {}", e))?;
    Ok(body)
}

fn main() {
    // Load .env file — first file to set a variable wins (dotenvy skips existing):
    // 1. cwd/.env (dev mode — highest priority)
    // 2. cwd/../.env (dev from src-tauri/)
    // 3. %APPDATA%/Condor Eye/.env (installed app — persistent config)
    // 4. Next to the exe (fallback)
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename("../.env");
    if let Ok(appdata) = std::env::var("APPDATA") {
        let _ = dotenvy::from_path(std::path::Path::new(&appdata).join("Condor Eye").join(".env"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = dotenvy::from_path(dir.join(".env"));
        }
    }

    let app_config = AppConfig::from_env();

    // Load profiles from the profiles/ directory.
    // Try: exe parent (release), cwd (dev), cwd parent (dev from src-tauri/).
    let profiles_dir = [
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("profiles"))),
        Some(std::env::current_dir().unwrap_or_default().join("profiles")),
        Some(std::env::current_dir().unwrap_or_default().join("../profiles")),
    ]
    .into_iter()
    .flatten()
    .find(|d| d.exists())
    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().join("profiles"));
    let profiles = config::load_all_profiles(&profiles_dir);

    if profiles.is_empty() {
        eprintln!("Warning: no profiles found in {}", profiles_dir.display());
    } else {
        eprintln!("Loaded {} profile(s): {:?}",
            profiles.len(),
            profiles.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
    }

    // Spawn a shared reqwest client for vision proxy
    let vision_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("reqwest client");
    let audio_registry = std::sync::Arc::new(tokio::sync::Mutex::new(audio::TapRegistry::default()));

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(vision_client)
        .manage(AppState {
            config: Mutex::new(app_config),
            profiles: Mutex::new(profiles),
            last_capture: Mutex::new(None),
            audio_registry: audio_registry.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            capture_and_compare,
            capture_free,
            list_profiles,
            start_drag,
            share_discord,
            share_coord,
            fetch_vision,
        ])
        .setup(move |app| {
            // Register Ctrl+Shift+C global shortcut
            let shortcut = Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyC,
            );
            let handle = app.handle().clone();
            if let Err(e) = app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
                // Emit event to frontend to trigger capture
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.emit("trigger-capture", ());
                }
            }) {
                eprintln!("Warning: failed to register Ctrl+Shift+C shortcut: {}. Use the UI button instead.", e);
            }
            // Start Condor Eye HTTP API server
            let ce_config = app.state::<AppState>().config.lock().unwrap().clone();
            tauri::async_runtime::spawn(http_api::start_server(
                ce_config.clone(),
                ce_config.condor_eye_bind.clone(),
                ce_config.condor_eye_port,
            ));
            tauri::async_runtime::spawn(http_api::start_audio_server(
                ce_config.clone(),
                ce_config.audio_bind.clone(),
                ce_config.audio_port,
                audio_registry.clone(),
            ));
            if ce_config.condor_audio_auto_watch {
                tauri::async_runtime::spawn(audio_watcher::run_watcher(
                    ce_config,
                    audio_registry.clone(),
                ));
            } else {
                eprintln!("[condor_audio] auto-watch disabled; manual tap mode is active");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
