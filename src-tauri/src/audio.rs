use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(target_os = "windows")]
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
#[cfg(target_os = "windows")]
use std::time::Duration;
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

pub fn tap_is_active(tap: &ActiveTap) -> bool {
    tap.status != TapStatus::Stopped
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveTap {
    pub tap_id: String,
    pub app_name: String,
    pub target_pid: u32,
    pub include_tree: bool,
    pub started_at: String,
    pub chunk_seconds: u16,
    pub stitch_ms: u16,
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
    #[serde(skip)]
    pub controls: HashMap<String, TapRuntime>,
}

#[derive(Debug, Clone)]
pub struct TapRuntime {
    pub stop_flag: Arc<AtomicBool>,
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
    pub chunk_seconds: u16,
    pub stitch_ms: u16,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioChunkPlan {
    pub chunk_seconds: u16,
    pub stitch_ms: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioChunkWindow {
    pub chunk_index: u64,
    pub nominal_start_ms: u64,
    pub nominal_end_ms: u64,
    pub capture_start_ms: u64,
    pub capture_end_ms: u64,
    pub stitch_ms: u16,
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
        CaptureBackendState::Ready
    } else {
        CaptureBackendState::Unsupported
    }
}

pub fn chunk_plan(config: &AppConfig) -> AudioChunkPlan {
    let chunk_seconds = config.audio_chunk_seconds.max(1);
    let max_stitch_ms = u32::from(chunk_seconds) * 1000;
    let stitch_ms = u16::try_from(u32::from(config.audio_stitch_ms).min(max_stitch_ms))
        .unwrap_or(chunk_seconds * 1000);
    AudioChunkPlan {
        chunk_seconds,
        stitch_ms,
    }
}

pub fn chunk_window(plan: AudioChunkPlan, chunk_index: u64) -> AudioChunkWindow {
    let chunk_ms = u64::from(plan.chunk_seconds) * 1000;
    let nominal_start_ms = chunk_index.saturating_mul(chunk_ms);
    let nominal_end_ms = nominal_start_ms.saturating_add(chunk_ms);
    let capture_start_ms = nominal_start_ms.saturating_sub(u64::from(plan.stitch_ms));
    AudioChunkWindow {
        chunk_index,
        nominal_start_ms,
        nominal_end_ms,
        capture_start_ms,
        capture_end_ms: nominal_end_ms,
        stitch_ms: plan.stitch_ms,
    }
}

pub fn project_status(config: &AppConfig) -> AudioProjectStatus {
    let plan = chunk_plan(config);
    let (backend, backend_ready, next_step) = match capture_backend_state() {
        CaptureBackendState::Ready => (
            "windows-wasapi".to_string(),
            true,
            "backend ready for manual taps; watcher still depends on process discovery".to_string(),
        ),
        CaptureBackendState::Stubbed => (
            "windows-wasapi-stubbed".to_string(),
            false,
            "implement live session enumeration, per-process capture, and stitched chunk writer"
                .to_string(),
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
        chunk_seconds: plan.chunk_seconds,
        stitch_ms: plan.stitch_ms,
        target_apps: default_target_apps(),
        next_step,
    }
}

pub async fn status_snapshot(
    config: &AppConfig,
    registry: &SharedTapRegistry,
) -> AudioStatusSnapshot {
    let taps = registry
        .lock()
        .await
        .taps
        .values()
        .cloned()
        .collect::<Vec<_>>();
    AudioStatusSnapshot {
        running: true,
        project: project_status(config),
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
    fs::create_dir_all(root.join("transcripts"))
        .map_err(|e| format!("create transcripts dir: {}", e))?;
    Ok(())
}

pub fn audio_wav_dir(config: &AppConfig) -> PathBuf {
    Path::new(&config.audio_output_dir).join("wav")
}

pub fn audio_transcript_dir(config: &AppConfig) -> PathBuf {
    Path::new(&config.audio_output_dir).join("transcripts")
}

pub fn enumerate_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    match capture_backend_state() {
        CaptureBackendState::Ready => enumerate_process_sessions(),
        CaptureBackendState::Stubbed => Err("Windows audio backend is scaffolded but session enumeration is not implemented in this build".to_string()),
        CaptureBackendState::Unsupported => Err("Audio session enumeration is only supported on Windows".to_string()),
    }
}

pub async fn start_tap(
    registry: &SharedTapRegistry,
    config: &AppConfig,
    app_name: &str,
    pid: u32,
    include_tree: bool,
) -> Result<ActiveTap, String> {
    ensure_audio_dirs(config)?;
    let plan = chunk_plan(config);
    match capture_backend_state() {
        CaptureBackendState::Ready => {}
        CaptureBackendState::Stubbed => {
            return Err("Audio tap start is not implemented yet in this build".to_string());
        }
        CaptureBackendState::Unsupported => {
            return Err("Audio tap start is only supported on Windows".to_string());
        }
    }

    let resolved_pid = if pid == 0 {
        resolve_target_pid(app_name)?
    } else {
        pid
    };

    let mut guard = registry.lock().await;
    if let Some(existing) = guard
        .taps
        .values()
        .find(|tap| tap.app_name == app_name && tap_is_active(tap))
        .cloned()
    {
        return Ok(existing);
    }

    let tap_id = format!("{}-{}", app_name, unix_millis());
    let tap = ActiveTap {
        tap_id: tap_id.clone(),
        app_name: app_name.to_string(),
        target_pid: resolved_pid,
        include_tree,
        started_at: now_rfc3339(),
        chunk_seconds: plan.chunk_seconds,
        stitch_ms: plan.stitch_ms,
        chunks_written: 0,
        bytes_captured: 0,
        output_dir: config.audio_output_dir.clone(),
        status: TapStatus::Running,
        status_detail: None,
        last_chunk_path: None,
        last_chunk_ts: None,
        last_transcript_path: None,
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    guard.taps.insert(tap_id.clone(), tap.clone());
    guard.controls.insert(
        tap_id.clone(),
        TapRuntime {
            stop_flag: stop_flag.clone(),
        },
    );
    drop(guard);

    spawn_tap_worker(
        registry.clone(),
        config.clone(),
        tap_id.clone(),
        app_name.to_string(),
        resolved_pid,
        include_tree,
        stop_flag,
    );
    Ok(tap)
}

pub async fn stop_tap(registry: &SharedTapRegistry, tap_id: &str) -> Result<ActiveTap, String> {
    let mut guard = registry.lock().await;
    let runtime = guard.controls.get(tap_id).cloned();
    let tap = guard
        .taps
        .get_mut(tap_id)
        .ok_or_else(|| format!("Tap not found: {}", tap_id))?;
    if let Some(runtime) = runtime {
        runtime.stop_flag.store(true, Ordering::Relaxed);
    }
    tap.status = TapStatus::Stopped;
    tap.status_detail = None;
    Ok(tap.clone())
}

pub async fn get_tap(registry: &SharedTapRegistry, tap_id: &str) -> Option<ActiveTap> {
    registry.lock().await.taps.get(tap_id).cloned()
}

fn spawn_tap_worker(
    registry: SharedTapRegistry,
    config: AppConfig,
    tap_id: String,
    app_name: String,
    pid: u32,
    include_tree: bool,
    stop_flag: Arc<AtomicBool>,
) {
    let runtime = tokio::runtime::Handle::current();
    thread::spawn(move || {
        let result = capture_chunks(
            &runtime,
            registry.clone(),
            &config,
            &tap_id,
            &app_name,
            pid,
            include_tree,
            stop_flag,
        );

        if let Err(error) = result {
            runtime.block_on(update_tap_error(&registry, &tap_id, &error));
        }

        runtime.block_on(finalize_tap_stop(&registry, &tap_id));
    });
}

fn capture_chunks(
    runtime: &tokio::runtime::Handle,
    registry: SharedTapRegistry,
    config: &AppConfig,
    tap_id: &str,
    app_name: &str,
    pid: u32,
    include_tree: bool,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return capture_chunks_windows(
            runtime,
            registry,
            config,
            tap_id,
            app_name,
            pid,
            include_tree,
            stop_flag,
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            runtime,
            registry,
            config,
            tap_id,
            app_name,
            pid,
            include_tree,
            stop_flag,
        );
        Err("audio capture is only supported on Windows".to_string())
    }
}

async fn update_tap_after_chunk(
    registry: &SharedTapRegistry,
    tap_id: &str,
    wav_path: &Path,
    transcript_path: Option<&Path>,
    bytes_written: u64,
    chunk_ts: &str,
    status_detail: Option<&str>,
) {
    let mut guard = registry.lock().await;
    if let Some(tap) = guard.taps.get_mut(tap_id) {
        tap.chunks_written += 1;
        tap.bytes_captured += bytes_written;
        tap.last_chunk_path = Some(wav_path.to_string_lossy().into_owned());
        tap.last_chunk_ts = Some(chunk_ts.to_string());
        tap.last_transcript_path = transcript_path.map(|path| path.to_string_lossy().into_owned());
        tap.status = TapStatus::Running;
        tap.status_detail = status_detail.map(|value| value.to_string());
    }
}

async fn update_tap_error(registry: &SharedTapRegistry, tap_id: &str, error: &str) {
    let mut guard = registry.lock().await;
    if let Some(tap) = guard.taps.get_mut(tap_id) {
        tap.status = TapStatus::Error;
        tap.status_detail = Some(error.to_string());
    }
}

async fn finalize_tap_stop(registry: &SharedTapRegistry, tap_id: &str) {
    let mut guard = registry.lock().await;
    guard.controls.remove(tap_id);
    if let Some(tap) = guard.taps.get_mut(tap_id) {
        if tap.status != TapStatus::Error {
            tap.status = TapStatus::Stopped;
            tap.status_detail = None;
        }
    }
}

fn transcript_file_name(app_name: &str, chunk_started_at: OffsetDateTime) -> String {
    format!(
        "{}_{}{:02}{:02}T{:02}{:02}{:02}.txt",
        app_name,
        chunk_started_at.year(),
        u8::from(chunk_started_at.month()),
        chunk_started_at.day(),
        chunk_started_at.hour(),
        chunk_started_at.minute(),
        chunk_started_at.second()
    )
}

fn wav_file_name(app_name: &str, chunk_started_at: OffsetDateTime) -> String {
    transcript_file_name(app_name, chunk_started_at).replace(".txt", ".wav")
}

fn apply_retention(config: &AppConfig, app_name: &str, keep: usize) -> Result<(), String> {
    prune_old_files(&audio_wav_dir(config), "wav", app_name, keep)?;
    prune_old_files(&audio_transcript_dir(config), "txt", app_name, keep)?;
    Ok(())
}

fn prune_old_files(dir: &Path, ext: &str, app_name: &str, keep: usize) -> Result<(), String> {
    let mut entries = vec![];
    let rd = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some(ext) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with(&format!("{app_name}_")) {
            continue;
        }
        entries.push(path);
    }
    entries.sort();
    if entries.len() <= keep {
        return Ok(());
    }
    let remove_count = entries.len().saturating_sub(keep);
    for path in entries.into_iter().take(remove_count) {
        fs::remove_file(&path).map_err(|e| format!("remove {}: {}", path.display(), e))?;
    }
    Ok(())
}

async fn transcribe_wav(config: &AppConfig, wav_path: &Path) -> Result<String, String> {
    if config.audio_transport != "http" {
        return Err(format!(
            "AUDIO_TRANSPORT={} is not implemented; use http",
            config.audio_transport
        ));
    }

    let bytes = tokio::fs::read(wav_path)
        .await
        .map_err(|e| format!("read wav {}: {}", wav_path.display(), e))?;
    let file_name = wav_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("chunk.wav")
        .to_string();
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name)
        .mime_str("audio/wav")
        .map_err(|e| format!("mime: {}", e))?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let client = reqwest::Client::new();
    let response = client
        .post(&config.whisper_url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("whisper request: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("read whisper response: {}", e))?;
    if !status.is_success() {
        return Err(format!("whisper status {}: {}", status, body));
    }
    extract_whisper_text(&body)
}

fn extract_whisper_text(body: &str) -> Result<String, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(text) = value.get("text").and_then(|text| text.as_str()) {
            return Ok(text.trim().to_string());
        }
        if let Some(text) = value.get("result").and_then(|text| text.as_str()) {
            return Ok(text.trim().to_string());
        }
    }
    Ok(trimmed.to_string())
}

#[cfg(target_os = "windows")]
fn enumerate_process_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    #[derive(Deserialize)]
    struct ProcessEntry {
        #[serde(rename = "Id")]
        id: u32,
        #[serde(rename = "ProcessName")]
        process_name: String,
        #[serde(rename = "Path")]
        path: Option<String>,
    }

    let script = "Get-Process | Where-Object { $_.Path -and ($_.Path -match 'zoom' -or $_.Path -match 'discord' -or $_.ProcessName -match 'zoom' -or $_.ProcessName -match 'discord') } | Select-Object Id,ProcessName,Path | ConvertTo-Json -Compress";
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .output()
        .map_err(|e| format!("powershell enumerate: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "powershell enumerate failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() || stdout == "null" {
        return Ok(vec![]);
    }

    let entries: Vec<ProcessEntry> = match serde_json::from_str::<Vec<ProcessEntry>>(&stdout) {
        Ok(items) => items,
        Err(_) => {
            let one = serde_json::from_str::<ProcessEntry>(&stdout)
                .map_err(|e| format!("parse powershell process JSON: {}", e))?;
            vec![one]
        }
    };

    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let exe_path = entry.path.unwrap_or_else(|| entry.process_name.clone());
            let matched = match_target_app(&exe_path)?;
            Some(AudioSessionInfo {
                session_id: format!("proc-{}", entry.id),
                pid: entry.id,
                exe_path,
                display_name: entry.process_name,
                state: "process_running".to_string(),
                matched_target: Some(matched.id),
            })
        })
        .collect())
}

#[cfg(not(target_os = "windows"))]
fn enumerate_process_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    Err("Audio session enumeration is only supported on Windows".to_string())
}

#[cfg(target_os = "windows")]
fn resolve_target_pid(app_name: &str) -> Result<u32, String> {
    let sessions = enumerate_process_sessions()?;
    sessions
        .into_iter()
        .find(|session| session.matched_target.as_deref() == Some(app_name))
        .map(|session| session.pid)
        .ok_or_else(|| format!("No running process found for {}", app_name))
}

#[cfg(not(target_os = "windows"))]
fn resolve_target_pid(app_name: &str) -> Result<u32, String> {
    Err(format!(
        "No running process found for {} on this platform",
        app_name
    ))
}

#[cfg(target_os = "windows")]
fn capture_chunks_windows(
    runtime: &tokio::runtime::Handle,
    registry: SharedTapRegistry,
    config: &AppConfig,
    tap_id: &str,
    app_name: &str,
    pid: u32,
    include_tree: bool,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    use wasapi::{AudioClient, Direction, SampleType, ShareMode, WaveFormat};

    wasapi::initialize_mta().map_err(|e| format!("initialize_mta: {}", e))?;

    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
    let blockalign = desired_format.get_blockalign() as usize;
    let sample_rate = 48_000usize;
    let plan = chunk_plan(config);
    let chunk_frames = usize::from(plan.chunk_seconds) * sample_rate;
    let preroll_frames = usize::from(plan.stitch_ms) * sample_rate / 1000;
    let mut audio_client = AudioClient::new_application_loopback_client(pid, include_tree)
        .map_err(|e| format!("loopback client: {}", e))?;
    audio_client
        .initialize_client(
            &desired_format,
            0,
            &Direction::Capture,
            &ShareMode::Shared,
            true,
        )
        .map_err(|e| format!("initialize capture client: {}", e))?;
    let h_event = audio_client
        .set_get_eventhandle()
        .map_err(|e| format!("set event handle: {}", e))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|e| format!("get capture client: {}", e))?;
    audio_client
        .start_stream()
        .map_err(|e| format!("start stream: {}", e))?;

    let started_ms = unix_millis() as i64;
    let mut queue: VecDeque<u8> = VecDeque::new();
    let mut nominal_samples: Vec<i16> = Vec::with_capacity(chunk_frames);
    let mut last_tail: Vec<i16> = Vec::with_capacity(preroll_frames);
    let mut chunk_index: u64 = 0;

    loop {
        if stop_flag.load(Ordering::Relaxed) || !process_is_running(pid) {
            break;
        }

        let new_frames = capture_client
            .get_next_nbr_frames()
            .map_err(|e| format!("get next frames: {}", e))?
            .unwrap_or(0);
        let additional = (new_frames as usize * blockalign)
            .saturating_sub(queue.capacity().saturating_sub(queue.len()));
        queue.reserve(additional);
        if new_frames > 0 {
            capture_client
                .read_from_device_to_deque(&mut queue)
                .map_err(|e| format!("read capture queue: {}", e))?;
        }

        while queue.len() >= blockalign {
            let mut frame = [0u8; 8];
            for byte in &mut frame {
                *byte = queue.pop_front().unwrap_or_default();
            }
            let left = f32::from_le_bytes(frame[0..4].try_into().unwrap_or([0, 0, 0, 0]));
            let right = f32::from_le_bytes(frame[4..8].try_into().unwrap_or([0, 0, 0, 0]));
            nominal_samples.push(float_stereo_to_pcm16(left, right));

            if nominal_samples.len() >= chunk_frames {
                flush_chunk(
                    runtime,
                    registry.clone(),
                    config,
                    tap_id,
                    app_name,
                    started_ms,
                    chunk_index,
                    &nominal_samples,
                    &last_tail,
                )?;
                chunk_index += 1;
                last_tail = tail_samples(&nominal_samples, preroll_frames);
                nominal_samples.clear();
            }
        }

        if h_event.wait_for_event(250).is_err() {
            thread::sleep(Duration::from_millis(50));
        }
    }

    audio_client.stop_stream().ok();

    if !nominal_samples.is_empty() {
        flush_chunk(
            runtime,
            registry,
            config,
            tap_id,
            app_name,
            started_ms,
            chunk_index,
            &nominal_samples,
            &last_tail,
        )?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn flush_chunk(
    runtime: &tokio::runtime::Handle,
    registry: SharedTapRegistry,
    config: &AppConfig,
    tap_id: &str,
    app_name: &str,
    started_ms: i64,
    chunk_index: u64,
    nominal_samples: &[i16],
    last_tail: &[i16],
) -> Result<(), String> {
    if nominal_samples.is_empty() {
        return Ok(());
    }

    let chunk_started_at = chunk_started_at(started_ms, config.audio_chunk_seconds, chunk_index)?;
    let wav_name = wav_file_name(app_name, chunk_started_at);
    let txt_name = transcript_file_name(app_name, chunk_started_at);
    let wav_path = audio_wav_dir(config).join(&wav_name);
    let transcript_path = audio_transcript_dir(config).join(&txt_name);

    let mut stitched = Vec::with_capacity(last_tail.len() + nominal_samples.len());
    if chunk_index > 0 {
        stitched.extend_from_slice(last_tail);
    }
    stitched.extend_from_slice(nominal_samples);

    write_wav(&wav_path, &stitched, 48_000)?;

    let transcript_result = runtime.block_on(transcribe_wav(config, &wav_path));
    let (transcript_path_opt, status_detail) = match transcript_result {
        Ok(text) => {
            fs::write(&transcript_path, &text)
                .map_err(|e| format!("write transcript {}: {}", transcript_path.display(), e))?;
            let client = reqwest::Client::new();
            runtime.block_on(notify_condor_intel(
                &client,
                config,
                &txt_name,
                app_name,
                &text,
                Some(
                    &chunk_started_at
                        .format(&Rfc3339)
                        .unwrap_or_else(|_| now_rfc3339()),
                ),
            ));
            (Some(transcript_path.clone()), None)
        }
        Err(error) => (None, Some(format!("transcription failed: {}", error))),
    };

    apply_retention(config, app_name, 360)?;

    runtime.block_on(update_tap_after_chunk(
        &registry,
        tap_id,
        &wav_path,
        transcript_path_opt.as_deref(),
        (stitched.len() * std::mem::size_of::<i16>()) as u64,
        &chunk_started_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| now_rfc3339()),
        status_detail.as_deref(),
    ));

    Ok(())
}

#[cfg(target_os = "windows")]
fn write_wav(path: &Path, samples: &[i16], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| format!("create wav {}: {}", path.display(), e))?;
    for sample in samples {
        writer
            .write_sample(*sample)
            .map_err(|e| format!("write wav sample {}: {}", path.display(), e))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("finalize wav {}: {}", path.display(), e))
}

#[cfg(target_os = "windows")]
fn float_stereo_to_pcm16(left: f32, right: f32) -> i16 {
    let mono = ((left + right) / 2.0).clamp(-1.0, 1.0);
    (mono * i16::MAX as f32) as i16
}

#[cfg(target_os = "windows")]
fn tail_samples(samples: &[i16], count: usize) -> Vec<i16> {
    if count == 0 || samples.is_empty() {
        return vec![];
    }
    let start = samples.len().saturating_sub(count);
    samples[start..].to_vec()
}

#[cfg(target_os = "windows")]
fn chunk_started_at(
    started_ms: i64,
    chunk_seconds: u16,
    chunk_index: u64,
) -> Result<OffsetDateTime, String> {
    let offset_ms = i64::from(chunk_seconds) * 1000 * i64::try_from(chunk_index).unwrap_or(0);
    let timestamp_ms = started_ms.saturating_add(offset_ms);
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(timestamp_ms) * 1_000_000)
        .map_err(|e| format!("chunk timestamp: {}", e))
}

#[cfg(target_os = "windows")]
pub(crate) fn process_is_running(pid: u32) -> bool {
    let script = format!("$p = Get-Process -Id {} -ErrorAction SilentlyContinue; if ($p) {{ exit 0 }} else {{ exit 1 }}", pid);
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn process_is_running(_pid: u32) -> bool {
    false
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

    let entries =
        fs::read_dir(&transcript_dir).map_err(|e| format!("read transcripts dir: {}", e))?;
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
            created_at: created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| created_at.to_string()),
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
    let stem = filename
        .strip_suffix(".txt")
        .or_else(|| filename.strip_suffix(".wav"))?;
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

/// POST a completed transcript to condor-intel sidecar for gate + extraction.
/// Fire-and-forget: logs errors but does not fail the caller.
pub async fn notify_condor_intel(
    client: &reqwest::Client,
    config: &AppConfig,
    transcript_id: &str,
    app: &str,
    text: &str,
    chunk_started_at: Option<&str>,
) {
    let plan = chunk_plan(config);
    let url = format!(
        "{}/ingest/condor-audio",
        config.condor_intel_url.trim_end_matches('/')
    );
    let payload = serde_json::json!({
        "transcript_id": transcript_id,
        "app": app,
        "text": text,
        "chunk_started_at": chunk_started_at,
        "chunk_seconds": plan.chunk_seconds,
        "stitch_ms": plan.stitch_ms,
        "source": "condor_audio",
    });
    match client.post(&url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => {
            eprintln!(
                "[condor_audio] condor-intel accepted transcript {}",
                transcript_id
            );
        }
        Ok(resp) => {
            eprintln!(
                "[condor_audio] condor-intel rejected transcript {} — status {}",
                transcript_id,
                resp.status()
            );
        }
        Err(e) => {
            eprintln!(
                "[condor_audio] condor-intel POST failed for {}: {}",
                transcript_id, e
            );
        }
    }
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
    use crate::config::AppConfig;

    fn test_config() -> AppConfig {
        AppConfig {
            api_key: String::new(),
            redis_url: String::new(),
            model: String::new(),
            discord_bridge_url: None,
            coord_api_url: String::new(),
            coord_api_token: String::new(),
            condor_eye_bind: "0.0.0.0".to_string(),
            condor_eye_port: 9050,
            audio_bind: "127.0.0.1".to_string(),
            audio_port: 9051,
            audio_output_dir: "/tmp/condor-audio".to_string(),
            audio_transport: "http".to_string(),
            whisper_url: "http://localhost:8080/inference".to_string(),
            audio_chunk_seconds: 10,
            audio_stitch_ms: 1500,
            condor_intel_url: "http://localhost:8791".to_string(),
        }
    }

    #[test]
    fn zoom_matcher_is_case_insensitive() {
        let target = &default_target_apps()[0];
        assert!(matches_target_process("Zoom.exe", target));
        assert!(matches_target_process(
            "C:\\Program Files\\Zoom\\bin\\zoom.exe",
            target
        ));
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
        let target =
            match_target_app("C:\\Users\\me\\AppData\\Local\\Discord\\app-1.0.0\\Discord.exe")
                .expect("discord target");
        assert_eq!(target.id, "discord");
    }

    #[test]
    fn chunk_plan_uses_expected_defaults() {
        let plan = chunk_plan(&test_config());
        assert_eq!(plan.chunk_seconds, 10);
        assert_eq!(plan.stitch_ms, 1500);
    }

    #[test]
    fn chunk_plan_clamps_stitch_to_chunk_length() {
        let mut config = test_config();
        config.audio_stitch_ms = 15_000;
        let plan = chunk_plan(&config);
        assert_eq!(plan.chunk_seconds, 10);
        assert_eq!(plan.stitch_ms, 10_000);
    }

    #[test]
    fn first_chunk_does_not_go_negative() {
        let plan = AudioChunkPlan {
            chunk_seconds: 10,
            stitch_ms: 1500,
        };
        let window = chunk_window(plan, 0);
        assert_eq!(window.nominal_start_ms, 0);
        assert_eq!(window.capture_start_ms, 0);
        assert_eq!(window.capture_end_ms, 10_000);
    }

    #[test]
    fn later_chunks_include_preroll_overlap() {
        let plan = AudioChunkPlan {
            chunk_seconds: 10,
            stitch_ms: 1500,
        };
        let window = chunk_window(plan, 2);
        assert_eq!(window.nominal_start_ms, 20_000);
        assert_eq!(window.capture_start_ms, 18_500);
        assert_eq!(window.capture_end_ms, 30_000);
    }

    #[test]
    fn transcription_failure_keeps_tap_running() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let registry = Arc::new(Mutex::new(TapRegistry::default()));
            {
                let mut guard = registry.lock().await;
                guard.taps.insert(
                    "zoom-test".to_string(),
                    ActiveTap {
                        tap_id: "zoom-test".to_string(),
                        app_name: "zoom".to_string(),
                        target_pid: 1234,
                        include_tree: true,
                        started_at: now_rfc3339(),
                        chunk_seconds: 10,
                        stitch_ms: 1500,
                        chunks_written: 0,
                        bytes_captured: 0,
                        output_dir: "/tmp/condor-audio".to_string(),
                        status: TapStatus::Running,
                        status_detail: None,
                        last_chunk_path: None,
                        last_chunk_ts: None,
                        last_transcript_path: None,
                    },
                );
            }

            update_tap_after_chunk(
                &registry,
                "zoom-test",
                Path::new("/tmp/condor-audio/wav/zoom_20260325T010000.wav"),
                None,
                1024,
                "2026-03-25T01:00:00Z",
                Some("transcription failed: timeout"),
            )
            .await;

            let tap = get_tap(&registry, "zoom-test").await.expect("tap");
            assert_eq!(tap.status, TapStatus::Running);
            assert_eq!(
                tap.status_detail.as_deref(),
                Some("transcription failed: timeout")
            );
        });
    }
}
