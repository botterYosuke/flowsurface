use std::time::Instant;

use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::super::{
    ReplayLoadEvent, ReplayMessage, ReplaySession, ReplayUserMessage, loader, min_timeframe_ms,
    parse_replay_range,
    store::{EventStore, LoadedData},
};
use super::ReplayController;

impl ReplayController {
    /// UI 操作を処理する。非同期タスクを起動する可能性がある（Play 時に kline ロードタスクを発行）。
    pub fn handle_user_message(
        &mut self,
        msg: ReplayUserMessage,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        match msg {
            ReplayUserMessage::ToggleMode => {
                let was_replay = self.state.is_replay();
                self.state.toggle_mode();
                if was_replay && !self.state.is_replay() {
                    // Replay → Live: ペイン content を再構築して WS を自動復帰させる
                    dashboard.rebuild_for_live(main_window_id);
                } else if !was_replay && self.state.is_replay() {
                    // Live → Replay: replay_mode=true で再構築してフェッチループを抑制する
                    dashboard.clear_chart_for_replay(main_window_id);
                }
                (Task::none(), None)
            }

            ReplayUserMessage::StartTimeChanged(s) => {
                self.state.range_input.start = s;
                self.handle_range_input_change(dashboard, main_window_id);
                (Task::none(), None)
            }

            ReplayUserMessage::EndTimeChanged(s) => {
                self.state.range_input.end = s;
                self.handle_range_input_change(dashboard, main_window_id);
                (Task::none(), None)
            }

            ReplayUserMessage::Play => {
                self.state.resume_pending = false;

                let (start_ms, end_ms) = match parse_replay_range(
                    &self.state.range_input.start,
                    &self.state.range_input.end,
                ) {
                    Ok(range) => range,
                    Err(e) => {
                        return (Task::none(), Some(Toast::error(format!("Replay: {e}"))));
                    }
                };

                // 大きすぎる範囲の早期検出（チャートクリア前に検証）
                {
                    const MAX_LOAD_PAGES: u64 = 100;
                    const PAGE_SIZE: u64 = 1000;
                    let preview = dashboard.peek_kline_streams(main_window_id);
                    for (_, stream) in preview.iter() {
                        let Some((_, tf)) = stream.as_kline_stream() else {
                            continue;
                        };
                        let step_ms = tf.to_milliseconds().max(1);
                        let range = super::super::compute_load_range(start_ms, end_ms, step_ms);
                        let estimated_klines = range.end.saturating_sub(range.start) / step_ms;
                        let estimated_pages = estimated_klines.div_ceil(PAGE_SIZE);
                        if estimated_pages > MAX_LOAD_PAGES {
                            return (
                                Task::none(),
                                Some(Toast::error(format!(
                                    "Replay range too large: ~{estimated_pages} API pages for \
                                     {tf:?} chart. Max ~{MAX_LOAD_PAGES} pages \
                                     (~{} bars). Please shorten the range.",
                                    MAX_LOAD_PAGES * PAGE_SIZE
                                ))),
                            );
                        }
                    }
                }

                // ペイン content をクリアし、kline ストリームを収集
                let kline_targets = dashboard.prepare_replay(main_window_id);

                // 最小 timeframe で StepClock を初期化
                let step_size_ms = kline_targets
                    .iter()
                    .filter_map(|(_, s)| s.as_kline_stream())
                    .map(|(_, tf)| tf.to_milliseconds())
                    .min()
                    .unwrap_or(min_timeframe_ms(&Default::default()));

                // active_streams 収集（Kline stream のみ — Trade/Depth は除外）
                use exchange::adapter::StreamKind;
                use std::collections::HashSet;
                let active_streams: HashSet<StreamKind> = kline_targets
                    .iter()
                    .filter_map(|(_, s)| {
                        if matches!(s, StreamKind::Kline { .. }) {
                            Some(*s)
                        } else {
                            None
                        }
                    })
                    .collect();
                let pending_count = active_streams.len();

                // 各 kline ストリームに対して load_klines を発行
                // ストリーム固有の timeframe で pre-history window を計算する（D1 と 1m が混在する場合、
                // min_timeframe_ms だと D1 バーの timestamp がウィンドウ外になる）。
                let kline_tasks: Vec<Task<ReplayMessage>> = kline_targets
                    .into_iter()
                    .filter_map(|(_, stream)| {
                        if !matches!(stream, StreamKind::Kline { .. }) {
                            return None;
                        }
                        let stream_step_ms = stream
                            .as_kline_stream()
                            .map(|(_, tf)| tf.to_milliseconds())
                            .unwrap_or(step_size_ms);
                        let range =
                            super::super::compute_load_range(start_ms, end_ms, stream_step_ms);
                        Some(Task::perform(
                            loader::load_klines(stream, range),
                            |result| match result {
                                Ok(r) => ReplayMessage::Load(ReplayLoadEvent::KlinesLoadCompleted(
                                    r.stream, r.range, r.klines,
                                )),
                                Err(e) => ReplayMessage::Load(ReplayLoadEvent::DataLoadFailed(e)),
                            },
                        ))
                    })
                    .collect();

                // 既存セッションの速度設定を引き継ぐ（リセット時に 1x に戻らないよう）
                let previous_speed = match &self.state.session {
                    ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                        clock.speed()
                    }
                    ReplaySession::Idle => 1.0,
                };

                if pending_count == 0 {
                    // kline chart 無し: 即座に Playing へ
                    use super::super::clock::StepClock;
                    let mut clock = StepClock::new(start_ms, end_ms, step_size_ms);
                    clock.set_speed(previous_speed);
                    clock.play(Instant::now());
                    self.state.session = ReplaySession::Active {
                        clock,
                        store: EventStore::new(),
                        active_streams,
                    };
                    (Task::none(), None)
                } else {
                    use super::super::clock::StepClock;
                    let mut clock = StepClock::new(start_ms, end_ms, step_size_ms);
                    clock.set_speed(previous_speed);
                    clock.set_waiting();
                    self.state.session = ReplaySession::Loading {
                        clock,
                        pending_count,
                        store: EventStore::new(),
                        active_streams,
                    };
                    (Task::batch(kline_tasks), None)
                }
            }

            ReplayUserMessage::Resume => {
                use super::super::clock::ClockStatus;
                let now = Instant::now();
                match &mut self.state.session {
                    ReplaySession::Active { clock, .. }
                        if clock.status() == ClockStatus::Paused =>
                    {
                        clock.play(now);
                    }
                    ReplaySession::Loading { .. } => {
                        // データロード完了後に自動再開するようフラグを立てる。
                        // ticker/timeframe 変更直後にユーザーが Resume を呼んだが、
                        // まだ klines が届いていない場合に有効。
                        self.state.resume_pending = true;
                    }
                    _ => {}
                }
                (Task::none(), None)
            }

            ReplayUserMessage::Pause => {
                if let ReplaySession::Active { clock, .. } = &mut self.state.session {
                    clock.pause();
                }
                self.state.resume_pending = false;
                (Task::none(), None)
            }

            ReplayUserMessage::StepForward => {
                let step_size = match &self.state.session {
                    ReplaySession::Loading { active_streams, .. }
                    | ReplaySession::Active { active_streams, .. } => {
                        min_timeframe_ms(active_streams)
                    }
                    ReplaySession::Idle => return (Task::none(), None),
                };

                if self.state.is_playing() {
                    // Playing 中: End まで一気に進めて停止
                    let end_ms = match &self.state.session {
                        ReplaySession::Active { clock, .. } => clock.full_range().end,
                        _ => 0,
                    };
                    self.seek_to(end_ms, dashboard, main_window_id);
                    return (Task::none(), None);
                }

                // Paused 時のみ位置を 1 bar 前進する
                if !self.state.is_paused() {
                    return (Task::none(), None);
                }

                let current_time = self.state.current_time();
                let new_time = current_time + step_size;

                if let ReplaySession::Active { clock, .. } = &mut self.state.session {
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

            ReplayUserMessage::CycleSpeed => {
                // 速度変更のみ。シークやリセットは行わない（Playing 中は即時反映される）。
                self.state.cycle_speed();
                (Task::none(), None)
            }

            ReplayUserMessage::StepBackward => {
                if self.state.is_playing() {
                    // Playing 中: 停止して start に戻す
                    let start_ms = match &self.state.session {
                        ReplaySession::Active { clock, .. } => clock.full_range().start,
                        _ => 0,
                    };
                    self.seek_to(start_ms, dashboard, main_window_id);
                    return (Task::none(), None);
                }

                // Paused 時: 1 bar 前の位置へシーク
                let current_time = self.state.current_time();

                // 全アクティブ stream の前の kline 時刻の最大値
                let (prev_time, start_ms, step_size) = match &self.state.session {
                    ReplaySession::Active {
                        clock,
                        store,
                        active_streams,
                        ..
                    } => {
                        let prev = active_streams
                            .iter()
                            .filter_map(|stream| {
                                let klines = store.klines_in(stream, 0..current_time);
                                klines
                                    .iter()
                                    .rev()
                                    .find(|k| k.time < current_time)
                                    .map(|k| k.time)
                            })
                            .max();
                        let step = super::super::min_timeframe_ms(active_streams);
                        (prev, clock.full_range().start, step)
                    }
                    _ => return (Task::none(), None),
                };
                let new_time = super::super::compute_step_backward_target(
                    prev_time,
                    current_time,
                    start_ms,
                    step_size,
                );

                // ビューポートを保持したままデータのみリセット（KlineChart 再構築なし）
                self.seek_to(new_time, dashboard, main_window_id);
                (Task::none(), None)
            }
        }
    }

    /// 非同期ロードイベントを処理する。
    /// KlinesLoadCompleted も DataLoadFailed もタスクを起動しないため、
    /// Task を返す必要がない。これを型で表現する。
    pub fn handle_load_event(
        &mut self,
        event: ReplayLoadEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Option<Toast> {
        match event {
            ReplayLoadEvent::KlinesLoadCompleted(stream, range, klines) => {
                // 空 klines でも EventStore に登録してストリームをロード済みとマークする。
                // klines が空 = データなし（市場休場・範囲外）であり「ロード未完了」ではない。
                // Idle なら DataLoadFailed 後の遅延 KlinesLoadCompleted → サイレントドロップ。

                // Step 1: ミュータブルボローで内部を更新し、遷移すべきかを bool で返す
                let should_activate = if let ReplaySession::Loading {
                    pending_count,
                    store,
                    clock,
                    ..
                } = &mut self.state.session
                {
                    store.ingest_loaded(
                        stream,
                        range,
                        LoadedData {
                            klines: klines.clone(),
                            trades: vec![],
                        },
                    );
                    *pending_count = pending_count.saturating_sub(1);
                    if *pending_count == 0 {
                        clock.resume_from_waiting(Instant::now());
                        true
                    } else {
                        false
                    }
                } else {
                    // Idle: DataLoadFailed 後の遅延 KlinesLoadCompleted → 無視
                    false
                };

                // Step 2: ボローが解放されてから mem::replace で Loading → Active に遷移
                if should_activate {
                    let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
                    if let ReplaySession::Loading {
                        clock,
                        store,
                        active_streams,
                        ..
                    } = old
                    {
                        self.state.session = ReplaySession::Active {
                            clock,
                            store,
                            active_streams,
                        };
                    }

                    // Loading 中に Resume が呼ばれていた場合、Active 遷移後に再開する。
                    if self.state.resume_pending {
                        self.state.resume_pending = false;
                        if let ReplaySession::Active { clock, .. } = &mut self.state.session {
                            use super::super::clock::ClockStatus;
                            if clock.status() == ClockStatus::Paused {
                                clock.play(Instant::now());
                            }
                        }
                    }
                }

                // Start 時刻より前のバーのみを注入する（pre_start_history バー）。
                // Start 以降のバーは dispatch_tick が逐次注入するため、ここで注入すると
                // dedup で無視されてバーが増えなくなる。
                let start_ms = match &self.state.session {
                    ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                        clock.full_range().start
                    }
                    ReplaySession::Idle => 0,
                };
                let history_klines = super::super::pre_start_history(&klines, start_ms);
                if !history_klines.is_empty() {
                    dashboard.ingest_replay_klines(&stream, &history_klines, main_window_id);
                }
                None
            }

            ReplayLoadEvent::DataLoadFailed(err) => {
                // session をリセットして残留状態を除去する。
                // これがないと次回 Play 時に古いデータが混入する可能性がある。
                self.reset_session();
                Some(Toast::error(format!("Replay data load failed: {err}")))
            }
        }
    }

    /// `StartTimeChanged` / `EndTimeChanged` で共通化された範囲変更後処理。
    /// clock が存在する場合は先頭に戻して停止し、チャートを先頭時点にリセットする。
    fn handle_range_input_change(
        &mut self,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        let start_ms = match &self.state.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                Some(clock.full_range().start)
            }
            ReplaySession::Idle => None,
        };
        if let Some(start_ms) = start_ms {
            self.seek_to(start_ms, dashboard, main_window_id);
        }
    }

    /// session を Idle にリセットする。
    /// `DataLoadFailed` 時に呼ぶことで次回 Play 時に残留状態が混入しないようにする。
    fn reset_session(&mut self) {
        self.state.session = ReplaySession::Idle;
        self.state.resume_pending = false;
    }
}
