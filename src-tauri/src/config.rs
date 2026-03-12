use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub api_key: String,
    pub redis_url: String,
    pub model: String,
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
        }
    }
}
