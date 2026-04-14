use std::time::Instant;

use exchange::Trade;
use exchange::adapter::StreamKind;
use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::{ReplayMessage, ReplayState, loader, min_timeframe_ms, parse_replay_range};

/// `ReplayState` をラップし、replay ロジックを `main.rs` から分離するコントローラ。
///
/// `Deref<Target = ReplayState>` を実装するため、既存の `replay.is_replay()` 等の
/// 読み取りメソッドはそのままコンパイルできる。状態変化・副作用を伴う処理は
/// [`ReplayController::handle_message`] と [`ReplayController::tick`] に集約する。
#[derive(Default)]
pub struct ReplayController {
    pub state: ReplayState,
}

impl std::ops::Deref for ReplayController {
    type Target = ReplayState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for ReplayController {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}


impl From<ReplayState> for ReplayController {
    fn from(state: ReplayState) -> Self {
        Self { state }
    }
}

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
    /// `ReplayMessage` を処理し、必要な非同期 Task と通知 Toast を返す。
    ///
    /// `dashboard` には `main.rs` 側で `layout_manager` から取り出した `&mut Dashboard`
    /// を渡す。これにより `self.replay` と `self.layout_manager` の同時可変借用が成立する。
    pub fn handle_message(
        &mut self,
        msg: ReplayMessage,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        match msg {
            ReplayMessage::ToggleMode => {
                let was_replay = self.state.is_replay();
                self.state.toggle_mode();
                if was_replay && !self.state.is_replay() {
                    // Replay → Live: ペイン content を再構築して WS を自動復帰させる
                    dashboard.rebuild_for_live(main_window_id);
                }
                (Task::none(), None)
            }

            ReplayMessage::StartTimeChanged(s) => {
                self.state.range_input.start = s;
                (Task::none(), None)
            }

            ReplayMessage::EndTimeChanged(s) => {
                self.state.range_input.end = s;
                (Task::none(), None)
            }

            ReplayMessage::Play => {
                self.state.on_manual_play_requested();

                let (start_ms, end_ms) = match parse_replay_range(
                    &self.state.range_input.start,
                    &self.state.range_input.end,
                ) {
                    Ok(range) => range,
                    Err(e) => {
                        return (Task::none(), Some(Toast::error(format!("Replay: {e}"))));
                    }
                };

                // ペイン content をクリアし、kline ストリームを収集
                let kline_targets = dashboard.prepare_replay(main_window_id);

                // 最小 timeframe で StepClock を初期化
                let step_size_ms = kline_targets
                    .iter()
                    .filter_map(|(_, s)| s.as_kline_stream())
                    .map(|(_, tf)| tf.to_milliseconds())
                    .min()
                    .unwrap_or(min_timeframe_ms(&Default::default()));

                self.state.start(start_ms, end_ms, step_size_ms);

                // active_streams に登録（Kline stream のみ — Trade/Depth は除外）
                for (_, stream) in &kline_targets {
                    if matches!(stream, StreamKind::Kline { .. }) {
                        self.state.active_streams.insert(*stream);
                    }
                }

                // 各 kline ストリームに対して load_klines を発行
                let kline_tasks: Vec<Task<ReplayMessage>> = kline_targets
                    .into_iter()
                    .map(|(_, stream)| {
                        let range = super::compute_load_range(start_ms, end_ms, step_size_ms);
                        Task::perform(
                            loader::load_klines(stream, range),
                            |result| match result {
                                Ok(r) => ReplayMessage::KlinesLoadCompleted(
                                    r.stream, r.range, r.klines,
                                ),
                                Err(e) => ReplayMessage::DataLoadFailed(e),
                            },
                        )
                    })
                    .collect();

                if kline_tasks.is_empty() {
                    // kline chart 無し: 即座に Playing へ
                    self.state.resume_from_waiting(Instant::now());
                    (Task::none(), None)
                } else {
                    (Task::batch(kline_tasks), None)
                }
            }

            ReplayMessage::KlinesLoadCompleted(stream, range, klines) => {
                // (D) 空 klines は "未ロード" と同義 — EventStore に登録しない
                if klines.is_empty() {
                    return (Task::none(), None);
                }

                let now = Instant::now();
                self.state.on_klines_loaded(stream, range, klines.clone(), now);

                // Start 時刻より前のバーのみを注入する（pre_start_history バー）。
                // Start 以降のバーは dispatch_tick が逐次注入するため、ここで注入すると
                // dedup で無視されてバーが増えなくなる。
                let start_ms = self.state.clock
                    .as_ref()
                    .map(|c| c.full_range().start)
                    .unwrap_or(0);
                let history_klines = super::pre_start_history(&klines, start_ms);
                if !history_klines.is_empty() {
                    dashboard.ingest_replay_klines(&stream, &history_klines, main_window_id);
                }
                (Task::none(), None)
            }

            ReplayMessage::Resume => {
                let now = Instant::now();
                if let Some(clock) = &mut self.state.clock {
                    use super::clock::ClockStatus;
                    if clock.status() == ClockStatus::Paused {
                        clock.play(now);
                    }
                    // Waiting: ロード完了時に try_resume_from_waiting が自動で Playing に移行
                    // Playing: 既に再生中 — no-op
                }
                (Task::none(), None)
            }

            ReplayMessage::Pause => {
                if let Some(clock) = &mut self.state.clock {
                    clock.pause();
                }
                (Task::none(), None)
            }

            ReplayMessage::StepForward => {
                // Playing 中は tick が自動で進める — StepForward は Paused 時のみ有効
                if !self.state.is_paused() {
                    return (Task::none(), None);
                }

                let current_time = self.state.current_time();
                let step_size = min_timeframe_ms(&self.state.active_streams);
                let new_time = current_time + step_size;

                if let Some(clock) = &mut self.state.clock {
                    let range_end = clock.full_range().end;
                    if new_time > range_end {
                        return (Task::none(), None); // 範囲終端を超える — no-op
                    }
                    clock.seek(new_time);
                }

                // 新時刻までの klines をチャートに注入
                self.inject_klines_up_to(new_time, dashboard, main_window_id);
                (Task::none(), None)
            }

            ReplayMessage::CycleSpeed => {
                self.state.cycle_speed();
                (Task::none(), None)
            }

            ReplayMessage::StepBackward => {
                let current_time = self.state.current_time();

                // 全アクティブ stream の前の kline 時刻の最大値
                let prev_time = self
                    .state
                    .active_streams
                    .iter()
                    .filter_map(|stream| {
                        let klines =
                            self.state.event_store.klines_in(stream, 0..current_time);
                        klines.iter().rev().find(|k| k.time < current_time).map(|k| k.time)
                    })
                    .max();

                let start_ms = self.state.clock
                    .as_ref()
                    .map(|c| c.full_range().start)
                    .unwrap_or(0);
                let new_time = super::compute_step_backward_target(prev_time, current_time, start_ms);

                if let Some(clock) = &mut self.state.clock {
                    clock.seek(new_time);
                    clock.pause();
                }

                // ビューポートを保持したままデータのみリセット（KlineChart 再構築なし）
                dashboard.reset_charts_for_seek(main_window_id);
                self.inject_klines_up_to(new_time, dashboard, main_window_id);
                (Task::none(), None)
            }

            ReplayMessage::DataLoadFailed(err) => {
                self.state.clock = None;
                (
                    Task::none(),
                    Some(Toast::error(format!("Replay data load failed: {err}"))),
                )
            }

            ReplayMessage::SyncReplayBuffers => {
                // mid-replay でペイン構成が変わった場合に step_size を再計算する
                if let Some(clock) = &mut self.state.clock {
                    let step_size_ms = min_timeframe_ms(&self.state.active_streams);
                    clock.set_step_size(step_size_ms);
                }
                (Task::none(), None)
            }

            ReplayMessage::ReloadKlineStream { old_stream, new_stream } => {
                let Some(clock) = &mut self.state.clock else {
                    return (Task::none(), None);
                };

                // 旧 stream を active_streams から除去し、新 stream を登録
                if let Some(old) = old_stream {
                    self.state.active_streams.remove(&old);
                }
                self.state.active_streams.insert(new_stream);

                // step_size を新 active_streams の最小 timeframe に更新
                let step_size_ms = min_timeframe_ms(&self.state.active_streams);
                let start_ms = clock.full_range().start;
                let end_ms = clock.full_range().end;

                // クロックをリセットして先頭に戻し、データロード待ちへ
                clock.set_step_size(step_size_ms);
                clock.seek(start_ms);
                clock.set_waiting();

                // 新 stream の klines を再ロード
                let range = super::compute_load_range(start_ms, end_ms, step_size_ms);
                let task = Task::perform(
                    loader::load_klines(new_stream, range),
                    |result| match result {
                        Ok(r) => ReplayMessage::KlinesLoadCompleted(r.stream, r.range, r.klines),
                        Err(e) => ReplayMessage::DataLoadFailed(e),
                    },
                );
                (task, None)
            }
        }
    }

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
        let Some(clock) = &mut self.state.clock else {
            return TickOutcome {
                trade_events: vec![],
                reached_end: false,
            };
        };

        let dispatch = super::dispatcher::dispatch_tick(
            clock,
            &self.state.event_store,
            &self.state.active_streams,
            now,
        );

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

    /// A-1: `start_ms..=target_ms` の klines を全 active_streams からチャートに注入する。
    /// StepForward / StepBackward の重複コードを統一。
    fn inject_klines_up_to(
        &self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        for stream in self.state.active_streams.iter() {
            let klines = self.state.event_store.klines_in(stream, 0..target_ms + 1);
            if !klines.is_empty() {
                dashboard.ingest_replay_klines(stream, klines, main_window_id);
            }
        }
    }
}
