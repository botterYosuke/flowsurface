// NOTE: ADR-0001 §2 で Play/Pause/Resume/StepForward/StepBackward/CycleSpeed variant は
// 削除済み。UI 側の `▶` / `⏭` / `⏮` は agent session API を直接叩く方式に移行中。
use crate::replay::{self, ReplayMessage, ReplayUserMessage};
use crate::replay_api;
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
                // ADR-0001 §2 自動再生機構の全廃:
                // 以前はここで `self.replay.tick(...)` による headless CI 向け
                // auto-tick 発火を行っていたが削除。Replay 進行は agent session API
                // からの明示的 step / advance / rewind-to-start に限定される。
                reply_tx.send(reply_replay_status(self));
            }
            replay::ReplayCommand::Toggle { init_range } => {
                if let Some((start, end)) = init_range {
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

                let task = self.handle_replay(ReplayMessage::User(ReplayUserMessage::ToggleMode));
                reply_tx.send(reply_replay_status(self));
                return task;
            }
            replay::ReplayCommand::SaveState => {
                let empty_windows = std::collections::HashMap::new();
                self.save_state_to_disk(&empty_windows);
                reply_tx.send(reply_replay_status(self));
            }
            replay::ReplayCommand::SetMode { mode } => {
                let target_is_replay = mode == "replay";
                let task = if self.replay.is_replay() != target_is_replay {
                    self.handle_replay(ReplayMessage::User(ReplayUserMessage::ToggleMode))
                } else {
                    Task::none()
                };
                reply_tx.send(reply_replay_status(self));
                return task;
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
                // Loading 中は last_close_price が未確定のため 503 を返す
                // (unwrap_or(0.0) で評価すると unrealized_pnl が誤る)。
                if self.replay.is_loading() {
                    reply_tx.send_status(
                        503,
                        r#"{"error":"replay is loading, try again shortly"}"#.to_string(),
                    );
                } else if let Some(engine) = &self.virtual_engine {
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
                if self.replay.is_loading() {
                    reply_tx.send_status(
                        503,
                        r#"{"error":"replay is loading, try again shortly"}"#.to_string(),
                    );
                } else if let Some(engine) = &self.virtual_engine {
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
