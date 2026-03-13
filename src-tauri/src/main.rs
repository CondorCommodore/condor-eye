#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

/// Shared app state managed by Tauri.
pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub profiles: Mutex<Vec<ExtractionProfile>>,
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

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState {
            config: Mutex::new(app_config),
            profiles: Mutex::new(profiles),
        })
        .invoke_handler(tauri::generate_handler![
            capture_and_compare,
            capture_free,
            list_profiles,
            start_drag,
        ])
        .setup(|app| {
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
            // Bind 0.0.0.0 by default — required for WSL2→Windows access.
            // WSL2 can't reach Windows 127.0.0.1 (separate network namespace).
            let ce_bind = std::env::var("CONDOR_EYE_BIND")
                .unwrap_or_else(|_| "0.0.0.0".to_string());
            let ce_port = std::env::var("CONDOR_EYE_PORT")
                .unwrap_or_else(|_| "9050".to_string())
                .parse::<u16>()
                .unwrap_or(9050);
            tauri::async_runtime::spawn(http_api::start_server(ce_config, ce_bind, ce_port));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
