use std::time::Duration;

use crate::audio::{
    enumerate_audio_sessions, process_is_running, stop_tap, SharedTapRegistry, TapStatus,
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
