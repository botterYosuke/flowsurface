//! Phase 4a: Agent ナラティブ基盤。
//!
//! エージェントが行動するたびに「なぜそう判断したか」をローカルに保存し、
//! 将来（Phase 4b）の ASI 連携の入力となる構造化データを提供する。
//!
//! # モジュール構成
//! - `model`: Narrative / SnapshotRef / NarrativeAction / NarrativeOutcome データ型
//! - `snapshot_store`: observation_snapshot の gzip + sha256 ファイル永続化
//! - `store`: SQLite メタデータストア（`NarrativeStore` trait + rusqlite 実装）
//! - `marker`: チャート Canvas マーカー描画用

pub mod model;
pub mod snapshot_store;
pub mod store;
