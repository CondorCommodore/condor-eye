use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewProvider {
    Claude,
    Codex,
    None,
}

impl Default for ReviewProvider {
    fn default() -> Self {
        Self::None
    }
}

/// Review a screenshot using a local CLI tool.
///
/// Saves the PNG to a temp file, spawns the CLI, returns stdout.
/// Returns empty string for ReviewProvider::None.
pub async fn review_screenshot(
    provider: ReviewProvider,
    png_bytes: &[u8],
    prompt: &str,
) -> Result<String, String> {
    match provider {
        ReviewProvider::None => Ok(String::new()),
        ReviewProvider::Claude => run_cli_review("claude", png_bytes, prompt).await,
        ReviewProvider::Codex => run_cli_review("codex", png_bytes, prompt).await,
    }
}

async fn run_cli_review(
    tool: &str,
    png_bytes: &[u8],
    prompt: &str,
) -> Result<String, String> {
    // Save screenshot to temp file
    let tmp_dir = std::env::temp_dir().join("condor-eye-review");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let img_path = tmp_dir.join(format!("capture-{}.png", ts));
    std::fs::write(&img_path, png_bytes)
        .map_err(|e| format!("Failed to write temp image: {}", e))?;

    let img_path_str = img_path.to_string_lossy().to_string();
    let prompt_owned = prompt.to_string();
    let tool_owned = tool.to_string();

    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(&tool_owned);
        match tool_owned.as_str() {
            "claude" => {
                cmd.arg("-p")
                    .arg(&prompt_owned)
                    .arg("--allowedTools")
                    .arg("none")
                    .env("CLAUDE_IMAGE", &img_path_str);
            }
            "codex" => {
                cmd.arg("-q")
                    .arg(format!("{}\n\nImage: {}", prompt_owned, img_path_str));
            }
            _ => {
                cmd.arg(&prompt_owned);
            }
        }
        cmd.output()
    })
    .await
    .map_err(|e| format!("Task join: {}", e))?
    .map_err(|e| format!("{} not found or failed to run: {}", tool, e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&img_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{} failed: {}", tool, stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_provider_default_is_none() {
        assert_eq!(ReviewProvider::default(), ReviewProvider::None);
    }

    #[test]
    fn review_provider_deserialize() {
        let p: ReviewProvider = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(p, ReviewProvider::Claude);
        let p: ReviewProvider = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(p, ReviewProvider::Codex);
        let p: ReviewProvider = serde_json::from_str("\"none\"").unwrap();
        assert_eq!(p, ReviewProvider::None);
    }

    #[tokio::test]
    async fn review_none_returns_empty() {
        let result = review_screenshot(ReviewProvider::None, b"fake png", "describe").await;
        assert_eq!(result.unwrap(), "");
    }
}
