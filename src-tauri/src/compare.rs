use serde::{Deserialize, Serialize};
use crate::config::tick_size;

// ── Types from Claude extraction ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedLevel {
    #[serde(default)]
    pub price: f64,
    pub volume: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelCount {
    #[serde(default)]
    pub bids: usize,
    #[serde(default)]
    pub asks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub symbol: Option<String>,
    #[serde(rename = "displayType", default)]
    pub display_type: String,
    #[serde(default)]
    pub bids: Vec<ExtractedLevel>,
    #[serde(default)]
    pub asks: Vec<ExtractedLevel>,
    #[serde(rename = "bestBid", default)]
    pub best_bid: f64,
    #[serde(rename = "bestAsk", default)]
    pub best_ask: f64,
    #[serde(default)]
    pub spread: f64,
    #[serde(rename = "levelCount")]
    pub level_count: Option<LevelCount>,
    #[serde(default)]
    pub confidence: String,
    pub notes: Option<String>,
}

// ── Types from ground truth ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthLevel {
    pub price: f64,
    #[serde(alias = "totalVolume")]
    pub total_volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthSnapshot {
    pub stream_id: String,
    pub timestamp: u64,
    pub symbol: String,
    pub source: String,
    pub bids: Vec<TruthLevel>,
    pub asks: Vec<TruthLevel>,
}

// ── Report types ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Status {
    PASS,
    WARN,
    FAIL,
    #[serde(rename = "EXTRACT_ONLY")]
    ExtractOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mismatch {
    pub side: String,
    pub price: f64,
    pub extracted_volume: u64,
    pub truth_volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingLevel {
    pub side: String,
    pub price: f64,
    pub volume: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraLevel {
    pub side: String,
    pub price: f64,
    pub volume: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonReport {
    pub timestamp: u64,
    pub symbol: String,
    pub overall: Status,
    pub extracted_bids: usize,
    pub extracted_asks: usize,
    pub truth_bids: usize,
    pub truth_asks: usize,
    pub best_bid_match: bool,
    pub best_ask_match: bool,
    pub mismatches: Vec<Mismatch>,
    pub missing: Vec<MissingLevel>,
    pub extra: Vec<ExtraLevel>,
    pub api_latency_ms: u64,
    pub estimated_cost_usd: f64,
    pub extraction: Option<ExtractionResult>,
}

/// Compare extracted screen data against ground truth.
pub fn compare_books(
    extracted: &ExtractionResult,
    truth: &DepthSnapshot,
) -> ComparisonReport {
    let tolerance = tick_size(&truth.symbol) * 0.5;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut extra = Vec::new();

    // Best bid/ask comparison
    let best_bid_match = truth.bids.first()
        .map_or(true, |t| (extracted.best_bid - t.price).abs() < tolerance);
    let best_ask_match = truth.asks.first()
        .map_or(true, |t| (extracted.best_ask - t.price).abs() < tolerance);

    // Compare each side
    for (side_name, truth_levels, extracted_levels) in [
        ("bids", &truth.bids, &extracted.bids),
        ("asks", &truth.asks, &extracted.asks),
    ] {
        // Forward pass: find truth levels missing from extraction
        for t in truth_levels {
            let found = extracted_levels.iter().find(|e| (e.price - t.price).abs() < tolerance);
            match found {
                None => {
                    missing.push(MissingLevel {
                        side: side_name.to_string(),
                        price: t.price,
                        volume: t.total_volume,
                    });
                }
                Some(e) => {
                    if let Some(ev) = e.volume {
                        if ev != t.total_volume {
                            mismatches.push(Mismatch {
                                side: side_name.to_string(),
                                price: t.price,
                                extracted_volume: ev,
                                truth_volume: t.total_volume,
                            });
                        }
                    }
                }
            }
        }

        // Reverse pass: find extracted levels not in truth
        for e in extracted_levels.iter() {
            let in_truth = truth_levels.iter().any(|t| (e.price - t.price).abs() < tolerance);
            if !in_truth {
                extra.push(ExtraLevel {
                    side: side_name.to_string(),
                    price: e.price,
                    volume: e.volume,
                });
            }
        }
    }

    // Determine overall status
    let overall = if !mismatches.is_empty() || missing.len() > 3 {
        Status::FAIL
    } else if !missing.is_empty() || !best_bid_match || !best_ask_match {
        Status::WARN
    } else {
        Status::PASS
    };

    ComparisonReport {
        timestamp: now,
        symbol: truth.symbol.clone(),
        overall,
        extracted_bids: extracted.bids.len(),
        extracted_asks: extracted.asks.len(),
        truth_bids: truth.bids.len(),
        truth_asks: truth.asks.len(),
        best_bid_match,
        best_ask_match,
        mismatches,
        missing,
        extra,
        api_latency_ms: 0,
        estimated_cost_usd: 0.0,
        extraction: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_truth(bids: Vec<(f64, u64)>, asks: Vec<(f64, u64)>) -> DepthSnapshot {
        DepthSnapshot {
            stream_id: "1234-0".to_string(),
            timestamp: 1000,
            symbol: "SPY".to_string(),
            source: "ibkr".to_string(),
            bids: bids.into_iter().map(|(p, v)| TruthLevel { price: p, total_volume: v }).collect(),
            asks: asks.into_iter().map(|(p, v)| TruthLevel { price: p, total_volume: v }).collect(),
        }
    }

    fn make_extraction(bids: Vec<(f64, Option<u64>)>, asks: Vec<(f64, Option<u64>)>) -> ExtractionResult {
        let best_bid = bids.first().map_or(0.0, |b| b.0);
        let best_ask = asks.first().map_or(0.0, |a| a.0);
        ExtractionResult {
            symbol: Some("SPY".to_string()),
            display_type: "dom_ladder".to_string(),
            bids: bids.into_iter().map(|(p, v)| ExtractedLevel { price: p, volume: v }).collect(),
            asks: asks.into_iter().map(|(p, v)| ExtractedLevel { price: p, volume: v }).collect(),
            best_bid,
            best_ask,
            spread: best_ask - best_bid,
            level_count: Some(LevelCount { bids: 0, asks: 0 }),
            confidence: "high".to_string(),
            notes: None,
        }
    }

    #[test]
    fn perfect_match_passes() {
        let truth = make_truth(
            vec![(100.50, 500), (100.49, 300)],
            vec![(100.51, 400), (100.52, 200)],
        );
        let ext = make_extraction(
            vec![(100.50, Some(500)), (100.49, Some(300))],
            vec![(100.51, Some(400)), (100.52, Some(200))],
        );
        let report = compare_books(&ext, &truth);
        assert_eq!(report.overall, Status::PASS);
        assert!(report.mismatches.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.extra.is_empty());
    }

    #[test]
    fn volume_mismatch_detected() {
        let truth = make_truth(
            vec![(100.50, 500)],
            vec![(100.51, 400)],
        );
        let ext = make_extraction(
            vec![(100.50, Some(999))],
            vec![(100.51, Some(400))],
        );
        let report = compare_books(&ext, &truth);
        assert_eq!(report.mismatches.len(), 1);
        assert_eq!(report.mismatches[0].extracted_volume, 999);
        assert_eq!(report.mismatches[0].truth_volume, 500);
    }

    #[test]
    fn missing_level_detected() {
        let truth = make_truth(
            vec![(100.50, 500), (100.49, 300)],
            vec![(100.51, 400)],
        );
        let ext = make_extraction(
            vec![(100.50, Some(500))],
            vec![(100.51, Some(400))],
        );
        let report = compare_books(&ext, &truth);
        assert_eq!(report.missing.len(), 1);
        assert_eq!(report.missing[0].price, 100.49);
    }

    #[test]
    fn extra_level_detected() {
        let truth = make_truth(
            vec![(100.50, 500)],
            vec![(100.51, 400)],
        );
        let ext = make_extraction(
            vec![(100.50, Some(500)), (100.48, Some(100))],
            vec![(100.51, Some(400))],
        );
        let report = compare_books(&ext, &truth);
        assert_eq!(report.extra.len(), 1);
        assert_eq!(report.extra[0].price, 100.48);
    }

    #[test]
    fn obscured_volume_not_counted_as_mismatch() {
        let truth = make_truth(
            vec![(100.50, 500)],
            vec![(100.51, 400)],
        );
        let ext = make_extraction(
            vec![(100.50, None)],
            vec![(100.51, Some(400))],
        );
        let report = compare_books(&ext, &truth);
        assert!(report.mismatches.is_empty(), "obscured volume should not be a mismatch");
    }

    #[test]
    fn many_missing_causes_fail() {
        let truth = make_truth(
            vec![(100.50, 500), (100.49, 300), (100.48, 200), (100.47, 100)],
            vec![(100.51, 400)],
        );
        let ext = make_extraction(
            vec![],
            vec![(100.51, Some(400))],
        );
        let report = compare_books(&ext, &truth);
        assert_eq!(report.overall, Status::FAIL);
    }

    #[test]
    fn futures_tick_size_tolerance() {
        let truth = DepthSnapshot {
            stream_id: "1-0".to_string(),
            timestamp: 1000,
            symbol: "/ES".to_string(),
            source: "ibkr".to_string(),
            bids: vec![TruthLevel { price: 5600.25, total_volume: 100 }],
            asks: vec![TruthLevel { price: 5600.50, total_volume: 50 }],
        };
        let ext = ExtractionResult {
            symbol: Some("/ES".to_string()),
            display_type: "dom_ladder".to_string(),
            bids: vec![ExtractedLevel { price: 5600.25, volume: Some(100) }],
            asks: vec![ExtractedLevel { price: 5600.50, volume: Some(50) }],
            best_bid: 5600.25,
            best_ask: 5600.50,
            spread: 0.25,
            level_count: Some(LevelCount { bids: 1, asks: 1 }),
            confidence: "high".to_string(),
            notes: None,
        };
        let report = compare_books(&ext, &truth);
        assert_eq!(report.overall, Status::PASS);
    }
}
