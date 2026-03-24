use std::time::Duration;

use tauri::Manager;

use crate::audio::{enumerate_audio_sessions, SharedTapRegistry, TapStatus};
use crate::config::AppConfig;

pub async fn run_watcher(
    config: AppConfig,
    registry: SharedTapRegistry,
    app: tauri::AppHandle,
) {
    eprintln!(
        "[condor_audio] watcher starting: bind={} port={} transport={}",
        config.audio_bind, config.audio_port, config.audio_transport
    );

    loop {
        // Reflect active tap count in the window title (consent indicator).
        {
            let guard = registry.lock().await;
            let recording: Vec<String> = guard
                .taps
                .values()
                .filter(|t| t.status == TapStatus::Running)
                .map(|t| t.app_name.clone())
                .collect();
            if let Some(win) = app.get_webview_window("main") {
                let title = if recording.is_empty() {
                    "Condor Eye".to_string()
                } else {
                    format!("Condor Eye \u{2014} Recording: {}", recording.join(", "))
                };
                win.set_title(&title).ok();
            }
        }

        match enumerate_audio_sessions() {
            Ok(sessions) => {
                if !sessions.is_empty() {
                    eprintln!(
                        "[condor_audio] {} active audio session(s) detected",
                        sessions.len()
                    );
                }
            }
            Err(err) => {
                eprintln!("[condor_audio] watcher: {}", err);
            }
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
