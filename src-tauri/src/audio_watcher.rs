use std::time::Duration;

use crate::audio::{enumerate_audio_sessions, SharedTapRegistry};
use crate::config::AppConfig;

pub async fn run_watcher(config: AppConfig, registry: SharedTapRegistry) {
    let _ = registry;
    eprintln!(
        "[condor_audio] watcher starting: bind={} port={} transport={}",
        config.audio_bind, config.audio_port, config.audio_transport
    );

    loop {
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
