use std::time::Instant;

use exchange::Trade;
use exchange::adapter::StreamKind;
use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::super::{
    ReplayLoadEvent, ReplayMessage, ReplaySession, ReplaySystemEvent, loader, min_timeframe_ms,
};
use super::ReplayController;

/// `ReplayController::tick` の戻り値。
/// kline 注入はコントローラ内で完結するが、trade 注入には `Task` が必要なため
/// 呼び出し側 (main.rs) で処理できるよう trade イベントを返す。
pub struct TickOutcome {
    /// (stream, trades, update_t) のリスト。空でないものだけ含まれる。
    pub trade_events: Vec<(StreamKind, Vec<Trade>, u64)>,
    /// true なら replay 終端に到達した（clock は Paused 済み）。
    pub reached_end: bool,
}

impl ReplayController {
    /// Tick ごとに `dispatch_tick` を実行し、kline をチャートに直接注入する。
    /// Trade 注入には `Task` が必要なため [`TickOutcome`] として返す。
    ///
    /// C-2: `reached_end == true` のとき呼び出し側で Toast を発行する。
    pub fn tick(
        &mut self,
        now: Instant,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> TickOutcome {
        let (clock, store, active_streams) = match &mut self.state.session {
            ReplaySession::Loading {
                clock,
                store,
                active_streams,
                ..
            }
            | ReplaySession::Active {
                clock,
                store,
                active_streams,
                ..
            } => (clock, store, active_streams),
            ReplaySession::Idle => {
                return TickOutcome {
                    trade_events: vec![],
                    reached_end: false,
                };
            }
        };

        let dispatch = super::super::dispatcher::dispatch_tick(clock, store, active_streams, now);

        // kline をチャートへ同期注入
        for (stream, klines) in &dispatch.kline_events {
            if !klines.is_empty() {
                dashboard.ingest_replay_klines(stream, klines, main_window_id);
            }
        }

        // trade イベントは Task が必要なため呼び出し側に委ねる
        let trade_events = dispatch
            .trade_events
            .into_iter()
            .filter(|(_, trades)| !trades.is_empty())
            .map(|(stream, trades)| {
                let update_t = trades.last().map_or(dispatch.current_time, |t| t.time);
                (stream, trades, update_t)
            })
            .collect();

        TickOutcome {
            trade_events,
            reached_end: dispatch.reached_end,
        }
    }

    /// システムイベントを処理する。
    pub fn handle_system_event(
        &mut self,
        event: ReplaySystemEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        match event {
            ReplaySystemEvent::SyncReplayBuffers => {
                // mid-replay でペイン構成が変わった場合に step_size を再計算する
                match &mut self.state.session {
                    ReplaySession::Loading {
                        clock,
                        active_streams,
                        ..
                    }
                    | ReplaySession::Active {
                        clock,
                        active_streams,
                        ..
                    } => {
                        let step_size_ms = min_timeframe_ms(active_streams);
                        clock.set_step_size(step_size_ms);
                    }
                    ReplaySession::Idle => {}
                }
                (Task::none(), None)
            }

            ReplaySystemEvent::ReloadKlineStream {
                old_stream,
                new_stream,
            } => {
                // Active のみ対応（Idle/Loading 時は no-op）

                // Step 1: ミュータブルボローで更新値を計算
                let (start_ms, end_ms, stream_step_ms) = {
                    let ReplaySession::Active {
                        clock,
                        active_streams,
                        ..
                    } = &mut self.state.session
                    else {
                        return (Task::none(), None);
                    };

                    clock.pause();

                    if let Some(old) = old_stream {
                        active_streams.remove(&old);
                    }
                    active_streams.insert(new_stream);

                    let step_size_ms = min_timeframe_ms(active_streams);
                    let start_ms = clock.full_range().start;
                    let end_ms = clock.full_range().end;

                    clock.set_step_size(step_size_ms);
                    clock.seek(start_ms);

                    let stream_step_ms = new_stream
                        .as_kline_stream()
                        .map(|(_, tf)| tf.to_milliseconds())
                        .unwrap_or(step_size_ms);
                    (start_ms, end_ms, stream_step_ms)
                };

                // チャートの表示をクリアして新しいデータ受信に備える
                dashboard.reset_charts_for_seek(main_window_id);

                // Step 2: Active → Loading に遷移（ボロー解放後）
                let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
                if let ReplaySession::Active {
                    clock,
                    store,
                    active_streams,
                } = old
                {
                    self.state.session = ReplaySession::Loading {
                        clock,
                        pending_count: 1,
                        store,
                        active_streams,
                    };
                }

                // 新 stream の klines を再ロード
                let range = super::super::compute_load_range(start_ms, end_ms, stream_step_ms);
                let task =
                    Task::perform(
                        loader::load_klines(new_stream, range),
                        |result| match result {
                            Ok(r) => ReplayMessage::Load(ReplayLoadEvent::KlinesLoadCompleted(
                                r.stream, r.range, r.klines,
                            )),
                            Err(e) => ReplayMessage::Load(ReplayLoadEvent::DataLoadFailed(e)),
                        },
                    );
                (task, None)
            }
        }
    }

    /// StepForward 後に仮想エンジンへ渡す合成トレードを生成する。
    ///
    /// 現在の clock 位置に対応する kline の close 価格で 1 ティック分の Trade を合成する。
    /// Trades EventStore が常に空のため（`ingest_loaded` が `trades: vec![]`）、
    /// step-forward 時に成行注文を約定させるための代替手段として使用する。
    pub fn synthetic_trades_at_current_time(&self) -> Vec<(StreamKind, Vec<Trade>)> {
        let (clock, store, active_streams) = match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
                ..
            } => (clock, store, active_streams),
            _ => return vec![],
        };
        let current_time = clock.now_ms();
        active_streams
            .iter()
            .filter(|s| matches!(s, StreamKind::Kline { .. }))
            .filter_map(|stream| {
                // current_time 以下の最新 kline を取得
                let klines = store.klines_in(stream, 0..current_time + 1);
                let kline = klines.iter().rev().find(|k| k.time <= current_time)?;
                let trade = Trade {
                    time: current_time,
                    is_sell: false,
                    price: kline.close,
                    qty: exchange::unit::qty::Qty::from_f32(1.0),
                };
                Some((*stream, vec![trade]))
            })
            .collect()
    }
}
