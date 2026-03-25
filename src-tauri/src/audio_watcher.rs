use std::time::Duration;

use crate::audio::{
    enumerate_audio_sessions, process_is_running, start_tap, stop_tap, tap_is_active,
    SharedTapRegistry, TapStatus,
};
use crate::config::AppConfig;

pub async fn run_watcher(config: AppConfig, registry: SharedTapRegistry) {
    eprintln!(
        "[condor_audio] watcher starting: bind={} port={} transport={}",
        config.audio_bind, config.audio_port, config.audio_transport
    );

    loop {
        let stale_taps = {
            let guard = registry.lock().await;
            guard
                .taps
                .values()
                .filter(|tap| {
                    tap.status == TapStatus::Running && !process_is_running(tap.target_pid)
                })
                .map(|tap| tap.tap_id.clone())
                .collect::<Vec<_>>()
        };
        for tap_id in stale_taps {
            let _ = stop_tap(&registry, &tap_id).await;
            eprintln!(
                "[condor_audio] watcher stopped {} because pid exited",
                tap_id
            );
        }

        match enumerate_audio_sessions() {
            Ok(sessions) => {
                let active_apps = {
                    let guard = registry.lock().await;
                    guard
                        .taps
                        .values()
                        .filter(|tap| tap_is_active(tap))
                        .map(|tap| tap.app_name.clone())
                        .collect::<std::collections::HashSet<_>>()
                };

                if !sessions.is_empty() {
                    eprintln!(
                        "[condor_audio] {} active audio session(s) detected",
                        sessions.len()
                    );
                }

                let discovered_apps = sessions
                    .iter()
                    .filter_map(|session| {
                        session
                            .matched_target
                            .as_ref()
                            .map(|app| (app.clone(), session.pid))
                    })
                    .collect::<Vec<_>>();

                for (app_name, pid) in &discovered_apps {
                    if !active_apps.contains(app_name) {
                        match start_tap(&registry, &config, app_name, *pid, true).await {
                            Ok(tap) => {
                                eprintln!(
                                    "[condor_audio] watcher started {} tap {} for pid {}",
                                    app_name, tap.tap_id, pid
                                );
                            }
                            Err(err) => {
                                eprintln!(
                                    "[condor_audio] watcher failed to start {} tap for pid {}: {}",
                                    app_name, pid, err
                                );
                            }
                        }
                    }
                }

                let stale_app_taps = {
                    let discovered = discovered_apps
                        .iter()
                        .map(|(app_name, _)| app_name.as_str())
                        .collect::<std::collections::HashSet<_>>();
                    let guard = registry.lock().await;
                    guard
                        .taps
                        .values()
                        .filter(|tap| {
                            tap_is_active(tap)
                                && !discovered.contains(tap.app_name.as_str())
                        })
                        .map(|tap| tap.tap_id.clone())
                        .collect::<Vec<_>>()
                };
                for tap_id in stale_app_taps {
                    let _ = stop_tap(&registry, &tap_id).await;
                    eprintln!(
                        "[condor_audio] watcher stopped {} because no matching app was discovered",
                        tap_id
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
