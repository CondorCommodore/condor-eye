use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use tokio::sync::Mutex;

use crate::config::AppConfig;

pub type SharedTapRegistry = Arc<Mutex<TapRegistry>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioTargetApp {
    pub id: String,
    pub display_name: String,
    pub process_matchers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioSessionInfo {
    pub session_id: String,
    pub pid: u32,
    pub exe_path: String,
    pub display_name: String,
    pub state: String,
    pub matched_target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TapStatus {
    Running,
    Paused,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTap {
    pub tap_id: String,
    pub app_name: String,
    pub target_pid: u32,
    pub include_tree: bool,
    pub started_at: String,
    pub chunks_written: u64,
    pub bytes_captured: u64,
    pub output_dir: String,
    pub status: TapStatus,
    pub status_detail: Option<String>,
    pub last_chunk_path: Option<String>,
    pub last_chunk_ts: Option<String>,
    pub last_transcript_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TapRegistry {
    pub taps: HashMap<String, ActiveTap>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub id: String,
    pub app: String,
    pub created_at: String,
    pub wav_path: Option<String>,
    pub transcript_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioProjectStatus {
    pub supported: bool,
    pub backend: String,
    pub backend_ready: bool,
    pub target_apps: Vec<AudioTargetApp>,
    pub next_step: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioStatusSnapshot {
    pub running: bool,
    pub project: AudioProjectStatus,
    pub active_taps: Vec<ActiveTap>,
    pub audio_bind: String,
    pub audio_port: u16,
    pub audio_transport: String,
    pub audio_output_dir: String,
    pub whisper_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackendState {
    Ready,
    Stubbed,
    Unsupported,
}

pub fn default_target_apps() -> Vec<AudioTargetApp> {
    vec![
        AudioTargetApp {
            id: "zoom".to_string(),
            display_name: "Zoom".to_string(),
            process_matchers: vec!["zoom".to_string()],
        },
        AudioTargetApp {
            id: "discord".to_string(),
            display_name: "Discord".to_string(),
            process_matchers: vec!["discord".to_string()],
        },
    ]
}

pub fn matches_target_process(process_name: &str, target: &AudioTargetApp) -> bool {
    let value = process_name.trim().to_ascii_lowercase();
    target
        .process_matchers
        .iter()
        .any(|matcher| value.contains(&matcher.to_ascii_lowercase()))
}

pub fn match_target_app(exe_path: &str) -> Option<AudioTargetApp> {
    default_target_apps()
        .into_iter()
        .find(|target| matches_target_process(exe_path, target))
}

pub fn capture_backend_state() -> CaptureBackendState {
    if cfg!(target_os = "windows") {
        CaptureBackendState::Stubbed
    } else {
        CaptureBackendState::Unsupported
    }
}

pub fn project_status() -> AudioProjectStatus {
    let (backend, backend_ready, next_step) = match capture_backend_state() {
        CaptureBackendState::Ready => (
            "windows-wasapi".to_string(),
            true,
            "backend ready".to_string(),
        ),
        CaptureBackendState::Stubbed => (
            "windows-wasapi-stubbed".to_string(),
            false,
            "implement live session enumeration and per-process capture".to_string(),
        ),
        CaptureBackendState::Unsupported => (
            "unsupported-platform".to_string(),
            false,
            "run the condor-eye Tauri app on Windows to enable app-audio capture".to_string(),
        ),
    };

    AudioProjectStatus {
        supported: cfg!(target_os = "windows"),
        backend,
        backend_ready,
        target_apps: default_target_apps(),
        next_step,
    }
}

pub async fn status_snapshot(
    config: &AppConfig,
    registry: &SharedTapRegistry,
) -> AudioStatusSnapshot {
    let taps = registry.lock().await.taps.values().cloned().collect::<Vec<_>>();
    AudioStatusSnapshot {
        running: true,
        project: project_status(),
        active_taps: taps,
        audio_bind: config.audio_bind.clone(),
        audio_port: config.audio_port,
        audio_transport: config.audio_transport.clone(),
        audio_output_dir: config.audio_output_dir.clone(),
        whisper_url: config.whisper_url.clone(),
    }
}

pub fn ensure_audio_dirs(config: &AppConfig) -> Result<(), String> {
    let root = Path::new(&config.audio_output_dir);
    fs::create_dir_all(root.join("wav")).map_err(|e| format!("create wav dir: {}", e))?;
    fs::create_dir_all(root.join("transcripts")).map_err(|e| format!("create transcripts dir: {}", e))?;
    Ok(())
}

pub fn audio_wav_dir(config: &AppConfig) -> PathBuf {
    Path::new(&config.audio_output_dir).join("wav")
}

pub fn audio_transcript_dir(config: &AppConfig) -> PathBuf {
    Path::new(&config.audio_output_dir).join("transcripts")
}

pub(crate) fn display_name_from_exe_path(exe_path: &str) -> String {
    // Works with both Windows (backslash) and Unix (forward-slash) paths.
    let basename = exe_path
        .rsplit(|c: char| c == '/' || c == '\\')
        .next()
        .unwrap_or(exe_path);
    basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".EXE"))
        .unwrap_or(basename)
        .to_string()
}

#[cfg(target_os = "windows")]
fn exe_path_from_pid(pid: u32) -> String {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return format!("pid:{pid}");
        }
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        if ok != 0 {
            String::from_utf16_lossy(&buf[..size as usize])
        } else {
            format!("pid:{pid}")
        }
    }
}

#[cfg(target_os = "windows")]
pub fn enumerate_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    use wasapi::{DeviceEnumerator, Direction};

    // S_OK=0 (newly initialized) and S_FALSE=1 (already MTA) are both fine.
    let _ = wasapi::initialize_mta();

    let dev_enum = DeviceEnumerator::new()
        .map_err(|e| format!("DeviceEnumerator::new: {e}"))?;
    let collection = dev_enum
        .get_device_collection(&Direction::Render)
        .map_err(|e| format!("get_device_collection: {e}"))?;

    let mut sessions: Vec<AudioSessionInfo> = Vec::new();
    let mut seen_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for device_result in &collection {
        let device = match device_result {
            Ok(d) => d,
            Err(_) => continue,
        };
        let manager = match device.get_iaudiosessionmanager() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let session_enum = match manager.get_audiosessionenumerator() {
            Ok(e) => e,
            Err(_) => continue,
        };
        let count = match session_enum.get_count() {
            Ok(c) => c,
            Err(_) => continue,
        };
        for i in 0..count {
            let control = match session_enum.get_session(i) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let pid = match control.get_process_id() {
                Ok(p) => p,
                Err(_) => continue,
            };
            if pid == 0 || !seen_pids.insert(pid) {
                continue;
            }
            let state_str = control
                .get_state()
                .map(|s| format!("{s:?}").to_lowercase())
                .unwrap_or_else(|_| "unknown".to_string());
            let exe_path = exe_path_from_pid(pid);
            let display_name = display_name_from_exe_path(&exe_path);
            let matched_target = match_target_app(&exe_path).map(|t| t.id);
            sessions.push(AudioSessionInfo {
                session_id: format!("{pid}"),
                pid,
                exe_path,
                display_name,
                state: state_str,
                matched_target,
            });
        }
    }

    Ok(sessions)
}

#[cfg(not(target_os = "windows"))]
pub fn enumerate_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    Err("Audio session enumeration is only supported on Windows".to_string())
}

pub async fn start_tap(
    registry: &SharedTapRegistry,
    config: &AppConfig,
    app_name: &str,
    pid: u32,
    include_tree: bool,
) -> Result<ActiveTap, String> {
    ensure_audio_dirs(config)?;
    match capture_backend_state() {
        CaptureBackendState::Ready => {}
        CaptureBackendState::Stubbed => {
            return Err("Audio tap start is not implemented yet in this build".to_string());
        }
        CaptureBackendState::Unsupported => {
            return Err("Audio tap start is only supported on Windows".to_string());
        }
    }

    let tap_id = format!("{}-{}", app_name, unix_millis());
    let tap = ActiveTap {
        tap_id: tap_id.clone(),
        app_name: app_name.to_string(),
        target_pid: pid,
        include_tree,
        started_at: now_rfc3339(),
        chunks_written: 0,
        bytes_captured: 0,
        output_dir: config.audio_output_dir.clone(),
        status: TapStatus::Running,
        status_detail: None,
        last_chunk_path: None,
        last_chunk_ts: None,
        last_transcript_path: None,
    };

    registry.lock().await.taps.insert(tap_id, tap.clone());
    Ok(tap)
}

pub async fn stop_tap(registry: &SharedTapRegistry, tap_id: &str) -> Result<ActiveTap, String> {
    let mut guard = registry.lock().await;
    let tap = guard
        .taps
        .get_mut(tap_id)
        .ok_or_else(|| format!("Tap not found: {}", tap_id))?;
    tap.status = TapStatus::Stopped;
    tap.status_detail = None;
    Ok(tap.clone())
}

pub async fn get_tap(registry: &SharedTapRegistry, tap_id: &str) -> Option<ActiveTap> {
    registry.lock().await.taps.get(tap_id).cloned()
}

pub fn list_transcripts(
    config: &AppConfig,
    app: Option<&str>,
    since: Option<&str>,
) -> Result<Vec<TranscriptEntry>, String> {
    ensure_audio_dirs(config)?;
    let transcript_dir = audio_transcript_dir(config);
    let since_ts = parse_since(since)?;
    let mut items = vec![];

    if !transcript_dir.exists() {
        return Ok(items);
    }

    let entries = fs::read_dir(&transcript_dir).map_err(|e| format!("read transcripts dir: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("txt") {
            continue;
        }

        let filename = match path.file_name().and_then(|v| v.to_str()) {
            Some(value) => value.to_string(),
            None => continue,
        };
        let Some((entry_app, created_at)) = parse_timestamped_name(&filename) else {
            continue;
        };

        if let Some(filter) = app {
            if entry_app != filter {
                continue;
            }
        }

        if let Some(since_filter) = since_ts {
            if created_at < since_filter {
                continue;
            }
        }

        let wav_path = audio_wav_dir(config).join(filename.replace(".txt", ".wav"));
        items.push(TranscriptEntry {
            id: filename.clone(),
            app: entry_app.to_string(),
            created_at: created_at.format(&Rfc3339).unwrap_or_else(|_| created_at.to_string()),
            wav_path: if wav_path.exists() {
                Some(wav_path.to_string_lossy().into_owned())
            } else {
                None
            },
            transcript_path: path.to_string_lossy().into_owned(),
        });
    }

    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(items)
}

pub fn read_transcript(config: &AppConfig, transcript_id: &str) -> Result<String, String> {
    let path = audio_transcript_dir(config).join(transcript_id);
    fs::read_to_string(path).map_err(|e| format!("read transcript: {}", e))
}

pub fn latest_chunk_bytes(tap: &ActiveTap) -> Result<Vec<u8>, String> {
    let path = tap
        .last_chunk_path
        .as_ref()
        .ok_or_else(|| format!("Tap {} has no chunk yet", tap.tap_id))?;
    fs::read(path).map_err(|e| format!("read latest chunk: {}", e))
}

pub fn latest_transcript_text(tap: &ActiveTap) -> Result<String, String> {
    let path = tap
        .last_transcript_path
        .as_ref()
        .ok_or_else(|| format!("Tap {} has no transcript yet", tap.tap_id))?;
    fs::read_to_string(path).map_err(|e| format!("read latest transcript: {}", e))
}

fn parse_since(since: Option<&str>) -> Result<Option<OffsetDateTime>, String> {
    match since {
        Some(value) => OffsetDateTime::parse(value, &Rfc3339)
            .map(Some)
            .map_err(|e| format!("invalid since timestamp '{}': {}", value, e)),
        None => Ok(None),
    }
}

fn parse_timestamped_name(filename: &str) -> Option<(&str, OffsetDateTime)> {
    let stem = filename.strip_suffix(".txt").or_else(|| filename.strip_suffix(".wav"))?;
    let (app, ts) = stem.split_once('_')?;
    let ts = ts.replace('T', "T");
    let iso = if ts.len() == 15 {
        format!(
            "{}-{}-{}T{}:{}:{}Z",
            &ts[0..4],
            &ts[4..6],
            &ts[6..8],
            &ts[9..11],
            &ts[11..13],
            &ts[13..15]
        )
    } else {
        return None;
    };
    let parsed = OffsetDateTime::parse(&iso, &Rfc3339).ok()?;
    Some((app, parsed))
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_matcher_is_case_insensitive() {
        let target = &default_target_apps()[0];
        assert!(matches_target_process("Zoom.exe", target));
        assert!(matches_target_process("C:\\Program Files\\Zoom\\bin\\zoom.exe", target));
    }

    #[test]
    fn discord_matcher_matches_helper_processes() {
        let target = &default_target_apps()[1];
        assert!(matches_target_process("Discord.exe", target));
        assert!(matches_target_process("discord_canary.exe", target));
    }

    #[test]
    fn unrelated_process_does_not_match() {
        let target = &default_target_apps()[0];
        assert!(!matches_target_process("chrome.exe", target));
    }

    #[test]
    fn parse_timestamped_text_name() {
        let parsed = parse_timestamped_name("zoom_20260324T143000.txt").expect("timestamped name");
        assert_eq!(parsed.0, "zoom");
        assert_eq!(parsed.1.year(), 2026);
        assert_eq!(u8::from(parsed.1.month()), 3);
        assert_eq!(parsed.1.day(), 24);
    }

    #[test]
    fn match_target_app_from_path() {
        let target = match_target_app("C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0.0\\Discord.exe")
            .expect("discord target");
        assert_eq!(target.id, "discord");
    
    #[test]
    fn display_name_strips_windows_path_and_extension() {
        assert_eq!(
            display_name_from_exe_path(r"C:\Program Files\Zoomin\Zoom.exe"),
            "Zoom"
        );
    }

    #[test]
    fn display_name_strips_unix_path_and_extension() {
        assert_eq!(
            display_name_from_exe_path("/usr/bin/zoom.exe"),
            "zoom"
        );
    }

    #[test]
    fn display_name_bare_name_no_extension() {
        assert_eq!(display_name_from_exe_path("discord"), "discord");
    }

    #[test]
    fn display_name_bare_exe_no_path() {
        assert_eq!(display_name_from_exe_path("Discord.exe"), "Discord");
    }

    #[test]
    fn display_name_pid_fallback_unchanged() {
        assert_eq!(display_name_from_exe_path("pid:1234"), "pid:1234");
    }

    #[test]
    fn parse_timestamped_wav_name() {
        let parsed = parse_timestamped_name("discord_20260324T090000.wav")
            .expect("timestamped wav name");
        assert_eq!(parsed.0, "discord");
        assert_eq!(parsed.1.year(), 2026);
    }

    #[test]
    fn parse_timestamped_name_bad_format_returns_none() {
        assert!(parse_timestamped_name("nodash.txt").is_none());
        assert!(parse_timestamped_name("app_badts.txt").is_none());
        assert!(parse_timestamped_name("").is_none());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn enumerate_sessions_returns_error_on_non_windows() {
        let result = enumerate_audio_sessions();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only supported on Windows"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn capture_backend_unsupported_on_non_windows() {
        assert_eq!(capture_backend_state(), CaptureBackendState::Unsupported);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn project_status_not_supported_on_non_windows() {
        let status = project_status();
        assert!(!status.supported);
        assert!(!status.backend_ready);
        assert_eq!(status.backend, "unsupported-platform");
    }

    #[test]
    fn default_target_apps_has_zoom_and_discord() {
        let apps = default_target_apps();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].id, "zoom");
        assert_eq!(apps[1].id, "discord");
    }
}
