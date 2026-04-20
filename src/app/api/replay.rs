use crate::replay::{self, ReplayMessage, ReplayUserMessage};
use crate::replay_api;
use crate::screen::dashboard;
use crate::widget::toast::Toast;
use crate::{Flowsurface, Message};
use iced::Task;

use super::helpers::{KlineStateItem, TradeStateItem};

impl Flowsurface {
    /// `ReplayCommand` を処理する。`handle_replay_api` から呼ばれる。
    /// 成功時は `reply_replay_status(self)` の文字列を reply_fn に渡すため、
    /// 呼び出し元で `reply_tx.send(reply_replay_status(self))` を行う。
    pub(crate) fn handle_replay_commands(
        &mut self,
        cmd: replay::ReplayCommand,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        let reply_replay_status = |this: &Self| {
            serde_json::to_string(&this.replay.to_status())
                .unwrap_or_else(|_| r#"{"error":"failed to serialize replay status"}"#.to_string())
        };

        match cmd {
            replay::ReplayCommand::GetStatus => {
                // headless CI 対応: iced::time::every が発火しない環境でも
                // API ポーリング毎に clock を前進させる。
                let mut tick_tasks: Vec<Task<Message>> = Vec::new();
                if self.replay.is_playing() {
                    let now = std::time::Instant::now();
                    let main_window_id = self.main_window.id;
                    if let Some(id) = self.layout_manager.active_layout_id().map(|l| l.unique)
                        && let Some(dash) =
                            self.layout_manager.get_mut(id).map(|l| &mut l.dashboard)
                    {
                        let outcome = self.replay.tick(now, dash, main_window_id);
                        if outcome.reached_end {
                            self.notifications.push(Toast::info("Replay reached end"));
                        }
                        for (stream, trades, update_t) in outcome.trade_events {
                            if let Some(engine) = &mut self.virtual_engine {
                                let ticker = stream.ticker_info().ticker.to_string();
                                let clock_ms = self.replay.current_time_ms().unwrap_or(0);
                                let fills = engine.on_tick(&ticker, &trades, clock_ms);
                                for fill in fills {
                                    tick_tasks.push(Task::done(Message::Dashboard {
                                        layout_id: None,
                                        event: dashboard::Message::VirtualOrderFilled(fill),
                                    }));
                                }
                            }
                            if let Some(d) = self.active_dashboard_mut() {
                                let ingest_task = d
                                    .ingest_trades(&stream, &trades, update_t, main_window_id)
                                    .map(move |msg| Message::Dashboard {
                                        layout_id: None,
                                        event: msg,
                                    });
                                tick_tasks.push(ingest_task);
                            }
                        }
                    }
                }
                reply_tx.send(reply_replay_status(self));
                if !tick_tasks.is_empty() {
                    return Task::batch(tick_tasks);
                }
            }
            replay::ReplayCommand::Toggle => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::ToggleMode));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::Play { start, end } => {
                let main_window_id = self.main_window.id;
                let Some(active_id) = self.layout_manager.active_layout_id().map(|l| l.unique)
                else {
                    reply_tx.send_status(500, r#"{"error":"no active layout"}"#.to_string());
                    return Task::none();
                };
                let Some(dashboard) = self
                    .layout_manager
                    .get_mut(active_id)
                    .map(|l| &mut l.dashboard)
                else {
                    reply_tx.send_status(500, r#"{"error":"no active dashboard"}"#.to_string());
                    return Task::none();
                };
                let (task, toast) =
                    self.replay
                        .play_with_range(start, end, dashboard, main_window_id);
                if let Some(t) = toast {
                    self.notifications.push(t);
                }
                reply_tx.send(reply_replay_status(self));
                return task.map(Message::Replay);
            }
            replay::ReplayCommand::Pause => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::Pause));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::Resume => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::Resume));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::StepForward => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::StepForward));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::StepBackward => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::StepBackward));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::CycleSpeed => {
                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::CycleSpeed));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::SaveState => {
                let empty_windows = std::collections::HashMap::new();
                self.save_state_to_disk(&empty_windows);
                reply_tx.send(reply_replay_status(self));
            }
        }
        Task::none()
    }

    /// `VirtualExchangeCommand` を処理する。`handle_replay_api` から呼ばれる。
    pub(crate) fn handle_virtual_exchange_commands(
        &mut self,
        cmd: replay_api::VirtualExchangeCommand,
        reply_tx: replay_api::ReplySender,
    ) -> Task<Message> {
        use replay::virtual_exchange::{
            PositionSide, VirtualOrder, VirtualOrderStatus, VirtualOrderType,
        };
        use replay_api::VirtualExchangeCommand;

        match cmd {
            VirtualExchangeCommand::PlaceOrder {
                ticker,
                side,
                qty,
                order_type,
                limit_price,
            } => {
                if let Some(engine) = &mut self.virtual_engine {
                    let order_side = if side == "buy" {
                        PositionSide::Long
                    } else {
                        PositionSide::Short
                    };
                    let vo_type = if order_type == "limit" {
                        let Some(price) = limit_price.filter(|p| *p > 0.0) else {
                            reply_tx.send_status(
                                400,
                                r#"{"error":"limit order requires limit_price > 0"}"#.to_string(),
                            );
                            return Task::none();
                        };
                        VirtualOrderType::Limit { price }
                    } else {
                        VirtualOrderType::Market
                    };
                    let now_ms = self.replay.current_time_ms().unwrap_or(0);
                    let vo = VirtualOrder {
                        order_id: uuid::Uuid::new_v4().to_string(),
                        ticker,
                        side: order_side,
                        qty,
                        order_type: vo_type,
                        placed_time_ms: now_ms,
                        status: VirtualOrderStatus::Pending,
                    };
                    let order_id = engine.place_order(vo);
                    reply_tx.send(
                        serde_json::json!({
                            "order_id": order_id,
                            "status": "pending"
                        })
                        .to_string(),
                    );
                } else {
                    reply_tx.send_status(
                        400,
                        r#"{"error":"REPLAY mode only. Start replay first."}"#.to_string(),
                    );
                }
            }
            VirtualExchangeCommand::GetPortfolio => {
                if let Some(engine) = &self.virtual_engine {
                    let current_price = self.replay.last_close_price().unwrap_or(0.0);
                    let snap = engine.portfolio_snapshot(current_price);
                    reply_tx.send(
                        serde_json::to_string(&snap)
                            .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
                    );
                } else {
                    reply_tx.send_status(
                        400,
                        r#"{"error":"REPLAY mode only. Start replay first."}"#.to_string(),
                    );
                }
            }
            VirtualExchangeCommand::GetState => {
                if let Some(data) = self.replay.get_api_state(50) {
                    let klines: Vec<KlineStateItem> = data
                        .klines
                        .into_iter()
                        .flat_map(|(stream, ks)| {
                            ks.into_iter().map(move |k| KlineStateItem {
                                stream: stream.clone(),
                                time: k.time,
                                open: k.open.to_f64(),
                                high: k.high.to_f64(),
                                low: k.low.to_f64(),
                                close: k.close.to_f64(),
                                volume: k.volume.total().to_f64(),
                            })
                        })
                        .collect();
                    let trades: Vec<TradeStateItem> = data
                        .trades
                        .into_iter()
                        .flat_map(|(stream, ts)| {
                            ts.into_iter().map(move |t| TradeStateItem {
                                stream: stream.clone(),
                                time: t.time,
                                price: t.price.to_f64(),
                                qty: t.qty.to_f64(),
                                is_sell: t.is_sell,
                            })
                        })
                        .collect();
                    reply_tx.send(
                        serde_json::json!({
                            "current_time_ms": data.current_time_ms,
                            "klines": klines,
                            "trades": trades,
                        })
                        .to_string(),
                    );
                } else if self.replay.is_loading() {
                    reply_tx.send_status(
                        503,
                        r#"{"error":"replay is loading, try again shortly"}"#.to_string(),
                    );
                } else {
                    reply_tx.send_status(
                        400,
                        r#"{"error":"REPLAY mode only. Start replay first."}"#.to_string(),
                    );
                }
            }
            VirtualExchangeCommand::GetOrders => {
                if let Some(engine) = &self.virtual_engine {
                    let orders = engine.get_orders();
                    reply_tx.send(serde_json::json!({ "orders": orders }).to_string());
                } else {
                    reply_tx.send_status(
                        400,
                        r#"{"error":"REPLAY mode only. Start replay first."}"#.to_string(),
                    );
                }
            }
        }
        Task::none()
    }
}
