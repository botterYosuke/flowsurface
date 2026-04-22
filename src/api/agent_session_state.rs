//! Agent 専用 Replay API のセッション固有 state。
//!
//! ADR-0001 / phase4b_agent_replay_api.md §3.3 に基づき、`client_order_id` →
//! `(order_id, request_key)` の冪等性マップを保持する。セッション状態遷移は
//! `VirtualExchange::session_generation()` の変化として観測し、値が変わったら
//! map をクリアする（UI リモコン API から agent state を直接触らない）。

use crate::api::contract::ClientOrderId;
use crate::api::order_request::AgentOrderRequestKey;
use std::collections::HashMap;

/// 発注レコード（冪等性判定と reverse lookup 両用）。
#[derive(Debug, Clone)]
pub struct AgentOrderRecord {
    pub order_id: String,
    pub key: AgentOrderRequestKey,
}

/// agent API セッション（Phase 4b-1 では "default" 固定）の state。
///
/// - `client_order_id → AgentOrderRecord` の冪等性マップ
/// - `last_seen_generation`: 最後に観測した `VirtualExchange::session_generation()`
///   値。現在値と異なれば `handle_lifecycle_bump()` が map をクリアする。
#[derive(Debug, Default)]
pub struct AgentSessionState {
    map: HashMap<ClientOrderId, AgentOrderRecord>,
    last_seen_generation: u64,
}

/// `place_order` 受付の結果。dispatcher はこれを見て HTTP レスポンスを組み立てる。
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceOrderOutcome {
    /// 新規受付。serverが生成した order_id を返す。
    Created { order_id: String },
    /// 既存の (session, client_order_id) に完全一致の再送。同じ order_id を返す。
    IdempotentReplay { order_id: String },
    /// 既存の client_order_id に対して body 構造が異なる → 409 Conflict。
    Conflict { existing_order_id: String },
}

impl AgentSessionState {
    pub fn new() -> Self {
        Self::default()
    }

    /// VirtualExchange の現在世代を受け取り、変化を検知したら map をクリアする。
    /// Phase 4b-1 では SessionLifecycleEvent を生の enum として受け取らず、
    /// 世代カウンタの変化で代用する（単一プロセス想定の最小実装）。
    pub fn observe_generation(&mut self, current_generation: u64) {
        if self.last_seen_generation != current_generation {
            if !self.map.is_empty() {
                log::debug!(
                    "AgentSessionState: lifecycle event detected \
                     (gen {prev} -> {current}), clearing {n} entry(ies)",
                    prev = self.last_seen_generation,
                    current = current_generation,
                    n = self.map.len(),
                );
            }
            self.map.clear();
            self.last_seen_generation = current_generation;
        }
    }

    /// 冪等性チェック + 挿入。呼び出し側は new_order_id（UUID）を事前採番して渡す。
    /// - 未登録: `Created { order_id: new_order_id }` を返し、map に追加
    /// - 登録済み & key 完全一致: 既存 order_id を `IdempotentReplay` で返す
    /// - 登録済み & key 相違: `Conflict` を返す（map は変更しない）
    pub fn place_or_replay(
        &mut self,
        client_order_id: ClientOrderId,
        key: AgentOrderRequestKey,
        new_order_id: String,
    ) -> PlaceOrderOutcome {
        if let Some(existing) = self.map.get(&client_order_id) {
            if existing.key == key {
                return PlaceOrderOutcome::IdempotentReplay {
                    order_id: existing.order_id.clone(),
                };
            }
            return PlaceOrderOutcome::Conflict {
                existing_order_id: existing.order_id.clone(),
            };
        }
        let record = AgentOrderRecord {
            order_id: new_order_id.clone(),
            key,
        };
        self.map.insert(client_order_id, record);
        PlaceOrderOutcome::Created {
            order_id: new_order_id,
        }
    }

    /// `order_id` から `client_order_id` を逆引きする（step レスポンスの fill に
    /// client_order_id を埋めるため）。
    pub fn client_order_id_for(&self, order_id: &str) -> Option<ClientOrderId> {
        self.map
            .iter()
            .find(|(_, v)| v.order_id == order_id)
            .map(|(k, _)| k.clone())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::contract::TickerContract;
    use crate::api::order_request::{AgentOrderSide, AgentOrderType};

    fn cli(s: &str) -> ClientOrderId {
        ClientOrderId::new(s).unwrap()
    }

    fn key(qty: f64) -> AgentOrderRequestKey {
        AgentOrderRequestKey {
            ticker: TickerContract::new("HyperliquidLinear", "BTC"),
            side: AgentOrderSide::Buy,
            qty,
            order_type: AgentOrderType::Market {},
        }
    }

    #[test]
    fn created_on_first_placement() {
        let mut state = AgentSessionState::new();
        let outcome = state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        assert_eq!(outcome, PlaceOrderOutcome::Created { order_id: "ord_uuid_1".to_string() });
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn idempotent_replay_on_exact_rerun() {
        // 同じ client_order_id + 同じ key の再送は既存 order_id を返す。
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        let outcome = state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_should_not_use".to_string());
        assert_eq!(
            outcome,
            PlaceOrderOutcome::IdempotentReplay {
                order_id: "ord_uuid_1".to_string()
            }
        );
        // 2 回目の new_order_id は無視され、map サイズは 1 のまま。
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn conflict_when_same_client_order_id_with_different_body() {
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        let outcome = state.place_or_replay(cli("cli_1"), key(0.2), "ord_uuid_2".to_string());
        assert_eq!(
            outcome,
            PlaceOrderOutcome::Conflict {
                existing_order_id: "ord_uuid_1".to_string()
            }
        );
        assert_eq!(state.len(), 1, "map must not change on conflict");
    }

    #[test]
    fn different_client_order_ids_coexist() {
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_a".to_string());
        state.place_or_replay(cli("cli_2"), key(0.1), "ord_b".to_string());
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn observe_generation_clears_map_when_changed() {
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_a".to_string());
        assert_eq!(state.len(), 1);
        state.observe_generation(1); // 世代変化 — Lifecycle event と等価
        assert_eq!(state.len(), 0, "map must clear on generation bump");
    }

    #[test]
    fn observe_generation_noop_when_unchanged() {
        let mut state = AgentSessionState::new();
        state.observe_generation(0); // 初期値
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_a".to_string());
        state.observe_generation(0); // 同じ世代
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn observe_generation_advances_last_seen() {
        let mut state = AgentSessionState::new();
        state.observe_generation(5);
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_a".to_string());
        state.observe_generation(5); // 同じ 5 ならクリアしない
        assert_eq!(state.len(), 1);
        state.observe_generation(6); // 変化 → クリア
        assert_eq!(state.len(), 0);
    }

    #[test]
    fn client_order_id_for_returns_mapping_after_placement() {
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        assert_eq!(
            state.client_order_id_for("ord_uuid_1"),
            Some(cli("cli_1"))
        );
    }

    #[test]
    fn client_order_id_for_returns_none_for_unknown_order() {
        let state = AgentSessionState::new();
        assert!(state.client_order_id_for("ord_unknown").is_none());
    }

    #[test]
    fn client_order_id_for_returns_none_after_generation_bump() {
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        state.observe_generation(1);
        assert!(state.client_order_id_for("ord_uuid_1").is_none());
    }

    #[test]
    fn after_clear_same_client_order_id_can_be_reused() {
        // lifecycle event（/play 等）の後、同じ client_order_id を別注文に流用できる。
        let mut state = AgentSessionState::new();
        state.place_or_replay(cli("cli_1"), key(0.1), "ord_uuid_1".to_string());
        state.observe_generation(1); // play 等で clear
        let outcome = state.place_or_replay(cli("cli_1"), key(0.2), "ord_uuid_2".to_string());
        assert_eq!(
            outcome,
            PlaceOrderOutcome::Created {
                order_id: "ord_uuid_2".to_string()
            }
        );
    }
}
