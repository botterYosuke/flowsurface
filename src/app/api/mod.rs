pub(crate) mod helpers;
pub(crate) mod narrative;
pub(crate) mod pane;
pub(crate) mod pane_ticker;
pub(crate) mod replay;

use crate::{Flowsurface, Message};
use crate::{connector, replay_api};
use iced::Task;

impl Flowsurface {
    pub(crate) fn handle_replay_api(
        &mut self,
        command: replay_api::ApiCommand,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        use replay_api::ApiCommand;

        match command {
            ApiCommand::Replay(cmd) => {
                return self.handle_replay_commands(cmd, reply_tx);
            }
            ApiCommand::Pane(cmd) => {
                let (status, body, task) = self.handle_pane_api(cmd);
                reply_tx.send_status(status, body);
                return task;
            }
            ApiCommand::Auth(cmd) => {
                let body = self.handle_auth_api(cmd);
                reply_tx.send(body);
            }
            ApiCommand::FetchBuyingPower => {
                return Task::perform(connector::order::fetch_buying_power(), move |result| {
                    Message::BuyingPowerApiResult {
                        reply: reply_tx,
                        result,
                    }
                });
            }
            ApiCommand::TachibanaNewOrder { req } => {
                return Task::perform(connector::order::submit_new_order(*req), move |result| {
                    Message::TachibanaOrderApiResult {
                        reply: reply_tx,
                        result,
                    }
                });
            }
            ApiCommand::FetchTachibanaOrders { eig_day } => {
                return Task::perform(connector::order::fetch_orders(eig_day), move |result| {
                    Message::FetchOrdersApiResult {
                        reply: reply_tx,
                        result,
                    }
                });
            }
            ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day } => {
                return Task::perform(
                    connector::order::fetch_order_detail(order_num, eig_day),
                    move |result| Message::FetchOrderDetailApiResult {
                        reply: reply_tx,
                        result,
                    },
                );
            }
            ApiCommand::TachibanaCorrectOrder { req } => {
                return Task::perform(
                    connector::order::submit_correct_order(*req),
                    move |result| Message::ModifyOrderApiResult {
                        reply: reply_tx,
                        result,
                    },
                );
            }
            ApiCommand::TachibanaOrderCancel { req } => {
                return Task::perform(connector::order::submit_cancel_order(*req), move |result| {
                    Message::ModifyOrderApiResult {
                        reply: reply_tx,
                        result,
                    }
                });
            }
            ApiCommand::FetchTachibanaHoldings { issue_code } => {
                return Task::perform(
                    connector::order::fetch_holdings(issue_code),
                    move |result| Message::FetchHoldingsApiResult {
                        reply: reply_tx,
                        result,
                    },
                );
            }
            ApiCommand::VirtualExchange(cmd) => {
                return self.handle_virtual_exchange_commands(cmd, reply_tx);
            }
            ApiCommand::Narrative(cmd) => {
                return self.handle_narrative_api(cmd, reply_tx);
            }
            ApiCommand::AgentSession(cmd) => {
                // Phase 4b-1 サブフェーズ B: ルーティングのみ実装済み。
                // サブフェーズ C 以降で step の副作用同梱レスポンスを実装する。
                use replay_api::AgentSessionCommand;
                match cmd {
                    AgentSessionCommand::Step { .. } => {
                        reply_tx.send_status(
                            501,
                            r#"{"error":"agent session step not yet implemented (Phase 4b-1 subphase C pending)"}"#
                                .to_string(),
                        );
                    }
                }
            }
            #[cfg(debug_assertions)]
            ApiCommand::Test(cmd) => {
                let (body, task) = self.handle_test_api(cmd);
                reply_tx.send(body);
                return task;
            }
        }
        Task::none()
    }

    pub(crate) fn handle_auth_api(&self, cmd: replay_api::AuthCommand) -> String {
        use replay_api::AuthCommand;
        match cmd {
            AuthCommand::TachibanaSessionStatus => {
                format_tachibana_session_status(connector::auth::get_session().as_ref())
            }
            AuthCommand::TachibanaLogout => {
                connector::auth::clear_session();
                log::info!("Tachibana: explicit logout via API (session cleared)");
                serde_json::json!({ "ok": true, "action": "logout" }).to_string()
            }
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn handle_test_api(
        &mut self,
        cmd: replay_api::TestCommand,
    ) -> (String, Task<Message>) {
        use replay_api::TestCommand;
        match cmd {
            TestCommand::TachibanaDeletePersistedSession => {
                connector::auth::delete_all_sessions();
                (
                    serde_json::json!({"ok": true, "action": "delete-persisted-session"})
                        .to_string(),
                    Task::none(),
                )
            }
        }
    }

    pub(crate) fn handle_api_buying_power(
        &self,
        reply: replay_api::ReplySender,
        result: Result<
            (
                exchange::adapter::tachibana::BuyingPowerResponse,
                exchange::adapter::tachibana::MarginPowerResponse,
            ),
            String,
        >,
    ) {
        let body = match result {
            Ok((cash, margin)) => serde_json::json!({
                "cash_buying_power": cash.cash_buying_power,
                "nisa_growth_buying_power": cash.nisa_growth_buying_power,
                "shortage_flag": cash.shortage_flag,
                "margin_new_order_power": margin.margin_new_order_power,
                "maintenance_margin_rate": margin.maintenance_margin_rate,
                "margin_call_flag": margin.margin_call_flag,
            })
            .to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }

    pub(crate) fn handle_api_tachibana_order(
        &self,
        reply: replay_api::ReplySender,
        result: Result<exchange::adapter::tachibana::NewOrderResponse, String>,
    ) {
        let body = match result {
            Ok(resp) => serde_json::json!({
                "order_number": resp.order_number,
                "eig_day": resp.eig_day,
                "delivery_amount": resp.delivery_amount,
                "commission": resp.commission,
                "consumption_tax": resp.consumption_tax,
                "order_datetime": resp.order_datetime,
                "warning_code": resp.warning_code,
                "warning_text": resp.warning_text,
            })
            .to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }

    pub(crate) fn handle_api_fetch_orders(
        &self,
        reply: replay_api::ReplySender,
        result: Result<Vec<exchange::adapter::tachibana::OrderRecord>, String>,
    ) {
        let body = match result {
            Ok(orders) => serde_json::to_string(&serde_json::json!({ "orders": orders
                .iter()
                .map(|o| serde_json::json!({
                    "order_num": o.order_num,
                    "issue_code": o.issue_code,
                    "order_qty": o.order_qty,
                    "current_qty": o.current_qty,
                    "order_price": o.order_price,
                    "order_datetime": o.order_datetime,
                    "status_text": o.status_text,
                    "executed_qty": o.executed_qty,
                    "executed_price": o.executed_price,
                    "eig_day": o.eig_day,
                }))
                .collect::<Vec<_>>()
            }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }

    pub(crate) fn handle_api_fetch_order_detail(
        &self,
        reply: replay_api::ReplySender,
        result: Result<Vec<exchange::adapter::tachibana::ExecutionRecord>, String>,
    ) {
        let body = match result {
            Ok(executions) => serde_json::to_string(&serde_json::json!({ "executions": executions
                .iter()
                .map(|e| serde_json::json!({
                    "exec_qty": e.exec_qty,
                    "exec_price": e.exec_price,
                    "exec_datetime": e.exec_datetime,
                }))
                .collect::<Vec<_>>()
            }))
            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }

    pub(crate) fn handle_api_modify_order(
        &self,
        reply: replay_api::ReplySender,
        result: Result<exchange::adapter::tachibana::ModifyOrderResponse, String>,
    ) {
        let body = match result {
            Ok(resp) => serde_json::json!({
                "order_number": resp.order_number,
                "eig_day": resp.eig_day,
                "order_datetime": resp.order_datetime,
            })
            .to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }

    pub(crate) fn handle_api_fetch_holdings(
        &self,
        reply: replay_api::ReplySender,
        result: Result<u64, String>,
    ) {
        let body = match result {
            Ok(qty) => serde_json::json!({ "holdings_qty": qty }).to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        reply.send(body);
    }
}

/// `/api/auth/tachibana/status` のレスポンス JSON を組み立てる。
///
/// - セッションなし: `{"session": "none"}`
/// - デモ保存: `{"session": "present", "environment": "demo"}`
/// - 本番保存: `{"session": "present", "environment": "prod"}`
///
/// Python 側テスト (`@pytest.mark.tachibana_demo`) が `environment == "demo"`
/// を確認して本番口座での誤発注を skip する判定に使用する。
fn format_tachibana_session_status(
    session: Option<&exchange::adapter::tachibana::TachibanaSession>,
) -> String {
    match session {
        None => serde_json::json!({ "session": "none" }).to_string(),
        Some(s) => serde_json::json!({
            "session": "present",
            "environment": if s.is_demo { "demo" } else { "prod" },
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_tachibana_session_status;
    use exchange::adapter::tachibana::TachibanaSession;

    fn make_session(is_demo: bool) -> TachibanaSession {
        TachibanaSession {
            url_request: "r".to_string(),
            url_master: "m".to_string(),
            url_price: "p".to_string(),
            url_event: "e".to_string(),
            url_event_ws: "ws".to_string(),
            is_demo,
        }
    }

    #[test]
    fn status_none_when_no_session() {
        let body = format_tachibana_session_status(None);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["session"], "none");
        assert!(
            v.get("environment").is_none(),
            "セッション未保存時は environment を含めない"
        );
    }

    #[test]
    fn status_demo_when_session_is_demo() {
        let session = make_session(true);
        let body = format_tachibana_session_status(Some(&session));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["session"], "present");
        assert_eq!(v["environment"], "demo");
    }

    #[test]
    fn status_prod_when_session_is_not_demo() {
        let session = make_session(false);
        let body = format_tachibana_session_status(Some(&session));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["session"], "present");
        assert_eq!(v["environment"], "prod");
    }
}
