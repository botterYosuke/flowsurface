//! Narrative / SnapshotRef / NarrativeAction / NarrativeOutcome モデル。
//!
//! 計画 §3.1 に従い、エージェントの判断根拠・アクション・結果を保持する
//! データ型を定義する。JSON でのクライアント ↔ サーバー往復を想定し、
//! `serde::Serialize` / `serde::Deserialize` を実装する。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// エージェントが記録するナラティブ 1 件。
///
/// `observation_snapshot` 本体は容量が大きくなり得るため SQLite に直接保持せず、
/// `snapshot_ref` 経由で別ファイルに保存する（§3.2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Narrative {
    /// ローカル主キー（サーバー側で常に生成、クライアント指定は無視）。
    pub id: Uuid,

    /// エージェント識別子（Python 側が任意に付ける表示名）。
    pub agent_id: String,

    /// Phase 4b で使用する Fetch.ai uAgent アドレス。4a では常に `None`。
    #[serde(default)]
    pub uagent_address: Option<String>,

    /// 仮想時刻（リプレイ中は `StepClock::now_ms()`、ライブでは実時刻）。
    pub timestamp_ms: i64,

    pub ticker: String,
    pub timeframe: String,

    /// observation_snapshot 本体への参照（メタのみ SQLite に保存）。
    pub snapshot_ref: SnapshotRef,

    /// 自然言語の判断根拠。
    pub reasoning: String,

    pub action: NarrativeAction,

    /// 信頼度 0.0..=1.0。
    pub confidence: f64,

    /// 約定後に自動更新される結果情報。
    #[serde(default)]
    pub outcome: Option<NarrativeOutcome>,

    /// Phase 2 の `VirtualOrder.order_id` と紐付く（`String`）。
    #[serde(default)]
    pub linked_order_id: Option<String>,

    /// 公開フラグ（Phase 4b で送信対象とするかどうか）。
    #[serde(default)]
    pub public: bool,

    /// 実時刻（監査用）。
    pub created_at_ms: i64,

    /// 冪等性キー（クライアント指定、重複 POST 防止用）。
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// observation_snapshot 本体（gzip 圧縮済みファイル）への参照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRef {
    /// `data_path()` からの相対パス（例: `narratives/snapshots/2026/04/21/{uuid}.json.gz`）。
    pub path: PathBuf,
    /// gzip 圧縮後のバイト数。
    pub size_bytes: u64,
    /// SHA-256 ダイジェスト（16 進小文字）。整合性検証・4b の配信 ID に流用。
    pub sha256: String,
}

/// エージェントがとったアクション。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrativeAction {
    pub side: NarrativeSide,
    pub qty: f64,
    pub price: f64,
}

/// Buy / Sell の 2 種。Phase 2 の `PositionSide` とは独立に定義する
/// （ナラティブ API 層の自然な表現を優先し、外部 SDK 連携時にマッピング）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NarrativeSide {
    Buy,
    Sell,
}

/// ナラティブの結果情報。約定イベントで自動更新される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NarrativeOutcome {
    /// 約定価格。
    pub fill_price: f64,
    /// 約定時刻（仮想時刻 ms）。
    pub fill_time_ms: i64,
    /// 決済時刻（ポジションクローズ時刻 ms）。未決済なら `None`。
    #[serde(default)]
    pub closed_at_ms: Option<i64>,
    /// 決済 PnL（確定後のみ）。未決済なら `None`。
    #[serde(default)]
    pub realized_pnl: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_narrative() -> Narrative {
        Narrative {
            id: Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
            agent_id: "agent_alpha".to_string(),
            uagent_address: None,
            timestamp_ms: 1_704_067_200_000,
            ticker: "BTCUSDT".to_string(),
            timeframe: "1h".to_string(),
            snapshot_ref: SnapshotRef {
                path: PathBuf::from("narratives/snapshots/2026/04/21/abc.json.gz"),
                size_bytes: 4096,
                sha256: "ab".repeat(32),
            },
            reasoning: "RSI divergence on 4h, volume confirmed".to_string(),
            action: NarrativeAction {
                side: NarrativeSide::Buy,
                qty: 0.1,
                price: 92_500.0,
            },
            confidence: 0.76,
            outcome: Some(NarrativeOutcome {
                fill_price: 92_510.5,
                fill_time_ms: 1_704_067_260_000,
                closed_at_ms: Some(1_704_070_800_000),
                realized_pnl: Some(42.5),
            }),
            linked_order_id: Some("ord_01JG".to_string()),
            public: true,
            created_at_ms: 1_704_067_200_500,
            idempotency_key: Some("agent_alpha#step_42".to_string()),
        }
    }

    #[test]
    fn roundtrip_json() {
        let original = sample_narrative();
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: Narrative = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, parsed);
    }

    #[test]
    fn side_serializes_as_lowercase() {
        let json = serde_json::to_string(&NarrativeSide::Buy).unwrap();
        assert_eq!(json, "\"buy\"");
        let json = serde_json::to_string(&NarrativeSide::Sell).unwrap();
        assert_eq!(json, "\"sell\"");
    }

    #[test]
    fn outcome_optional_fields_default_to_none() {
        let body = r#"{"fill_price": 100.0, "fill_time_ms": 1}"#;
        let outcome: NarrativeOutcome = serde_json::from_str(body).expect("parse");
        assert_eq!(outcome.closed_at_ms, None);
        assert_eq!(outcome.realized_pnl, None);
    }

    #[test]
    fn narrative_optional_fields_default_on_missing_input() {
        // `uagent_address` / `outcome` / `linked_order_id` / `public` / `idempotency_key`
        // はクライアント未指定で受け入れる。
        let body = r#"{
            "id": "01234567-89ab-cdef-0123-456789abcdef",
            "agent_id": "a",
            "timestamp_ms": 1,
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "snapshot_ref": {
                "path": "x.json.gz",
                "size_bytes": 0,
                "sha256": "00"
            },
            "reasoning": "",
            "action": {"side": "buy", "qty": 0.0, "price": 0.0},
            "confidence": 0.0,
            "created_at_ms": 0
        }"#;
        let n: Narrative = serde_json::from_str(body).expect("parse");
        assert_eq!(n.uagent_address, None);
        assert_eq!(n.outcome, None);
        assert_eq!(n.linked_order_id, None);
        assert!(!n.public);
        assert_eq!(n.idempotency_key, None);
    }
}
