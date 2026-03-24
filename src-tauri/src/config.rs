use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub api_key: String,
    pub redis_url: String,
    pub model: String,
    pub discord_bridge_url: Option<String>,
    pub coord_api_url: String,
    pub coord_api_token: String,
    pub condor_eye_bind: String,
    pub condor_eye_port: u16,
    pub audio_bind: String,
    pub audio_port: u16,
    pub audio_output_dir: String,
    pub audio_transport: String,
    pub whisper_url: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY")
                .unwrap_or_default(),
            redis_url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
            model: std::env::var("CLAUDE_MODEL")
                .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string()),
            discord_bridge_url: std::env::var("DISCORD_BRIDGE_URL").ok(),
            coord_api_url: std::env::var("COORD_API_URL")
                .unwrap_or_else(|_| "http://localhost:8800".to_string()),
            coord_api_token: std::env::var("COORD_API_TOKEN")
                .unwrap_or_default(),
            condor_eye_bind: std::env::var("CONDOR_EYE_BIND")
                .unwrap_or_else(|_| "0.0.0.0".to_string()),
            condor_eye_port: std::env::var("CONDOR_EYE_PORT")
                .unwrap_or_else(|_| "9050".to_string())
                .parse::<u16>()
                .unwrap_or(9050),
            audio_bind: std::env::var("CONDOR_AUDIO_BIND")
                .or_else(|_| std::env::var("CONDOR_EYE_AUDIO_BIND"))
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            audio_port: std::env::var("CONDOR_AUDIO_PORT")
                .or_else(|_| std::env::var("CONDOR_EYE_AUDIO_PORT"))
                .unwrap_or_else(|_| "9051".to_string())
                .parse::<u16>()
                .unwrap_or(9051),
            audio_output_dir: std::env::var("CONDOR_AUDIO_OUTPUT_DIR")
                .or_else(|_| std::env::var("CONDOR_EYE_AUDIO_OUTPUT_DIR"))
                .unwrap_or_else(|_| default_audio_output_dir()),
            audio_transport: std::env::var("AUDIO_TRANSPORT")
                .unwrap_or_else(|_| "http".to_string()),
            whisper_url: std::env::var("WHISPER_URL")
                .unwrap_or_else(|_| "http://localhost:8080/inference".to_string()),
        }
    }
}

fn default_audio_output_dir() -> String {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        return Path::new(&local_app_data)
            .join("condor_audio")
            .join("audio-taps")
            .to_string_lossy()
            .into_owned();
    }

    std::env::temp_dir()
        .join("condor_audio")
        .join("audio-taps")
        .to_string_lossy()
        .into_owned()
}

/// Per-instrument tick sizes — determines price matching tolerance.
const TICK_SIZES: &[(&str, f64)] = &[
    ("SPY", 0.01), ("AAPL", 0.01), ("QQQ", 0.01), ("IWM", 0.01),
    ("ES", 0.25), ("NQ", 0.25), ("MES", 0.25), ("MNQ", 0.25),
    ("YM", 1.0), ("RTY", 0.10), ("CL", 0.01), ("GC", 0.10),
];

/// Returns tick size for a symbol. Strips leading '/' for futures.
pub fn tick_size(symbol: &str) -> f64 {
    let bare = symbol.trim_start_matches('/').to_uppercase();
    TICK_SIZES.iter()
        .find(|(s, _)| *s == bare)
        .map(|(_, t)| *t)
        .unwrap_or(0.01)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionProfile {
    pub name: String,
    pub prompt: String,
    #[serde(rename = "truthSource")]
    pub truth_source: TruthSourceConfig,
    pub comparison: ComparisonConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthSourceConfig {
    #[serde(rename = "type")]
    pub source_type: String,       // "redis_stream", "file", "none"
    pub stream: Option<String>,    // e.g. "market.depth"
    #[serde(rename = "matchField")]
    pub match_field: Option<String>, // e.g. "symbol"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonConfig {
    #[serde(rename = "priceToleranceMode")]
    pub price_tolerance_mode: String,  // "tick_size" or "fixed"
    #[serde(rename = "volumeField")]
    pub volume_field: VolumeFieldMapping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeFieldMapping {
    pub extracted: String,
    pub truth: String,
}

/// Load a profile from a JSON file.
pub fn load_profile(path: &Path) -> Result<ExtractionProfile, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read profile {}: {}", path.display(), e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Invalid profile JSON {}: {}", path.display(), e))
}

/// Load all profiles from a directory.
pub fn load_all_profiles(dir: &Path) -> Vec<ExtractionProfile> {
    let mut profiles = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "json") {
                if let Ok(profile) = load_profile(&path) {
                    profiles.push(profile);
                }
            }
        }
    }
    profiles
}

/// Estimate Claude API cost for a screenshot of given dimensions.
/// Formula: image_tokens = (width * height) / 750, output ~ 500 tokens.
pub fn estimate_cost(width: u32, height: u32, model: &str) -> f64 {
    let image_tokens = (width as f64 * height as f64) / 750.0;
    let output_tokens = 500.0;
    let (input_rate, output_rate) = if model.contains("haiku") {
        (1.0, 5.0)  // per million tokens
    } else if model.contains("sonnet") {
        (3.0, 15.0)
    } else {
        (1.0, 5.0)  // default to haiku pricing
    };
    (image_tokens / 1_000_000.0) * input_rate + (output_tokens / 1_000_000.0) * output_rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_size_equity() {
        assert_eq!(tick_size("SPY"), 0.01);
        assert_eq!(tick_size("AAPL"), 0.01);
    }

    #[test]
    fn tick_size_futures_with_slash() {
        assert_eq!(tick_size("/ES"), 0.25);
        assert_eq!(tick_size("/NQ"), 0.25);
        assert_eq!(tick_size("/YM"), 1.0);
    }

    #[test]
    fn tick_size_futures_without_slash() {
        assert_eq!(tick_size("ES"), 0.25);
    }

    #[test]
    fn tick_size_unknown_defaults_penny() {
        assert_eq!(tick_size("ZZZZ"), 0.01);
    }

    #[test]
    fn tick_size_case_insensitive() {
        assert_eq!(tick_size("spy"), 0.01);
        assert_eq!(tick_size("es"), 0.25);
    }

    #[test]
    fn load_depth_profile() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("profiles/depth.json");
        let profile = load_profile(&path).expect("should load depth.json");
        assert_eq!(profile.name, "depth");
        assert_eq!(profile.truth_source.source_type, "redis_stream");
        assert_eq!(profile.truth_source.stream.as_deref(), Some("market.depth"));
        assert_eq!(profile.comparison.volume_field.truth, "totalVolume");
    }

    #[test]
    fn load_all_profiles_finds_five() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("profiles");
        let profiles = load_all_profiles(&dir);
        assert!(profiles.len() >= 5, "expected at least 5 profiles, got {}", profiles.len());
    }

    #[test]
    fn estimate_cost_haiku() {
        // ~3000 input tokens (typical screenshot), ~500 output tokens
        // Haiku: $1/M input, $5/M output
        let cost = estimate_cost(400, 700, "claude-haiku-4-5-20251001");
        assert!(cost > 0.001 && cost < 0.01, "haiku cost should be ~$0.005, got {}", cost);
    }

    #[test]
    fn estimate_cost_sonnet() {
        let cost = estimate_cost(400, 700, "claude-sonnet-4-6");
        assert!(cost > 0.005 && cost < 0.05, "sonnet cost should be more than haiku, got {}", cost);
    }
}
