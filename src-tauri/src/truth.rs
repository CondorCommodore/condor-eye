use redis::{Client, Commands};
use crate::compare::{DepthSnapshot, TruthLevel};

#[derive(Debug)]
pub enum TruthError {
    Connection(String),
    NotFound(String),
    Parse(String),
}

impl std::fmt::Display for TruthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connection(s) => write!(f, "Redis connection error: {}", s),
            Self::NotFound(s) => write!(f, "No truth data found: {}", s),
            Self::Parse(s) => write!(f, "Truth data parse error: {}", s),
        }
    }
}

/// Snapshot the latest depth data for `symbol` from a Redis stream.
///
/// Uses XREVRANGE to read the most recent entries and finds the first
/// matching the requested symbol.
pub fn snapshot_depth(
    redis_url: &str,
    stream: &str,
    symbol: &str,
) -> Result<DepthSnapshot, TruthError> {
    let client = Client::open(redis_url)
        .map_err(|e| TruthError::Connection(e.to_string()))?;
    let mut conn = client.get_connection()
        .map_err(|e| TruthError::Connection(e.to_string()))?;

    // XREVRANGE <stream> + - COUNT 50
    let entries: Vec<redis::streams::StreamId> = redis::cmd("XREVRANGE")
        .arg(stream)
        .arg("+")
        .arg("-")
        .arg("COUNT")
        .arg(50)
        .query::<redis::streams::StreamRangeReply>(&mut conn)
        .map_err(|e| TruthError::Connection(e.to_string()))?
        .ids;

    for entry in entries {
        // Extract the "data" field from the stream entry
        let data_str: Option<String> = entry.map.get("data").and_then(|v| {
            match v {
                redis::Value::BulkString(d) => String::from_utf8(d.clone()).ok(),
                _ => None,
            }
        });

        if let Some(json_str) = data_str {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&json_str) {
                if msg["symbol"].as_str() == Some(symbol) {
                    let bids: Vec<TruthLevel> = serde_json::from_value(
                        msg["bids"].clone(),
                    ).unwrap_or_default();
                    let asks: Vec<TruthLevel> = serde_json::from_value(
                        msg["asks"].clone(),
                    ).unwrap_or_default();

                    let stream_id = entry.id.clone();
                    let timestamp = stream_id
                        .split('-')
                        .next()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);

                    return Ok(DepthSnapshot {
                        stream_id,
                        timestamp,
                        symbol: symbol.to_string(),
                        source: msg["source"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_string(),
                        bids,
                        asks,
                    });
                }
            }
        }
    }

    Err(TruthError::NotFound(format!(
        "No depth data for {} in stream {}",
        symbol, stream
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truth_error_display() {
        let e = TruthError::NotFound("SPY".to_string());
        assert!(format!("{}", e).contains("SPY"));
    }

    // Integration test — requires Redis running with market.depth data.
    // Run manually:
    //   REDIS_URL=redis://127.0.0.1:6379 cargo test truth::tests::snapshot_real -- --ignored
    #[test]
    #[ignore]
    fn snapshot_real() {
        let url = std::env::var("REDIS_URL").unwrap_or("redis://127.0.0.1:6379".to_string());
        let result = snapshot_depth(&url, "market.depth", "SPY");
        match result {
            Ok(snap) => {
                println!("Got snapshot: {} bids, {} asks", snap.bids.len(), snap.asks.len());
                assert!(!snap.bids.is_empty());
                assert!(!snap.asks.is_empty());
            }
            Err(e) => println!("Redis not available: {}", e),
        }
    }
}
