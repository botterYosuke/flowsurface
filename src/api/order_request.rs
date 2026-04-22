//! `POST /api/agent/session/:id/order` のリクエスト型。
//!
//! ADR-0001 / phase4b_agent_replay_api.md §3.3, §4.4 に基づき、
//! `ticker` は構造体必須、`client_order_id` と `order_type` は明示必須で、
//! 文字列 ticker / 省略 order_type を拒否して silent failure を構造的に防ぐ。

use crate::api::contract::{ClientOrderId, TickerContract};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOrderRequest {
    pub client_order_id: ClientOrderId,
    pub ticker: TickerContract,
    pub side: AgentOrderSide,
    pub qty: f64,
    pub order_type: AgentOrderType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOrderSide {
    Buy,
    Sell,
}

/// `{"market": {}}` / `{"limit": {"price": X}}` の externally tagged enum。
/// 省略時は 400 を返す（Phase 4a の silent market default 再発防止）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum AgentOrderType {
    Market {},
    Limit { price: f64 },
}

/// 冪等性判定用の構造的等価キー。リクエストボディ全体から `client_order_id` を除いて
/// `(ticker, side, qty, order_type)` のみで同等判定する。plan §3.3 の簡略化方針に準拠
/// （derive(PartialEq) で f64 bit equality — agent は同じ入力で同じ f64 を再生成する前提）。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOrderRequestKey {
    pub ticker: TickerContract,
    pub side: AgentOrderSide,
    pub qty: f64,
    pub order_type: AgentOrderType,
}

impl AgentOrderRequest {
    pub fn to_key(&self) -> AgentOrderRequestKey {
        AgentOrderRequestKey {
            ticker: self.ticker.clone(),
            side: self.side,
            qty: self.qty,
            order_type: self.order_type.clone(),
        }
    }
}

/// JSON 文字列を `AgentOrderRequest` に変換する。400 返却用のエラーメッセージを付ける。
pub fn parse_agent_order_request(body: &str) -> Result<AgentOrderRequest, String> {
    serde_json::from_str::<AgentOrderRequest>(body)
        .map_err(|e| format!("invalid order request: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_body() -> &'static str {
        r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#
    }

    #[test]
    fn parses_valid_market_buy_request() {
        let req = parse_agent_order_request(valid_body()).unwrap();
        assert_eq!(req.client_order_id.as_str(), "cli_42");
        assert_eq!(req.ticker.exchange, "HyperliquidLinear");
        assert_eq!(req.ticker.symbol, "BTC");
        assert_eq!(req.side, AgentOrderSide::Buy);
        assert_eq!(req.qty, 0.1);
        assert_eq!(req.order_type, AgentOrderType::Market {});
    }

    #[test]
    fn parses_limit_sell_request() {
        let body = r#"{
            "client_order_id": "cli_1",
            "ticker": {"exchange": "BinanceSpot", "symbol": "BTCUSDT"},
            "side": "sell",
            "qty": 0.5,
            "order_type": {"limit": {"price": 92500.0}}
        }"#;
        let req = parse_agent_order_request(body).unwrap();
        assert_eq!(req.side, AgentOrderSide::Sell);
        assert_eq!(req.order_type, AgentOrderType::Limit { price: 92500.0 });
    }

    #[test]
    fn rejects_missing_client_order_id() {
        let body = r#"{
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(err.contains("client_order_id"), "got {err}");
    }

    #[test]
    fn rejects_missing_order_type_no_silent_default() {
        // Phase 4a silent failure 防止: order_type 省略は 400 で明示拒否。
        let body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.1
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(err.contains("order_type"), "got {err}");
    }

    #[test]
    fn rejects_string_ticker_no_silent_normalization() {
        // Phase 4a silent failure 防止: "Exchange:Symbol" 文字列は拒否。
        let body = r#"{
            "client_order_id": "cli_42",
            "ticker": "HyperliquidLinear:BTC",
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(
            err.contains("ticker") || err.contains("invalid type"),
            "got {err}"
        );
    }

    #[test]
    fn rejects_unknown_side() {
        let body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "Long",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(err.contains("side") || err.contains("unknown variant"));
    }

    #[test]
    fn rejects_invalid_client_order_id_charset() {
        let body = r#"{
            "client_order_id": "cli 42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(err.contains("client_order_id"), "got {err}");
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}},
            "extra": 42
        }"#;
        let err = parse_agent_order_request(body).unwrap_err();
        assert!(err.contains("extra") || err.contains("unknown"), "got {err}");
    }

    #[test]
    fn rejects_limit_without_price() {
        let body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"limit": {}}
        }"#;
        assert!(parse_agent_order_request(body).is_err());
    }

    // ── AgentOrderRequestKey 等価性 ──

    #[test]
    fn request_key_equal_for_same_structural_body() {
        let a = parse_agent_order_request(valid_body()).unwrap();
        // JSON key 順が違っても serde deserialize 後は同じ struct になる。
        let b_body = r#"{
            "qty": 0.1,
            "side": "buy",
            "client_order_id": "cli_42",
            "order_type": {"market": {}},
            "ticker": {"symbol": "BTC", "exchange": "HyperliquidLinear"}
        }"#;
        let b = parse_agent_order_request(b_body).unwrap();
        assert_eq!(a.to_key(), b.to_key());
    }

    #[test]
    fn request_key_differs_when_qty_differs() {
        let a = parse_agent_order_request(valid_body()).unwrap();
        let b_body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.2,
            "order_type": {"market": {}}
        }"#;
        let b = parse_agent_order_request(b_body).unwrap();
        assert_ne!(a.to_key(), b.to_key());
    }

    #[test]
    fn request_key_differs_when_order_type_differs() {
        let a = parse_agent_order_request(valid_body()).unwrap();
        let b_body = r#"{
            "client_order_id": "cli_42",
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"limit": {"price": 92500.0}}
        }"#;
        let b = parse_agent_order_request(b_body).unwrap();
        assert_ne!(a.to_key(), b.to_key());
    }

    #[test]
    fn request_key_ignores_client_order_id() {
        // key は idempotency 比較用。client_order_id 自体は key から除外する
        // （同じ key で異なる client_order_id は「別の注文」として扱われる）。
        let a = parse_agent_order_request(valid_body()).unwrap();
        let b_body = r#"{
            "client_order_id": "cli_99",
            "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let b = parse_agent_order_request(b_body).unwrap();
        assert_eq!(a.to_key(), b.to_key());
    }
}
