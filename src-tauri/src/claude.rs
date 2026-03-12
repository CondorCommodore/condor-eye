use base64::Engine;
use reqwest::Client;
use crate::compare::ExtractionResult;

/// Error types for Claude API extraction.
#[derive(Debug)]
pub enum ExtractionError {
    Api(String),
    Parse(String),
    RateLimit,
    Network(reqwest::Error),
}

impl From<reqwest::Error> for ExtractionError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            ExtractionError::Api("Request timed out".into())
        } else {
            ExtractionError::Network(e)
        }
    }
}

impl std::fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Api(s) => write!(f, "API error: {}", s),
            Self::Parse(s) => write!(f, "Parse error: {}", s),
            Self::RateLimit => write!(f, "Rate limited — retry in a moment"),
            Self::Network(e) => write!(f, "Network error: {}", e),
        }
    }
}

/// Send a screenshot to Claude for structured data extraction.
///
/// Uses the extraction prompt from the active profile.
/// The prompt should instruct Claude to return a JSON object.
pub async fn extract_from_screenshot(
    api_key: &str,
    png_bytes: &[u8],
    model: &str,
    prompt: &str,
) -> Result<ExtractionResult, ExtractionError> {
    let client = Client::new();
    let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes);

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 2000,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": b64,
                    }
                },
                {
                    "type": "text",
                    "text": prompt,
                }
            ]
        }]
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ExtractionError::RateLimit);
        }
        return Err(ExtractionError::Api(format!("{}: {}", status, text)));
    }

    let api_resp: serde_json::Value = resp.json().await?;
    let content_text = api_resp["content"][0]["text"]
        .as_str()
        .ok_or(ExtractionError::Parse("No text in response".into()))?;

    eprintln!("[VV] Claude raw response ({} chars):\n{}", content_text.len(), content_text);

    // Strip markdown code fences if present
    let json_str = content_text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    let result: ExtractionResult = serde_json::from_str(json_str)
        .map_err(|e| ExtractionError::Parse(
            format!("JSON parse error: {} — raw: {}", e, &json_str[..json_str.len().min(200)])
        ))?;

    Ok(result)
}

/// Parse a raw JSON string into ExtractionResult.
/// Exported for testing JSON parsing independently of the API call.
pub fn parse_extraction(json_str: &str) -> Result<ExtractionResult, String> {
    let cleaned = json_str
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    serde_json::from_str(cleaned)
        .map_err(|e| format!("Parse error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_extraction() {
        let json = r#"{
            "symbol": "SPY",
            "displayType": "dom_ladder",
            "bids": [{"price": 100.50, "volume": 500}],
            "asks": [{"price": 100.51, "volume": 400}],
            "bestBid": 100.50,
            "bestAsk": 100.51,
            "spread": 0.01,
            "levelCount": {"bids": 1, "asks": 1},
            "confidence": "high",
            "notes": null
        }"#;
        let result = parse_extraction(json).unwrap();
        assert_eq!(result.symbol, Some("SPY".to_string()));
        assert_eq!(result.bids.len(), 1);
        assert_eq!(result.bids[0].price, 100.50);
    }

    #[test]
    fn parse_extraction_with_code_fences() {
        let json = "```json\n{\"symbol\":\"SPY\",\"displayType\":\"dom_ladder\",\"bids\":[],\"asks\":[],\"bestBid\":0,\"bestAsk\":0,\"spread\":0,\"levelCount\":{\"bids\":0,\"asks\":0},\"confidence\":\"low\",\"notes\":null}\n```";
        let result = parse_extraction(json).unwrap();
        assert_eq!(result.symbol, Some("SPY".to_string()));
    }

    #[test]
    fn parse_extraction_invalid_json() {
        let result = parse_extraction("not json");
        assert!(result.is_err());
    }
}
