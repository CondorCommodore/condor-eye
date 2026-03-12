#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod claude;
mod compare;
mod config;
mod truth;

use std::sync::Mutex;
use tauri::Manager;

use compare::{ComparisonReport, ExtractionResult, Status};
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

    // Find the requested profile
    let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .cloned()
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    // 1. Hide frame (opacity → 0 to prevent capturing our own border)
    //    set_opacity avoids the Z-order thrashing that hide()/show() causes.
    let _ = window.set_opacity(0.0);
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
    let png = tokio::task::spawn_blocking(move || {
        capture::capture_region(cap_x, cap_y, cap_w, cap_h)
    })
    .await
    .map_err(|e| format!("Task join: {}", e))?
    .map_err(|e| format!("Capture: {}", e))?;

    // 4. Restore frame
    let _ = window.set_opacity(1.0);

    // 5. Send to Claude API (async — profile provides the prompt)
    let start = std::time::Instant::now();
    let extracted = claude::extract_from_screenshot(
        &cfg.api_key,
        &png,
        &cfg.model,
        &profile.prompt,
    )
    .await
    .map_err(|e| format!("Extraction: {}", e))?;
    let api_latency = start.elapsed().as_millis() as u64;

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
    .map_err(|e| format!("Truth: {}", e))?;

    // 8. Compare
    let mut report = compare::compare_books(&extracted, &truth_result);
    report.api_latency_ms = api_latency;
    report.estimated_cost_usd = cost;

    Ok(report)
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
    let app_config = AppConfig::from_env();

    // Load profiles from the profiles/ directory.
    // Try exe's parent dir first (release), fall back to cwd (dev mode).
    let profiles_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("profiles")))
        .filter(|d| d.exists())
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_default().join("profiles")
        });
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
        .plugin(tauri_plugin_global_shortcut::init())
        .manage(AppState {
            config: Mutex::new(app_config),
            profiles: Mutex::new(profiles),
        })
        .invoke_handler(tauri::generate_handler![
            capture_and_compare,
            list_profiles,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
