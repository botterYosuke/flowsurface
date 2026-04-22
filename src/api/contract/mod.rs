//! Agent 専用 Replay API の境界型。
//!
//! ADR-0001 / Phase 4b-1 計画 §3 参照。既存 `SerTicker` や `exchange/` 内部型には
//! 混ぜず、API 境界に限定して使用する。

pub mod client_order_id;
pub mod epoch;
pub mod ticker;

pub use client_order_id::{ClientOrderId, ClientOrderIdError};
pub use epoch::EpochMs;
pub use ticker::TickerContract;
