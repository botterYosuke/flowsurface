//! `POST /api/agent/session/:id/advance` のリクエスト / レスポンス型。
//!
//! ADR-0001 / phase4b_agent_replay_api.md §4.1, §4.3 に基づく。
//! 任意区間を wall-time 非依存で instant 実行し、停止条件 (`stop_on`) に
//! 応じて途中停止する。GUI ランタイムでは 400 で拒否する（iced 再描画競合回避）。

use crate::api::contract::EpochMs;
use crate::api::step_response::StepFill;
use crate::replay::virtual_exchange::portfolio::PortfolioSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentAdvanceRequest {
    pub until_ms: EpochMs,
    /// 停止条件。空または省略時は `until_ms` 到達まで走る。
    /// `"end"` は受理しない（plan §4.3: 範囲終端は常に停止するため明示不要）。
    #[serde(default)]
    pub stop_on: Vec<AdvanceStopCondition>,
    /// `true` なら fills 配列をレスポンスに同梱する。デフォルト false（件数のみ）。
    #[serde(default)]
    pub include_fills: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceStopCondition {
    Fill,
    Narrative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvanceStoppedReason {
    UntilReached,
    Fill,
    Narrative,
    End,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvanceResponse {
    pub clock_ms: EpochMs,
    pub stopped_reason: AdvanceStoppedReason,
    pub ticks_advanced: u64,
    pub aggregate_fills: usize,
    pub aggregate_updated_narratives: usize,
    /// `include_fills: true` のときのみ配列で同梱。false のときは None（serde で省略）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<Vec<StepFill>>,
    pub final_portfolio: PortfolioSnapshot,
}

pub fn parse_agent_advance_request(body: &str) -> Result<AgentAdvanceRequest, String> {
    serde_json::from_str::<AgentAdvanceRequest>(body)
        .map_err(|e| format!("invalid advance request: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_body() {
        let body = r#"{"until_ms": 1704067200000}"#;
        let req = parse_agent_advance_request(body).unwrap();
        assert_eq!(req.until_ms, EpochMs::new(1_704_067_200_000));
        assert!(req.stop_on.is_empty());
        assert!(!req.include_fills);
    }

    #[test]
    fn parses_stop_on_fill() {
        let body = r#"{"until_ms": 100, "stop_on": ["fill"]}"#;
        let req = parse_agent_advance_request(body).unwrap();
        assert_eq!(req.stop_on, vec![AdvanceStopCondition::Fill]);
    }

    #[test]
    fn parses_stop_on_multiple_conditions() {
        let body = r#"{"until_ms": 100, "stop_on": ["fill", "narrative"]}"#;
        let req = parse_agent_advance_request(body).unwrap();
        assert_eq!(
            req.stop_on,
            vec![AdvanceStopCondition::Fill, AdvanceStopCondition::Narrative]
        );
    }

    #[test]
    fn parses_include_fills_true() {
        let body = r#"{"until_ms": 100, "include_fills": true}"#;
        let req = parse_agent_advance_request(body).unwrap();
        assert!(req.include_fills);
    }

    #[test]
    fn rejects_end_in_stop_on() {
        // plan §4.3: `"end"` は受理しない（範囲終端は常に停止）
        let body = r#"{"until_ms": 100, "stop_on": ["end"]}"#;
        let err = parse_agent_advance_request(body).unwrap_err();
        assert!(err.contains("stop_on") || err.contains("unknown variant"));
    }

    #[test]
    fn rejects_missing_until_ms() {
        let body = r#"{}"#;
        let err = parse_agent_advance_request(body).unwrap_err();
        assert!(err.contains("until_ms"));
    }

    #[test]
    fn rejects_negative_until_ms_via_u64() {
        // until_ms は EpochMs(u64) なので負値は deserialize 失敗。
        let body = r#"{"until_ms": -1}"#;
        let err = parse_agent_advance_request(body).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let body = r#"{"until_ms": 100, "unknown": "x"}"#;
        let err = parse_agent_advance_request(body).unwrap_err();
        assert!(err.contains("unknown") || err.contains("field"));
    }
}
