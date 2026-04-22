//! `POST /api/agent/session/:id/step` のレスポンス組み立て。
//!
//! ADR-0001 / phase4b_agent_replay_api.md §4.2 に基づく。agent が polling せずに
//! 当該 tick の副作用（fills / narrative 更新）と観測を 1 RTT で取得できるように
//! する。本モジュールはレスポンスの JSON 構築に責務を限定し、tick 前進や
//! narrative 更新等の副作用は呼び出し側（headless / GUI）が担う。

use crate::api::contract::EpochMs;
use crate::replay::virtual_exchange::order_book::FillEvent;
use crate::replay::virtual_exchange::portfolio::{PortfolioSnapshot, PositionSide};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct StepResponse {
    pub clock_ms: EpochMs,
    pub reached_end: bool,
    pub observation: StepObservation,
    pub fills: Vec<StepFill>,
    /// 当該 tick で outcome が更新された narrative の UUID。
    /// サブフェーズ C 時点では空配列（サブフェーズ D で本実装）。
    pub updated_narrative_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepObservation {
    pub ohlcv: Vec<serde_json::Value>,
    pub recent_trades: Vec<serde_json::Value>,
    pub portfolio: PortfolioSnapshot,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepFill {
    pub order_id: String,
    /// `VirtualOrder.client_order_id` と紐付く。
    /// サブフェーズ C 時点では常に `None`（サブフェーズ E で本実装）。
    pub client_order_id: Option<String>,
    pub fill_price: f64,
    pub qty: f64,
    /// agent API JSON 上は `"buy"` / `"sell"`（request 側と対称）。
    /// 内部の `PositionSide { Long, Short }` とは意図的に分離する。
    pub side: &'static str,
    pub fill_time_ms: EpochMs,
}

impl StepFill {
    pub fn from_event(fill: &FillEvent, client_order_id: Option<String>) -> Self {
        Self {
            order_id: fill.order_id.clone(),
            client_order_id,
            fill_price: fill.fill_price,
            qty: fill.qty,
            side: match fill.side {
                PositionSide::Long => "buy",
                PositionSide::Short => "sell",
            },
            fill_time_ms: EpochMs::from(fill.fill_time_ms),
        }
    }
}

impl StepResponse {
    pub fn new(
        clock_ms: u64,
        reached_end: bool,
        observation: StepObservation,
        fills: Vec<StepFill>,
    ) -> Self {
        Self {
            clock_ms: EpochMs::from(clock_ms),
            reached_end,
            observation,
            fills,
            updated_narrative_ids: Vec::new(),
        }
    }

    /// JSON 文字列化。serialize 失敗時はエラー本文を返す（呼び出し側で 500 を返せる）。
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::virtual_exchange::portfolio::PortfolioSnapshot;

    fn empty_portfolio() -> PortfolioSnapshot {
        PortfolioSnapshot {
            cash: 1_000_000.0,
            unrealized_pnl: 0.0,
            realized_pnl: 0.0,
            total_equity: 1_000_000.0,
            open_positions: Vec::new(),
            closed_positions: Vec::new(),
        }
    }

    fn sample_fill(order_id: &str, fill_time_ms: u64) -> FillEvent {
        FillEvent {
            order_id: order_id.to_string(),
            ticker: "BTCUSDT".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            fill_price: 92100.5,
            fill_time_ms,
        }
    }

    #[test]
    fn response_includes_clock_ms_as_transparent_integer() {
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let resp = StepResponse::new(1_704_067_260_000, false, obs, vec![]);
        let v: serde_json::Value = serde_json::from_str(&resp.to_json_string().unwrap()).unwrap();
        assert_eq!(v["clock_ms"], 1_704_067_260_000_u64);
    }

    #[test]
    fn response_has_reached_end_flag() {
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let resp_at_end = StepResponse::new(0, true, obs.clone(), vec![]);
        let resp_running = StepResponse::new(0, false, obs, vec![]);
        let v_end: serde_json::Value =
            serde_json::from_str(&resp_at_end.to_json_string().unwrap()).unwrap();
        let v_run: serde_json::Value =
            serde_json::from_str(&resp_running.to_json_string().unwrap()).unwrap();
        assert_eq!(v_end["reached_end"], true);
        assert_eq!(v_run["reached_end"], false);
    }

    #[test]
    fn step_returns_fills_inline() {
        // ADR-0001 の核不変条件: agent は fills を polling せず、
        // step レスポンスに同梱された配列で受け取る。
        let fill1 = sample_fill("ord_1", 1_704_067_260_000);
        let fill2 = sample_fill("ord_2", 1_704_067_260_000);
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let fills = vec![
            StepFill::from_event(&fill1, None),
            StepFill::from_event(&fill2, None),
        ];
        let resp = StepResponse::new(1_704_067_260_000, false, obs, fills);
        let v: serde_json::Value = serde_json::from_str(&resp.to_json_string().unwrap()).unwrap();
        let fills_arr = v["fills"].as_array().expect("fills must be an array");
        assert_eq!(fills_arr.len(), 2);
        assert_eq!(fills_arr[0]["order_id"], "ord_1");
        assert_eq!(fills_arr[1]["order_id"], "ord_2");
    }

    #[test]
    fn fill_maps_long_to_buy_and_short_to_sell() {
        let mut fill_long = sample_fill("ord_1", 1);
        fill_long.side = PositionSide::Long;
        let mut fill_short = sample_fill("ord_2", 1);
        fill_short.side = PositionSide::Short;
        let sf_long = StepFill::from_event(&fill_long, None);
        let sf_short = StepFill::from_event(&fill_short, None);
        assert_eq!(sf_long.side, "buy");
        assert_eq!(sf_short.side, "sell");
    }

    #[test]
    fn fill_client_order_id_is_null_when_none() {
        // サブフェーズ C 時点では client_order_id は常に None。
        // サブフェーズ E で VirtualOrder に client_order_id を追加して埋める。
        let fill = sample_fill("ord_1", 1);
        let sf = StepFill::from_event(&fill, None);
        let json = serde_json::to_string(&sf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["client_order_id"].is_null(), "got {json}");
    }

    #[test]
    fn fill_client_order_id_populated_when_some() {
        let fill = sample_fill("ord_1", 1);
        let sf = StepFill::from_event(&fill, Some("cli_42".to_string()));
        let json = serde_json::to_string(&sf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["client_order_id"], "cli_42");
    }

    #[test]
    fn fill_time_ms_serializes_as_integer_via_epoch_ms() {
        let fill = sample_fill("ord_1", 1_704_067_260_000);
        let sf = StepFill::from_event(&fill, None);
        let json = serde_json::to_string(&sf).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        // EpochMs の #[serde(transparent)] により u64 整数として serialize される。
        assert_eq!(v["fill_time_ms"], 1_704_067_260_000_u64);
    }

    #[test]
    fn response_has_updated_narrative_ids_as_array_empty_by_default() {
        // サブフェーズ C 時点では常に空配列。サブフェーズ D で埋める。
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let resp = StepResponse::new(0, false, obs, vec![]);
        let v: serde_json::Value = serde_json::from_str(&resp.to_json_string().unwrap()).unwrap();
        let arr = v["updated_narrative_ids"]
            .as_array()
            .expect("must be array");
        assert!(arr.is_empty());
    }

    #[test]
    fn observation_contains_required_top_level_keys() {
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let resp = StepResponse::new(0, false, obs, vec![]);
        let v: serde_json::Value = serde_json::from_str(&resp.to_json_string().unwrap()).unwrap();
        assert!(v["observation"]["ohlcv"].is_array());
        assert!(v["observation"]["recent_trades"].is_array());
        assert!(v["observation"]["portfolio"].is_object());
        assert_eq!(v["observation"]["portfolio"]["cash"], 1_000_000.0);
    }

    #[test]
    fn top_level_shape_matches_plan_spec() {
        // phase4b_agent_replay_api.md §4.2 の JSON 形に一致すること。
        let obs = StepObservation {
            ohlcv: vec![],
            recent_trades: vec![],
            portfolio: empty_portfolio(),
        };
        let resp = StepResponse::new(42, false, obs, vec![]);
        let v: serde_json::Value = serde_json::from_str(&resp.to_json_string().unwrap()).unwrap();
        for key in ["clock_ms", "reached_end", "observation", "fills", "updated_narrative_ids"] {
            assert!(v.get(key).is_some(), "missing top-level key: {key}");
        }
    }
}
