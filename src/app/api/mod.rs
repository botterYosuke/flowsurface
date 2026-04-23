pub(crate) mod helpers;
pub(crate) mod narrative;
pub(crate) mod pane;
pub(crate) mod pane_ticker;
pub(crate) mod replay;

use crate::{Flowsurface, Message};
use crate::{connector, replay_api};
use exchange::{Trade, adapter::StreamKind};
use iced::Task;
use std::collections::HashMap;

/// virtual_engine 未初期化時用の空 PortfolioSnapshot を返す。
fn empty_portfolio_snapshot() -> crate::replay::virtual_exchange::portfolio::PortfolioSnapshot {
    crate::replay::virtual_exchange::portfolio::PortfolioSnapshot {
        cash: 0.0,
        unrealized_pnl: 0.0,
        realized_pnl: 0.0,
        total_equity: 0.0,
        open_positions: Vec::new(),
        closed_positions: Vec::new(),
    }
}

fn serialize_advance_response(resp: &crate::api::advance_request::AdvanceResponse) -> String {
    serde_json::to_string(resp).unwrap_or_else(|e| {
        log::error!("serialize AdvanceResponse failed: {e}");
        r#"{"error":"serialize failed"}"#.to_string()
    })
}

fn dashboard_fill_task(fill: crate::replay::virtual_exchange::FillEvent) -> Task<Message> {
    Task::done(Message::Dashboard {
        layout_id: None,
        event: crate::screen::dashboard::Message::VirtualOrderFilled(fill),
    })
}

pub(crate) fn apply_trade_events_to_virtual_engine(
    virtual_engine: Option<&mut crate::replay::virtual_exchange::VirtualExchangeEngine>,
    trade_events: Vec<(StreamKind, Vec<Trade>)>,
    current_time: u64,
) -> Vec<crate::replay::virtual_exchange::FillEvent> {
    let Some(virtual_engine) = virtual_engine else {
        return Vec::new();
    };

    let mut grouped: Vec<(String, Vec<Trade>)> = Vec::new();
    let mut grouped_index: HashMap<String, usize> = HashMap::new();

    for (stream, trades) in trade_events {
        if trades.is_empty() {
            continue;
        }

        let ticker = stream.ticker_info().ticker.to_string();
        if let Some(index) = grouped_index.get(&ticker).copied() {
            grouped[index].1.extend(trades);
        } else {
            grouped_index.insert(ticker.clone(), grouped.len());
            grouped.push((ticker, trades));
        }
    }

    let mut fills = Vec::new();
    for (ticker, trades) in grouped {
        fills.extend(virtual_engine.on_tick(&ticker, &trades, current_time));
    }
    fills
}

fn sync_narrative_updates_for_fills(
    narrative_store: std::sync::Arc<crate::narrative::store::NarrativeStore>,
    fills: &[crate::replay::virtual_exchange::FillEvent],
    context: &'static str,
) -> Vec<String> {
    if fills.is_empty() {
        return Vec::new();
    }

    let fills = fills.to_vec();
    let thread_name = format!("flowsurface-{context}-narrative-sync");
    let join_handle = match std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    log::warn!("{context}: failed to build tokio runtime for narrative sync: {e}");
                    return Vec::new();
                }
            };

            runtime.block_on(async move {
                let mut updated_narrative_ids = Vec::new();
                for fill in fills {
                    match crate::narrative::service::update_outcome_from_fill_returning_ids(
                        &narrative_store,
                        &fill.order_id,
                        fill.fill_price,
                        crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64(),
                        None,
                    )
                    .await
                    {
                        Ok(ids) => {
                            updated_narrative_ids.extend(ids.into_iter().map(|id| id.to_string()));
                        }
                        Err(e) => log::warn!(
                            "{context}: failed to update narrative outcome for {}: {e}",
                            fill.order_id
                        ),
                    }
                }
                updated_narrative_ids
            })
        }) {
        Ok(join_handle) => join_handle,
        Err(e) => {
            log::warn!("{context}: failed to spawn narrative sync thread: {e}");
            return Vec::new();
        }
    };

    match join_handle.join() {
        Ok(updated_narrative_ids) => updated_narrative_ids,
        Err(_) => {
            log::warn!("{context}: narrative sync thread panicked");
            Vec::new()
        }
    }
}

fn step_fills_for_agent_state(
    agent_session_state: &crate::api::agent_session_state::AgentSessionState,
    fills: &[crate::replay::virtual_exchange::FillEvent],
) -> Vec<crate::api::step_response::StepFill> {
    fills
        .iter()
        .map(|fill| {
            let client_order_id = agent_session_state
                .client_order_id_for(&fill.order_id)
                .map(|client_order_id| client_order_id.as_str().to_string());
            crate::api::step_response::StepFill::from_event(fill, client_order_id)
        })
        .collect()
}

fn build_gui_step_observation(
    replay: &crate::replay::controller::ReplayController,
    virtual_engine: Option<&crate::replay::virtual_exchange::VirtualExchangeEngine>,
    limit: usize,
) -> Option<crate::api::step_response::StepObservation> {
    let state = replay.get_api_state(limit)?;
    let mut ohlcv = Vec::new();
    for (stream, klines) in state.klines {
        for kline in klines {
            ohlcv.push(serde_json::json!({
                "stream": stream,
                "time": kline.time,
                "open": kline.open.to_f64(),
                "high": kline.high.to_f64(),
                "low": kline.low.to_f64(),
                "close": kline.close.to_f64(),
                "volume": kline.volume.total().to_f64(),
            }));
        }
    }

    let recent_trades = state
        .trades
        .into_iter()
        .map(|(stream, trades)| {
            let items: Vec<serde_json::Value> = trades
                .into_iter()
                .map(|trade| {
                    serde_json::json!({
                        "time": trade.time,
                        "price": trade.price.to_f64(),
                        "qty": trade.qty.to_f64(),
                        "is_sell": trade.is_sell,
                    })
                })
                .collect();
            serde_json::json!({ "stream": stream, "trades": items })
        })
        .collect();

    let snapshot_price = replay.last_close_price().unwrap_or(0.0);
    let portfolio = virtual_engine
        .map(|engine| engine.portfolio_snapshot(snapshot_price))
        .unwrap_or_else(empty_portfolio_snapshot);

    Some(crate::api::step_response::StepObservation {
        ohlcv,
        recent_trades,
        portfolio,
    })
}

/// 1 件の仮想 fill に対する非同期タスクを組み立てる:
/// 1. narrative outcome 更新 (`update_outcome_from_fill`) — `Task::perform`
/// 2. Dashboard への `VirtualOrderFilled` 通知 — `Task::done`
///
/// `context` は warn ログの前置詞 (`"step"` / `"advance"` 等)。
/// handle_agent (handlers.rs) と handle_agent_advance (api/mod.rs) で共有する。
/// advance に `stop_on: [Fill]` が追加される際の同期漏れを防ぐため集約してある。
pub(crate) fn narrative_tasks_for_fill(
    narrative_store: std::sync::Arc<crate::narrative::store::NarrativeStore>,
    fill: crate::replay::virtual_exchange::FillEvent,
    context: &'static str,
) -> (Task<Message>, Task<Message>) {
    let order_id = fill.order_id.clone();
    let fill_price = fill.fill_price;
    let fill_time_ms = crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64();
    let side_hint = match fill.side {
        crate::replay::virtual_exchange::PositionSide::Long => {
            Some(crate::narrative::model::NarrativeSide::Buy)
        }
        crate::replay::virtual_exchange::PositionSide::Short => {
            Some(crate::narrative::model::NarrativeSide::Sell)
        }
    };

    let narrative_task = Task::perform(
        async move {
            if let Err(e) = crate::narrative::service::update_outcome_from_fill(
                &narrative_store,
                &order_id,
                fill_price,
                fill_time_ms,
                side_hint,
            )
            .await
            {
                log::warn!("{context}: failed to update narrative outcome for {order_id}: {e}");
            }
        },
        |()| Message::Noop,
    );

    let dashboard_task = dashboard_fill_task(fill);

    (narrative_task, dashboard_task)
}

impl Flowsurface {
    fn start_replay_session(
        &mut self,
        start: &str,
        end: &str,
    ) -> Result<(String, Task<Message>), (u16, String)> {
        let main_window_id = self.main_window.id;
        let Some(layout_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
            return Err((500, r#"{"error":"no active layout"}"#.to_string()));
        };

        let replay_task = {
            let Some(dashboard) = self
                .layout_manager
                .get_mut(layout_id)
                .map(|layout| &mut layout.dashboard)
            else {
                return Err((500, r#"{"error":"no active dashboard"}"#.to_string()));
            };

            let task = self
                .replay
                .initialize_session(start, end, dashboard, main_window_id)
                .map_err(|err| (400, serde_json::json!({ "error": err }).to_string()))?;
            dashboard.is_replay = true;
            dashboard.sync_virtual_mode(main_window_id);
            task
        };

        if self.virtual_engine.is_none() {
            self.virtual_engine = Some(
                crate::replay::virtual_exchange::VirtualExchangeEngine::new(1_000_000.0),
            );
        } else if let Some(engine) = &mut self.virtual_engine {
            engine.reset();
        }

        if let Some(engine) = &mut self.virtual_engine {
            engine.mark_session_started();
        }

        let body = serde_json::json!({
            "ok": true,
            "status": "loading",
            "start": start,
            "end": end,
        })
        .to_string();
        Ok((body, replay_task.map(Message::Replay)))
    }

    fn handle_agent_rewind_request(
        &mut self,
        init_range: Option<(String, String)>,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        // session_id != "default" は routing 段階で 501 NotImplemented に変換済み
        // (replay_api.rs::parse_agent_session_rewind)。Phase 4c の multi-session 対応までは
        // ここに到達するのは "default" のみ。

        if self.replay.is_loading() {
            reply_tx.send_status(409, r#"{"error":"session loading"}"#.to_string());
            return Task::none();
        }

        if !self.replay.is_active() {
            let Some((start, end)) = init_range else {
                reply_tx.send_status(
                    400,
                    r#"{"error":"body required for init (session not initialized)"}"#.to_string(),
                );
                return Task::none();
            };

            return match self.start_replay_session(&start, &end) {
                Ok((body, task)) => {
                    reply_tx.send(body);
                    task
                }
                Err((status, body)) => {
                    reply_tx.send_status(status, body);
                    Task::none()
                }
            };
        }

        let main_window_id = self.main_window.id;
        let Some(layout_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
            reply_tx.send_status(500, r#"{"error":"no active layout"}"#.to_string());
            return Task::none();
        };
        let Some(dashboard) = self
            .layout_manager
            .get_mut(layout_id)
            .map(|l| &mut l.dashboard)
        else {
            reply_tx.send_status(500, r#"{"error":"no active dashboard"}"#.to_string());
            return Task::none();
        };
        self.replay.agent_rewind(dashboard, main_window_id);
        if let Some(ve) = &mut self.virtual_engine {
            ve.reset();
            ve.mark_session_reset();
        }

        let clock_ms = self.replay.current_time_ms().unwrap_or(0);
        let snapshot_price = self.replay.last_close_price().unwrap_or(0.0);
        let final_portfolio = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.portfolio_snapshot(snapshot_price))
            .unwrap_or_else(empty_portfolio_snapshot);
        let body = serde_json::json!({
            "ok": true,
            "clock_ms": clock_ms,
            "final_portfolio": final_portfolio,
        });
        reply_tx.send(body.to_string());
        Task::none()
    }

    fn handle_agent_step_request(&mut self, reply_tx: replay_api::ReplySender) -> Task<Message> {
        use crate::api::step_response::StepResponse;

        if self.replay.is_loading() {
            reply_tx.send_status(409, r#"{"error":"session loading"}"#.to_string());
            return Task::none();
        }

        let Some(end_time) = self.replay.range_end_ms() else {
            reply_tx.send_status(
                404,
                r#"{"error":"session not started","hint":"toggle to Replay mode and start a range first"}"#
                    .to_string(),
            );
            return Task::none();
        };

        let Some(session_generation) = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.session_generation())
        else {
            reply_tx.send_status(500, r#"{"error":"virtual engine unavailable"}"#.to_string());
            return Task::none();
        };
        self.agent_session_state
            .observe_generation(session_generation);

        let main_window_id = self.main_window.id;
        let Some(layout_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
            reply_tx.send_status(500, r#"{"error":"no active layout"}"#.to_string());
            return Task::none();
        };

        let Some((current_time, trade_events)) = ({
            let Some(dashboard) = self
                .layout_manager
                .get_mut(layout_id)
                .map(|layout| &mut layout.dashboard)
            else {
                reply_tx.send_status(500, r#"{"error":"no active dashboard"}"#.to_string());
                return Task::none();
            };
            self.replay.agent_step(dashboard, main_window_id)
        }) else {
            reply_tx.send_status(
                404,
                r#"{"error":"session not started","hint":"toggle to Replay mode and start a range first"}"#
                    .to_string(),
            );
            return Task::none();
        };

        let fills = apply_trade_events_to_virtual_engine(
            self.virtual_engine.as_mut(),
            trade_events,
            current_time,
        );
        let updated_narrative_ids =
            sync_narrative_updates_for_fills(self.narrative_store.clone(), &fills, "step");
        let Some(observation) =
            build_gui_step_observation(&self.replay, self.virtual_engine.as_ref(), 200)
        else {
            reply_tx.send_status(
                500,
                r#"{"error":"observation build failed: session not active"}"#.to_string(),
            );
            return Task::none();
        };

        let response = StepResponse::new(
            current_time,
            current_time >= end_time,
            observation,
            step_fills_for_agent_state(&self.agent_session_state, &fills),
        )
        .with_updated_narrative_ids(updated_narrative_ids);

        match response.to_json_string() {
            Ok(json) => reply_tx.send(json),
            Err(e) => {
                reply_tx.send_status(
                    500,
                    serde_json::json!({ "error": format!("serialize failed: {e}") }).to_string(),
                );
                return Task::none();
            }
        }

        let mut tasks = Vec::new();
        if !fills.is_empty() {
            tasks.push(self.refresh_narrative_markers_task());
        }
        tasks.extend(fills.into_iter().map(dashboard_fill_task));
        Task::batch(tasks)
    }

    fn handle_agent_place_order_request(
        &mut self,
        request: crate::api::order_request::AgentOrderRequest,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        use crate::api::agent_session_state::PlaceOrderOutcome;
        use crate::api::order_request::{AgentOrderSide, AgentOrderType};
        use crate::replay::virtual_exchange::{
            PositionSide, VirtualOrder, VirtualOrderStatus, VirtualOrderType,
        };

        if self.replay.is_loading() {
            reply_tx.send_status(409, r#"{"error":"session loading"}"#.to_string());
            return Task::none();
        }

        let Some(current_time) = self.replay.current_time_ms() else {
            reply_tx.send_status(
                404,
                r#"{"error":"session not started","hint":"toggle to Replay mode and start a range first"}"#
                    .to_string(),
            );
            return Task::none();
        };

        let Some(session_generation) = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.session_generation())
        else {
            reply_tx.send_status(500, r#"{"error":"virtual engine unavailable"}"#.to_string());
            return Task::none();
        };
        self.agent_session_state
            .observe_generation(session_generation);

        let outcome = self.agent_session_state.place_or_replay(
            request.client_order_id.clone(),
            request.to_key(),
            uuid::Uuid::new_v4().to_string(),
        );

        match outcome {
            PlaceOrderOutcome::Created { order_id } => {
                let side = match request.side {
                    AgentOrderSide::Buy => PositionSide::Long,
                    AgentOrderSide::Sell => PositionSide::Short,
                };
                let order_type = match request.order_type {
                    AgentOrderType::Market {} => VirtualOrderType::Market,
                    AgentOrderType::Limit { price } => VirtualOrderType::Limit { price },
                };

                let Some(virtual_engine) = &mut self.virtual_engine else {
                    reply_tx
                        .send_status(500, r#"{"error":"virtual engine unavailable"}"#.to_string());
                    return Task::none();
                };
                virtual_engine.place_order(VirtualOrder {
                    order_id: order_id.clone(),
                    ticker: request.ticker.symbol.clone(),
                    side,
                    qty: request.qty,
                    order_type,
                    placed_time_ms: current_time,
                    status: VirtualOrderStatus::Pending,
                });

                reply_tx.send(
                    serde_json::json!({
                        "order_id": order_id,
                        "client_order_id": request.client_order_id.as_str(),
                        "idempotent_replay": false,
                    })
                    .to_string(),
                );
            }
            PlaceOrderOutcome::IdempotentReplay { order_id } => {
                reply_tx.send(
                    serde_json::json!({
                        "order_id": order_id,
                        "client_order_id": request.client_order_id.as_str(),
                        "idempotent_replay": true,
                    })
                    .to_string(),
                );
            }
            PlaceOrderOutcome::Conflict { existing_order_id } => {
                reply_tx.send_status(
                    409,
                    serde_json::json!({
                        "error": "client_order_id conflict with different request body",
                        "existing_order_id": existing_order_id,
                    })
                    .to_string(),
                );
            }
        }

        Task::none()
    }

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
                use replay_api::AgentSessionCommand;
                match cmd.clone() {
                    AgentSessionCommand::Step => {
                        return self.handle_agent_step_request(reply_tx);
                    }
                    AgentSessionCommand::PlaceOrder { request } => {
                        return self.handle_agent_place_order_request(*request, reply_tx);
                    }
                    _ => {}
                }
                match cmd {
                    AgentSessionCommand::Step | AgentSessionCommand::PlaceOrder { .. } => {
                        // Step / PlaceOrder は GUI で未実装。
                        // agent 駆動のバックテストは `--headless` ランタイムを使う前提。
                        // UI からの 1 bar 進行は `Message::Agent(AgentMessage::Step)` 経由
                        // （UI ▶ ボタン）で別途利用可能。
                        reply_tx.send_status(
                            501,
                            r#"{"error":"agent session commands not yet supported in GUI mode; use headless"}"#
                                .to_string(),
                        );
                    }
                    AgentSessionCommand::Advance { request } => {
                        // ADR-0001 §3: GUI / Headless 両方で受理（旧 400 ガードは撤廃）。
                        // GUI 実装は MVP — advance 本体のみ。stop_on / include_fills の
                        // 完全サポートは別サブフェーズで shared helper を抽出する際に追加。
                        return self.handle_agent_advance(*request, reply_tx);
                    }
                    AgentSessionCommand::RewindToStart { init_range } => {
                        return self.handle_agent_rewind_request(init_range, reply_tx);
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

    fn handle_agent_advance_impl(
        &mut self,
        request: crate::api::advance_request::AgentAdvanceRequest,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        use crate::api::advance_request::{
            AdvanceResponse, AdvanceStopCondition, AdvanceStoppedReason,
        };

        if self.replay.is_loading() {
            reply_tx.send_status(409, r#"{"error":"session loading"}"#.to_string());
            return Task::none();
        }

        let (start_time, end_time) = match (
            self.replay
                .current_time_ms()
                .filter(|_| self.replay.is_active()),
            self.replay.range_end_ms(),
        ) {
            (Some(current), Some(end)) => (current, end),
            _ => {
                reply_tx.send_status(
                    404,
                    r#"{"error":"session not started","hint":"toggle to Replay mode and start a range first"}"#
                        .to_string(),
                );
                return Task::none();
            }
        };

        let Some(session_generation) = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.session_generation())
        else {
            reply_tx.send_status(500, r#"{"error":"virtual engine unavailable"}"#.to_string());
            return Task::none();
        };
        self.agent_session_state
            .observe_generation(session_generation);

        let until_ms = request.until_ms.as_u64();
        if until_ms <= start_time {
            let snapshot_price = self.replay.last_close_price().unwrap_or(0.0);
            let final_portfolio = self
                .virtual_engine
                .as_ref()
                .map(|ve| ve.portfolio_snapshot(snapshot_price))
                .unwrap_or_else(empty_portfolio_snapshot);
            let resp = AdvanceResponse {
                clock_ms: crate::api::contract::EpochMs::from(start_time),
                stopped_reason: AdvanceStoppedReason::UntilReached,
                ticks_advanced: 0,
                aggregate_fills: 0,
                aggregate_updated_narratives: 0,
                fills: request.include_fills.then(Vec::new),
                final_portfolio,
            };
            reply_tx.send(serialize_advance_response(&resp));
            return Task::none();
        }

        let main_window_id = self.main_window.id;
        let Some(layout_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
            reply_tx.send_status(500, r#"{"error":"no active layout"}"#.to_string());
            return Task::none();
        };

        let stop_on_fill = request.stop_on.contains(&AdvanceStopCondition::Fill);
        let stop_on_narrative = request.stop_on.contains(&AdvanceStopCondition::Narrative);
        let stopped_reason: AdvanceStoppedReason;
        let mut ticks_advanced = 0_u64;
        let mut aggregate_fills = 0_usize;
        let mut aggregate_updated_narratives = 0_usize;
        let mut collected_fills = Vec::new();
        let mut dashboard_fill_tasks = Vec::new();
        let mut needs_marker_refresh = false;

        loop {
            let before_time = self.replay.current_time_ms().unwrap_or(start_time);
            if before_time >= end_time {
                stopped_reason = AdvanceStoppedReason::End;
                break;
            }
            if before_time >= until_ms {
                stopped_reason = AdvanceStoppedReason::UntilReached;
                break;
            }

            let Some((current_time, trade_events)) = ({
                let Some(dashboard) = self
                    .layout_manager
                    .get_mut(layout_id)
                    .map(|layout| &mut layout.dashboard)
                else {
                    reply_tx.send_status(500, r#"{"error":"no active dashboard"}"#.to_string());
                    return Task::none();
                };
                self.replay
                    .agent_advance_next(dashboard, main_window_id, until_ms)
            }) else {
                reply_tx.send_status(404, r#"{"error":"session not active"}"#.to_string());
                return Task::none();
            };

            if current_time <= before_time {
                stopped_reason = if current_time >= end_time {
                    AdvanceStoppedReason::End
                } else {
                    AdvanceStoppedReason::UntilReached
                };
                break;
            }

            ticks_advanced += 1;

            let fills = apply_trade_events_to_virtual_engine(
                self.virtual_engine.as_mut(),
                trade_events,
                current_time,
            );
            let updated_narrative_ids =
                sync_narrative_updates_for_fills(self.narrative_store.clone(), &fills, "advance");
            let tick_updated_narratives = updated_narrative_ids.len();

            aggregate_fills += fills.len();
            aggregate_updated_narratives += tick_updated_narratives;

            if !fills.is_empty() {
                needs_marker_refresh = true;
            }
            if request.include_fills {
                collected_fills.extend(step_fills_for_agent_state(
                    &self.agent_session_state,
                    &fills,
                ));
            }
            let tick_had_fills = !fills.is_empty();
            dashboard_fill_tasks.extend(fills.into_iter().map(dashboard_fill_task));

            if stop_on_fill && tick_had_fills {
                stopped_reason = AdvanceStoppedReason::Fill;
                break;
            }
            if stop_on_narrative && tick_updated_narratives > 0 {
                stopped_reason = AdvanceStoppedReason::Narrative;
                break;
            }
            if current_time >= end_time {
                stopped_reason = AdvanceStoppedReason::End;
                break;
            }
            if current_time >= until_ms {
                stopped_reason = AdvanceStoppedReason::UntilReached;
                break;
            }
        }

        let clock_ms = self.replay.current_time_ms().unwrap_or(start_time);
        let snapshot_price = self.replay.last_close_price().unwrap_or(0.0);
        let final_portfolio = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.portfolio_snapshot(snapshot_price))
            .unwrap_or_else(empty_portfolio_snapshot);

        let resp = AdvanceResponse {
            clock_ms: crate::api::contract::EpochMs::from(clock_ms),
            stopped_reason,
            ticks_advanced,
            aggregate_fills,
            aggregate_updated_narratives,
            fills: request.include_fills.then_some(collected_fills),
            final_portfolio,
        };
        reply_tx.send(serialize_advance_response(&resp));

        let mut tasks = Vec::new();
        if needs_marker_refresh {
            tasks.push(self.refresh_narrative_markers_task());
        }
        tasks.extend(dashboard_fill_tasks);
        Task::batch(tasks)
    }

    /// `POST /api/agent/session/:id/advance` (GUI mode) の MVP ハンドラ。
    ///
    /// ADR-0001 §3 / §5 に基づく。UI ボタン経由の `AgentMessage::Advance` と同じ
    /// コアロジック（`ReplayController::agent_advance` + `virtual_engine.on_tick`
    /// + narrative outcome 非同期更新）を再利用し、HTTP レスポンスとして
    /// `AdvanceResponse` を返す。
    ///
    /// 制約（headless 実装との差分）:
    /// - `stop_on`（fill / narrative 到達時に中途停止）は未サポート。指定時は 501。
    /// - `include_fills`（fills 配列をレスポンスに同梱）は未サポート。指定時は 501。
    /// - `aggregate_updated_narratives` は常に `0` を返す（narrative 更新は非同期
    ///   タスクで発火するが、完了を待たずに応答するため正確な件数を集計できない）。
    ///
    /// 完全な parity が必要な場合は `--headless` ランタイムを利用すること。
    /// 将来 shared helper を抽出した時点で制約を解消する。
    #[allow(unreachable_code)]
    pub(crate) fn handle_agent_advance(
        &mut self,
        request: crate::api::advance_request::AgentAdvanceRequest,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        return self.handle_agent_advance_impl(request.clone(), reply_tx.clone());

        use crate::api::advance_request::{AdvanceResponse, AdvanceStoppedReason};

        // session_id != "default" は routing 段階で 501 NotImplemented に変換済み
        // (replay_api.rs::parse_agent_session_advance)。

        if !request.stop_on.is_empty() || request.include_fills {
            reply_tx.send_status(
                501,
                r#"{"error":"stop_on / include_fills not yet supported in GUI mode; use headless"}"#
                    .to_string(),
            );
            return Task::none();
        }

        // session 状態チェック（headless と同じルール）。
        // Loading は 409、Idle は 404、Active のみ進行する。
        // `current_time_ms()` は Loading でも Some を返すため、is_loading を先に判定する。
        // 409 は rewind-to-start / headless advance と一致させている（ADR-0001 §6）。
        if self.replay.is_loading() {
            reply_tx.send_status(409, r#"{"error":"session loading"}"#.to_string());
            return Task::none();
        }
        let (start_time, end_time) = match (
            self.replay
                .is_active()
                .then(|| self.replay.current_time_ms())
                .flatten(),
            self.replay.range_end_ms(),
        ) {
            (Some(t), Some(end)) => (t, end),
            _ => {
                reply_tx.send_status(
                    404,
                    r#"{"error":"session not started","hint":"toggle to Replay mode and start a range first"}"#
                        .to_string(),
                );
                return Task::none();
            }
        };

        let until_ms = request.until_ms.as_u64();

        if until_ms <= start_time {
            // 既に until_ms 到達済 — 進行 0 で成功を返す（advance 前の価格で OK）。
            let snapshot_price = self.replay.last_close_price().unwrap_or(0.0);
            let portfolio = self
                .virtual_engine
                .as_ref()
                .map(|ve| ve.portfolio_snapshot(snapshot_price))
                .unwrap_or_else(empty_portfolio_snapshot);
            let resp = AdvanceResponse {
                clock_ms: crate::api::contract::EpochMs::from(start_time),
                stopped_reason: AdvanceStoppedReason::UntilReached,
                ticks_advanced: 0,
                aggregate_fills: 0,
                aggregate_updated_narratives: 0,
                fills: None,
                final_portfolio: portfolio,
            };
            reply_tx.send(serialize_advance_response(&resp));
            return Task::none();
        }

        // layout / dashboard を取り出す
        let main_window_id = self.main_window.id;
        let Some(layout_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
            reply_tx.send_status(500, r#"{"error":"no active layout"}"#.to_string());
            return Task::none();
        };
        let Some(dashboard) = self
            .layout_manager
            .get_mut(layout_id)
            .map(|l| &mut l.dashboard)
        else {
            reply_tx.send_status(500, r#"{"error":"no active dashboard"}"#.to_string());
            return Task::none();
        };

        // agent_advance は cap_ms を受け取って min(now + cap, range.end) まで進める。
        // UI ボタン経由では UI_ADVANCE_CAP_MS が適用されるが、HTTP 経由では ADR §5 の
        // 「HTTP の cap なし」原則に従い `until_ms - now_ms` をそのまま渡す。
        let cap_ms = until_ms.saturating_sub(start_time);
        let outcome = self.replay.agent_advance(dashboard, main_window_id, cap_ms);

        let (current_time, trade_events) = match outcome {
            Some(o) => o,
            None => {
                reply_tx.send_status(404, r#"{"error":"session not active"}"#.to_string());
                return Task::none();
            }
        };

        // fills 集計 + narrative outcome 非同期更新（handle_agent と同じパターン）
        let mut aggregate_fills: usize = 0;
        let mut narrative_tasks: Vec<Task<Message>> = Vec::new();
        let mut fill_msgs: Vec<Task<Message>> = Vec::new();
        let mut needs_marker_refresh = false;

        if let Some(ve) = &mut self.virtual_engine {
            for (stream, trades) in trade_events {
                let ticker_str = stream.ticker_info().ticker.to_string();
                let fills = ve.on_tick(&ticker_str, &trades, current_time);
                if !fills.is_empty() {
                    needs_marker_refresh = true;
                }
                aggregate_fills += fills.len();
                for fill in fills {
                    let (narrative_task, dashboard_task) =
                        narrative_tasks_for_fill(self.narrative_store.clone(), fill, "advance");
                    narrative_tasks.push(narrative_task);
                    fill_msgs.push(dashboard_task);
                }
            }
        }

        // ticks_advanced は「進行した bar 数」。
        // `tick_until` は target が range.end の場合のみ非整列を許すため、末端 partial
        // step を 1 bar としてカウントできるよう ceil 除算を使う。
        let ticks_advanced = self
            .replay
            .step_size_ms()
            .filter(|&s| s > 0)
            .map(|s| {
                let delta = current_time.saturating_sub(start_time);
                delta.saturating_add(s - 1) / s
            })
            .unwrap_or(0);

        let stopped_reason = if current_time >= end_time {
            AdvanceStoppedReason::End
        } else {
            AdvanceStoppedReason::UntilReached
        };

        // advance 後の最新 close 価格で portfolio を評価する。
        // advance 前の snapshot_price を使うと unrealized_pnl / total_equity がズレる。
        let snapshot_price = self.replay.last_close_price().unwrap_or(0.0);
        let final_portfolio = self
            .virtual_engine
            .as_ref()
            .map(|ve| ve.portfolio_snapshot(snapshot_price))
            .unwrap_or_else(empty_portfolio_snapshot);

        let resp = AdvanceResponse {
            clock_ms: crate::api::contract::EpochMs::from(current_time),
            stopped_reason,
            ticks_advanced,
            aggregate_fills,
            aggregate_updated_narratives: 0, // MVP: async 更新のため集計省略
            fills: None,
            final_portfolio,
        };
        reply_tx.send(serialize_advance_response(&resp));

        let mut tasks: Vec<Task<Message>> = Vec::new();
        if needs_marker_refresh {
            tasks.push(self.refresh_narrative_markers_task());
        }
        tasks.extend(narrative_tasks);
        tasks.extend(fill_msgs);
        Task::batch(tasks)
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
