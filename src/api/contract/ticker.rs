//! `TickerContract` — agent API 境界で扱うティッカー（取引所 + シンボル）。
//!
//! 既存 `SerTicker` の `"Exchange:Symbol"` 文字列正規化で踏んだ silent failure
//! （narrative.md §13.2 #3）を構造体化で再発不能にする。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickerContract {
    pub exchange: String,
    pub symbol: String,
}

impl TickerContract {
    pub fn new(exchange: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            exchange: exchange.into(),
            symbol: symbol.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_as_object() {
        let original = TickerContract::new("HyperliquidLinear", "BTC");
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, r#"{"exchange":"HyperliquidLinear","symbol":"BTC"}"#);
        let restored: TickerContract = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn deserializes_from_object_json() {
        let json = r#"{"exchange":"BinanceSpot","symbol":"BTCUSDT"}"#;
        let t: TickerContract = serde_json::from_str(json).unwrap();
        assert_eq!(t.exchange, "BinanceSpot");
        assert_eq!(t.symbol, "BTCUSDT");
    }

    #[test]
    fn rejects_bare_string_json() {
        // Phase 4a silent failure 防止: "Exchange:Symbol" 文字列は受理しない。
        let result: Result<TickerContract, _> = serde_json::from_str(r#""HyperliquidLinear:BTC""#);
        assert!(
            result.is_err(),
            "string form must not deserialize to TickerContract"
        );
    }

    #[test]
    fn rejects_missing_exchange_field() {
        let json = r#"{"symbol":"BTC"}"#;
        let result: Result<TickerContract, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_symbol_field() {
        let json = r#"{"exchange":"HyperliquidLinear"}"#;
        let result: Result<TickerContract, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_unknown_fields() {
        // deny_unknown_fields: 将来のフィールド追加時の silent drop を防ぐ。
        let json = r#"{"exchange":"BinanceSpot","symbol":"BTCUSDT","extra":42}"#;
        let result: Result<TickerContract, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn allows_empty_strings_at_type_level() {
        // 空文字の禁則は API ルート層（replay_api.rs）で 400 を返す責務。
        // 型レベルでは許可して serde の責任範囲を絞る。
        let json = r#"{"exchange":"","symbol":""}"#;
        let t: TickerContract = serde_json::from_str(json).unwrap();
        assert_eq!(t.exchange, "");
        assert_eq!(t.symbol, "");
    }
}
