//! `TickerContract` — agent API 境界で扱うティッカー（取引所 + シンボル）。
//!
//! 既存 `SerTicker` の `"Exchange:Symbol"` 文字列正規化で踏んだ silent failure
//! （narrative.md §13.2 #3）を構造体化で再発不能にする。

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
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

// Deserialize をカスタム実装して、空文字・未知フィールドを拒否する。
// `#[serde(deny_unknown_fields)]` と同等の挙動 + 空文字ガードを兼ねる。
// 空文字は API ルート層では検知できず silent に約定しない VirtualOrder を
// 生むリスクがあるため、型境界で封じる（`ClientOrderId` と同方針）。
impl<'de> Deserialize<'de> for TickerContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Exchange,
            Symbol,
        }

        struct TickerContractVisitor;

        impl<'de> Visitor<'de> for TickerContractVisitor {
            type Value = TickerContract;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("ticker object with non-empty `exchange` and `symbol` fields")
            }

            fn visit_map<V>(self, mut map: V) -> Result<TickerContract, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut exchange: Option<String> = None;
                let mut symbol: Option<String> = None;
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::Exchange => {
                            if exchange.is_some() {
                                return Err(de::Error::duplicate_field("exchange"));
                            }
                            exchange = Some(map.next_value()?);
                        }
                        Field::Symbol => {
                            if symbol.is_some() {
                                return Err(de::Error::duplicate_field("symbol"));
                            }
                            symbol = Some(map.next_value()?);
                        }
                    }
                }
                let exchange = exchange.ok_or_else(|| de::Error::missing_field("exchange"))?;
                let symbol = symbol.ok_or_else(|| de::Error::missing_field("symbol"))?;
                if exchange.is_empty() {
                    return Err(de::Error::custom("ticker.exchange must not be empty"));
                }
                if symbol.is_empty() {
                    return Err(de::Error::custom("ticker.symbol must not be empty"));
                }
                Ok(TickerContract { exchange, symbol })
            }
        }

        deserializer.deserialize_map(TickerContractVisitor)
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
    fn rejects_empty_exchange() {
        let json = r#"{"exchange":"","symbol":"BTC"}"#;
        let result: Result<TickerContract, _> = serde_json::from_str(json);
        let err = result.expect_err("empty exchange must be rejected");
        assert!(
            err.to_string().contains("exchange"),
            "error should mention exchange field: {err}"
        );
    }

    #[test]
    fn rejects_empty_symbol() {
        let json = r#"{"exchange":"BinanceSpot","symbol":""}"#;
        let result: Result<TickerContract, _> = serde_json::from_str(json);
        let err = result.expect_err("empty symbol must be rejected");
        assert!(
            err.to_string().contains("symbol"),
            "error should mention symbol field: {err}"
        );
    }
}
