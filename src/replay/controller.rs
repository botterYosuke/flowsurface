use std::time::Instant;

use data::UserTimezone;
use exchange::Trade;
use exchange::adapter::StreamKind;
use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::{
    ReplayLoadEvent, ReplayMessage, ReplayMode, ReplayRangeInput, ReplaySession, ReplayState,
    ReplayStatus, ReplaySystemEvent, ReplayUserMessage, loader, min_timeframe_ms,
    parse_replay_range,
    store::{EventStore, LoadedData},
};

/// `ReplayState` をラップし、replay ロジックを `main.rs` から分離するコントローラ。
///
/// `main.rs` は公開メソッドのみを経由して状態を読み書きする。
/// 内部状態への直接アクセスは一切提供しない。
#[derive(Default)]
pub struct ReplayController {
    state: ReplayState,
}

impl From<ReplayState> for ReplayController {
    fn from(state: ReplayState) -> Self {
        Self { state }
    }
}

impl ReplayController {
    /// 永続化された設定からコントローラを復元する（アプリ起動時）。
    pub fn from_saved(
        mode: ReplayMode,
        range_start: String,
        range_end: String,
        pending_auto_play: bool,
    ) -> Self {
        Self {
            state: ReplayState {
                mode,
                range_input: ReplayRangeInput {
                    start: range_start,
                    end: range_end,
                },
                session: ReplaySession::Idle,
                pending_auto_play,
            },
        }
    }
}

// ── 公開 getter / setter（main.rs 向け） ──────────────────────────────────────

impl ReplayController {
    /// リプレイモードかどうか
    pub fn is_replay(&self) -> bool {
        self.state.is_replay()
    }

    /// 再生中かどうか
    pub fn is_playing(&self) -> bool {
        self.state.is_playing()
    }

    /// 一時停止中かどうか
    pub fn is_paused(&self) -> bool {
        self.state.is_paused()
    }

    /// ロード中（Waiting 状態）かどうか
    pub fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    /// クロックが存在するかどうか（UI の有効化判定に使用）
    pub fn has_clock(&self) -> bool {
        !matches!(self.state.session, ReplaySession::Idle)
    }

    /// 現在の仮想時刻がリプレイ終端に達しているかどうか
    pub fn is_at_end(&self) -> bool {
        matches!(&self.state.session, ReplaySession::Active { clock, .. } if clock.now_ms() >= clock.full_range().end)
    }

    /// 現在の仮想時刻（ms）を返す。セッションがアクティブでない場合は `None`。
    pub fn current_time_ms(&self) -> Option<u64> {
        match &self.state.session {
            ReplaySession::Active { clock, .. } | ReplaySession::Loading { clock, .. } => {
                Some(clock.now_ms())
            }
            ReplaySession::Idle => None,
        }
    }

    /// 現在の再生モード（永続化用）
    pub fn mode(&self) -> ReplayMode {
        self.state.mode
    }

    /// 現在の速度ラベル（"1x", "2x", etc.）
    pub fn speed_label(&self) -> String {
        self.state.speed_label()
    }

    /// 範囲入力の開始テキスト
    pub fn range_input_start(&self) -> &str {
        &self.state.range_input.start
    }

    /// 範囲入力の終了テキスト
    pub fn range_input_end(&self) -> &str {
        &self.state.range_input.end
    }

    /// auto-play フラグが立っているかどうか
    pub fn is_auto_play_pending(&self) -> bool {
        self.state.pending_auto_play
    }

    /// auto-play フラグをクリアする
    pub fn clear_pending_auto_play(&mut self) {
        self.state.pending_auto_play = false;
    }

    /// 範囲入力の開始テキストを設定する
    /// NOTE: play_with_range が一括処理するが、単独利用のために公開したまま残す。
    #[allow(dead_code)]
    pub fn set_range_start(&mut self, s: String) {
        self.state.range_input.start = s;
    }

    /// 範囲入力の終了テキストを設定する
    /// NOTE: play_with_range が一括処理するが、単独利用のために公開したまま残す。
    #[allow(dead_code)]
    pub fn set_range_end(&mut self, s: String) {
        self.state.range_input.end = s;
    }

    /// API コマンド `ReplayCommand::Play { start, end }` の処理を一括実行する。
    /// range_input を更新してから ReplayMessage::Play を処理する。
    /// `main.rs` の set_range_start + set_range_end + update の3ステップを1メソッドに集約。
    /// NOTE: `set_range_start` / `set_range_end` は引き続き単独利用可能として公開したまま残す。
    pub fn play_with_range(
        &mut self,
        start: String,
        end: String,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        self.state.range_input.start = start;
        self.state.range_input.end = end;
        self.handle_message(
            ReplayMessage::User(ReplayUserMessage::Play),
            dashboard,
            main_window_id,
        )
    }

    /// セッションが利用不可のとき呼ぶ
    pub fn on_session_unavailable(&mut self) {
        self.state.on_session_unavailable();
    }

    /// 現在の状態を API レスポンス用に変換
    pub fn to_status(&self) -> ReplayStatus {
        self.state.to_status()
    }

    /// 現在時刻の表示文字列を生成する（ヘッダー表示用）
    pub fn format_current_time(&self, timezone: UserTimezone) -> String {
        super::format_current_time(&self.state, timezone)
    }

    /// アクティブな kline ストリームを収集する（mid-replay 銘柄変更用）。
    /// `Kline` 種別のみを返す。
    pub fn active_kline_streams(&self) -> Vec<StreamKind> {
        let active_streams = match &self.state.session {
            ReplaySession::Loading { active_streams, .. }
            | ReplaySession::Active { active_streams, .. } => active_streams,
            ReplaySession::Idle => return vec![],
        };
        active_streams
            .iter()
            .filter(|s| matches!(s, StreamKind::Kline { .. }))
            .copied()
            .collect()
    }

    /// 全 active_streams をデバッグ文字列リストで返す（API 診断用）。
    pub fn active_stream_debug_labels(&self) -> Vec<String> {
        let active_streams = match &self.state.session {
            ReplaySession::Loading { active_streams, .. }
            | ReplaySession::Active { active_streams, .. } => active_streams,
            ReplaySession::Idle => return vec![],
        };
        active_streams.iter().map(|s| format!("{s:?}")).collect()
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
            ReplayMessage::User(m) => self.handle_user_message(m, dashboard, main_window_id),
            ReplayMessage::Load(e) => {
                let toast = self.handle_load_event(e, dashboard, main_window_id);
                (Task::none(), toast)
            }
            ReplayMessage::System(e) => self.handle_system_event(e, dashboard, main_window_id),
        }
    }

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
                        let range = super::compute_load_range(start_ms, end_ms, step_ms);
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
                        let range = super::compute_load_range(start_ms, end_ms, stream_step_ms);
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
                    use super::clock::StepClock;
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
                    use super::clock::StepClock;
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
                use super::clock::ClockStatus;
                let now = Instant::now();
                if let ReplaySession::Active { clock, .. } = &mut self.state.session
                    && clock.status() == ClockStatus::Paused
                {
                    clock.play(now);
                    // Playing: 既に再生中 — no-op
                }
                (Task::none(), None)
            }

            ReplayUserMessage::Pause => {
                if let ReplaySession::Active { clock, .. } = &mut self.state.session {
                    clock.pause();
                }
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
                let (prev_time, start_ms) = match &self.state.session {
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
                        (prev, clock.full_range().start)
                    }
                    _ => return (Task::none(), None),
                };
                let new_time =
                    super::compute_step_backward_target(prev_time, current_time, start_ms);

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
                let history_klines = super::pre_start_history(&klines, start_ms);
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
                let range = super::compute_load_range(start_ms, end_ms, stream_step_ms);
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

        let dispatch = super::dispatcher::dispatch_tick(clock, store, active_streams, now);

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

    /// Pause → Seek → ChartReset → KlineInject を一括実行する。
    /// StepForward/StepBackward (Playing 時)、StepBackward (Paused 時)、
    /// および handle_range_input_change から呼ぶ。
    ///
    /// # 対象外
    /// - `ReloadKlineStream`: reset_charts → ロード → 注入の順序が異なる
    /// - `StepForward` (Paused 時): pause も chart reset も不要（前進のみ）
    fn seek_to(
        &mut self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        match &mut self.state.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                clock.pause();
                clock.seek(target_ms);
            }
            ReplaySession::Idle => {}
        }
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(target_ms, dashboard, main_window_id);
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
    }

    /// `0..=target_ms` の klines を全 active_streams からチャートに注入する。
    /// StepForward / StepBackward / range_input_change で共通利用。
    ///
    /// NOTE: 範囲が `0..` から始まるのは意図的。`KlinesLoadCompleted` 時に
    /// pre-history バー（start_ms 前）も EventStore に格納されており、Seek 後に
    /// これらを含めて再注入しないとチャートに履歴バーが表示されない。
    fn inject_klines_up_to(
        &self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        let (store, active_streams) = match &self.state.session {
            ReplaySession::Loading {
                store,
                active_streams,
                ..
            }
            | ReplaySession::Active {
                store,
                active_streams,
                ..
            } => (store, active_streams),
            ReplaySession::Idle => return,
        };
        for stream in active_streams.iter() {
            let klines = store.klines_in(stream, 0..target_ms + 1);
            if !klines.is_empty() {
                dashboard.ingest_replay_klines(stream, klines, main_window_id);
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iced::window;

    use super::*;
    use crate::replay::clock::{ClockStatus, StepClock};
    use crate::screen::dashboard::Dashboard;

    /// B-3 テスト用のヘルパー定数
    const START_MS: u64 = 1_000_000;
    const END_MS: u64 = 4_000_000;
    const STEP_MS: u64 = 1_000_000;

    /// Playing 状態の `ReplayController` を生成する。
    /// clock は `start_ms` に位置し、status は `Playing`。
    fn make_playing_controller() -> ReplayController {
        let mut ctrl = ReplayController::default();
        let mut clock = StepClock::new(START_MS, END_MS, STEP_MS);
        clock.play(Instant::now());
        ctrl.state.session = ReplaySession::Active {
            clock,
            store: super::super::store::EventStore::new(),
            active_streams: std::collections::HashSet::new(),
        };
        ctrl
    }

    /// テスト用: Active セッションの clock への参照を返す。
    fn get_active_clock(ctrl: &ReplayController) -> &super::super::clock::StepClock {
        match &ctrl.state.session {
            ReplaySession::Active { clock, .. } => clock,
            _ => panic!("expected Active session"),
        }
    }

    /// テスト用: Active セッションの clock への可変参照を返す。
    fn get_active_clock_mut(ctrl: &mut ReplayController) -> &mut super::super::clock::StepClock {
        match &mut ctrl.state.session {
            ReplaySession::Active { clock, .. } => clock,
            _ => panic!("expected Active session"),
        }
    }

    // ── B-3: Playing 中に ⏮ を押したときの挙動 ────────────────────────────────

    /// Playing 中に StepBackward を押すと clock が Paused になること。
    #[test]
    fn step_backward_while_playing_pauses_clock() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepBackward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Paused,
            "StepBackward while Playing must pause the clock"
        );
    }

    /// Playing 中に StepBackward を押すと current_time が range.start に戻ること。
    #[test]
    fn step_backward_while_playing_seeks_to_range_start() {
        let mut ctrl = make_playing_controller();

        // clock を中間まで進める（2 ステップ: now_ms = 3_000_000）
        {
            let clock = get_active_clock_mut(&mut ctrl);
            let base = Instant::now();
            clock.tick(base + Duration::from_millis(200));
        }
        assert_ne!(
            ctrl.state.current_time(),
            START_MS,
            "pre-condition: clock must have advanced past start"
        );

        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepBackward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            ctrl.state.current_time(),
            START_MS,
            "StepBackward while Playing must seek current_time back to range.start"
        );
    }

    /// Playing 中に StepBackward を押しても range.end が変化しないこと。
    #[test]
    fn step_backward_while_playing_preserves_range_end() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepBackward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).full_range().end,
            END_MS,
            "StepBackward while Playing must not modify range.end"
        );
    }

    /// range.start ではない位置に Paused 状態の `ReplayController` を生成する。
    fn make_mid_range_paused_controller() -> ReplayController {
        let mut ctrl = ReplayController::default();
        let mut clock = StepClock::new(START_MS, END_MS, STEP_MS);
        // START_MS + STEP_MS (= 2_000_000) に移動して停止
        clock.seek(START_MS + STEP_MS);
        ctrl.state.session = ReplaySession::Active {
            clock,
            store: super::super::store::EventStore::new(),
            active_streams: std::collections::HashSet::new(),
        };
        ctrl
    }

    // ── StepForward while Playing ──────────────────────────────────────────────

    /// Playing 中に ⏭ を押すと clock が Paused になること。
    #[test]
    fn step_forward_while_playing_pauses_clock() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepForward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Paused,
            "StepForward while Playing must pause the clock"
        );
    }

    /// Playing 中に ⏭ を押すと current_time が range.end に移動すること。
    #[test]
    fn step_forward_while_playing_seeks_to_range_end() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepForward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            ctrl.state.current_time(),
            END_MS,
            "StepForward while Playing must seek current_time to range.end"
        );
    }

    /// Playing 中に ⏭ を押しても range.end が変化しないこと。
    #[test]
    fn step_forward_while_playing_preserves_range_end() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StepForward),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).full_range().end,
            END_MS,
            "StepForward while Playing must not modify range.end"
        );
    }

    // ── CycleSpeed ────────────────────────────────────────────────────────────

    /// Playing 中に CycleSpeed を押しても ClockStatus が変わらないこと（Playing のまま）。
    #[test]
    fn cycle_speed_while_playing_keeps_status() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::CycleSpeed),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Playing,
            "CycleSpeed must not change ClockStatus — speed only"
        );
    }

    /// Playing 中に CycleSpeed を押しても current_time が変化しないこと。
    #[test]
    fn cycle_speed_while_playing_does_not_seek() {
        let mut ctrl = make_playing_controller();

        // clock を中間まで進める
        {
            let clock = get_active_clock_mut(&mut ctrl);
            let base = Instant::now();
            clock.tick(base + Duration::from_millis(200));
        }
        let time_before = ctrl.state.current_time();

        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::CycleSpeed),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            ctrl.state.current_time(),
            time_before,
            "CycleSpeed must not seek — current_time unchanged"
        );
    }

    /// Paused 中に CycleSpeed を押しても current_time が変化しないこと。
    #[test]
    fn cycle_speed_while_paused_does_not_seek() {
        let mut ctrl = make_mid_range_paused_controller();
        let time_before = ctrl.state.current_time();

        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::CycleSpeed),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            ctrl.state.current_time(),
            time_before,
            "CycleSpeed while Paused must not seek"
        );
    }

    // ── StartTimeChanged / EndTimeChanged while clock active ──────────────────

    /// Playing 中に StartTimeChanged を受けると clock が Paused になること。
    #[test]
    fn start_time_changed_while_playing_pauses_clock() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StartTimeChanged(
                "2025-01-01 00:00".to_string(),
            )),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Paused,
            "StartTimeChanged while Playing must pause the clock"
        );
    }

    /// Playing 中に StartTimeChanged を受けると current_time が range.start に戻ること。
    #[test]
    fn start_time_changed_while_playing_seeks_to_range_start() {
        let mut ctrl = make_playing_controller();

        // clock を中間まで進める
        {
            let clock = get_active_clock_mut(&mut ctrl);
            let base = Instant::now();
            clock.tick(base + Duration::from_millis(200));
        }

        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::StartTimeChanged(
                "2025-01-01 00:00".to_string(),
            )),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            ctrl.state.current_time(),
            START_MS,
            "StartTimeChanged while Playing must seek current_time back to range.start"
        );
    }

    /// Playing 中に EndTimeChanged を受けると clock が Paused になること。
    #[test]
    fn end_time_changed_while_playing_pauses_clock() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::EndTimeChanged(
                "2025-12-31 00:00".to_string(),
            )),
            &mut dashboard,
            main_window,
        );

        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Paused,
            "EndTimeChanged while Playing must pause the clock"
        );
    }

    // ── P2: play_with_range ───────────────────────────────────────────────────

    /// play_with_range を呼ぶと range_input が更新されること
    #[test]
    fn play_with_range_updates_range_input() {
        let mut ctrl = ReplayController::default();
        ctrl.state.mode = ReplayMode::Replay;
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();

        let _ = ctrl.play_with_range(
            "2025-01-01 00:00".to_string(),
            "2025-01-02 00:00".to_string(),
            &mut dashboard,
            win,
        );

        assert_eq!(ctrl.state.range_input.start, "2025-01-01 00:00");
        assert_eq!(ctrl.state.range_input.end, "2025-01-02 00:00");
    }

    /// play_with_range の結果が set_range_start + set_range_end + handle_message(Play) と等価
    #[test]
    fn play_with_range_equivalent_to_set_then_play() {
        let make = || {
            let mut ctrl = ReplayController::default();
            ctrl.state.mode = ReplayMode::Replay;
            ctrl
        };

        let mut ctrl_combined = make();
        let mut ctrl_separate = make();
        let mut dash1 = Dashboard::default();
        let mut dash2 = Dashboard::default();
        let win = window::Id::unique();

        let _ = ctrl_combined.play_with_range(
            "2025-01-01 00:00".to_string(),
            "2025-01-02 00:00".to_string(),
            &mut dash1,
            win,
        );

        ctrl_separate.set_range_start("2025-01-01 00:00".to_string());
        ctrl_separate.set_range_end("2025-01-02 00:00".to_string());
        let _ = ctrl_separate.handle_message(
            ReplayMessage::User(ReplayUserMessage::Play),
            &mut dash2,
            win,
        );

        assert_eq!(
            ctrl_combined.state.range_input.start,
            ctrl_separate.state.range_input.start,
        );
        assert_eq!(
            ctrl_combined.state.range_input.end,
            ctrl_separate.state.range_input.end,
        );
    }

    // ── P1: seek_to ───────────────────────────────────────────────────────────

    /// seek_to を呼ぶと clock が Paused になること
    #[test]
    fn seek_to_pauses_clock() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        ctrl.seek_to(END_MS, &mut dashboard, win);
        assert_eq!(
            get_active_clock(&ctrl).status(),
            ClockStatus::Paused,
            "seek_to must pause the clock"
        );
    }

    /// seek_to を呼ぶと now_ms が target_ms にスナップされること
    #[test]
    fn seek_to_positions_clock_at_target() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        ctrl.seek_to(END_MS, &mut dashboard, win);
        assert_eq!(ctrl.state.current_time(), END_MS);
    }

    /// seek_to で range.start を渡したとき now_ms が start になること
    #[test]
    fn seek_to_range_start_resets_position() {
        let mut ctrl = make_playing_controller();
        {
            get_active_clock_mut(&mut ctrl).seek(START_MS + STEP_MS);
        }
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        ctrl.seek_to(START_MS, &mut dashboard, win);
        assert_eq!(ctrl.state.current_time(), START_MS);
    }

    // ── P3: ReplaySession State Machine ──────────────────────────────────────

    /// Play を送ると kline なし → 即 Active に遷移すること
    #[test]
    fn session_is_active_after_play_with_no_klines() {
        let mut ctrl = ReplayController::default();
        ctrl.state.mode = ReplayMode::Replay;
        ctrl.state.range_input.start = "2025-01-01 00:00".to_string();
        ctrl.state.range_input.end = "2025-01-02 00:00".to_string();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::User(ReplayUserMessage::Play),
            &mut dashboard,
            win,
        );

        assert!(
            matches!(ctrl.state.session, ReplaySession::Active { .. }),
            "kline なし → 即 Active に遷移するはず"
        );
    }

    /// DataLoadFailed を受けると session が Idle になること
    #[test]
    fn session_transitions_to_idle_on_data_load_failed() {
        use std::collections::HashSet;
        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Loading {
            clock: super::super::clock::StepClock::new(1_000_000, 4_000_000, 60_000),
            pending_count: 2,
            store: super::super::store::EventStore::new(),
            active_streams: HashSet::new(),
        };
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();

        let _ = ctrl.handle_message(
            ReplayMessage::Load(ReplayLoadEvent::DataLoadFailed("timeout".to_string())),
            &mut dashboard,
            win,
        );

        assert!(
            matches!(ctrl.state.session, ReplaySession::Idle),
            "DataLoadFailed → Idle に遷移するはず"
        );
    }

    /// Loading で pending_count=1 のとき KlinesLoadCompleted を受けると Active になること
    #[test]
    fn session_transitions_loading_to_active_when_last_stream_loaded() {
        use std::collections::HashSet;
        let stream = crate::replay::testutil::kline_stream();
        let mut active = HashSet::new();
        active.insert(stream);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Loading {
            clock: super::super::clock::StepClock::new(1_000_000, 4_000_000, 60_000),
            pending_count: 1,
            store: super::super::store::EventStore::new(),
            active_streams: active,
        };
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();

        let range = 1_000_000..4_000_000;
        let _ = ctrl.handle_message(
            ReplayMessage::Load(ReplayLoadEvent::KlinesLoadCompleted(stream, range, vec![])),
            &mut dashboard,
            win,
        );

        assert!(
            matches!(ctrl.state.session, ReplaySession::Active { .. }),
            "pending_count=1 → Active に遷移するはず"
        );
    }

    /// Idle 状態では is_loading / is_playing / is_paused / has_clock がすべて false
    #[test]
    fn session_idle_all_status_false() {
        let ctrl = ReplayController::default();
        assert!(!ctrl.is_loading());
        assert!(!ctrl.is_playing());
        assert!(!ctrl.is_paused());
        assert!(!ctrl.has_clock());
    }

    // ── P4: handle_load_event ─────────────────────────────────────────────────

    /// KlinesLoadCompleted を handle_load_event で処理すると Toast が返らないこと
    #[test]
    fn load_event_completed_returns_no_toast() {
        use std::collections::HashSet;
        let stream = crate::replay::testutil::kline_stream();
        let mut active = HashSet::new();
        active.insert(stream);
        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Loading {
            clock: StepClock::new(1_000_000, 4_000_000, 60_000),
            pending_count: 1,
            store: super::super::store::EventStore::new(),
            active_streams: active,
        };
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        let toast = ctrl.handle_load_event(
            ReplayLoadEvent::KlinesLoadCompleted(stream, 1_000_000..4_000_000, vec![]),
            &mut dashboard,
            win,
        );
        assert!(toast.is_none());
    }

    /// DataLoadFailed を handle_load_event で処理すると Toast が返ること
    #[test]
    fn load_event_failed_returns_error_toast() {
        let mut ctrl = ReplayController::default();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        let toast = ctrl.handle_load_event(
            ReplayLoadEvent::DataLoadFailed("connection refused".to_string()),
            &mut dashboard,
            win,
        );
        assert!(toast.is_some());
    }

    /// handle_load_event の戻り値型が Option<Toast> であること（型レベル検証）
    #[test]
    fn load_event_handler_signature_returns_option_toast() {
        let mut ctrl = ReplayController::default();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        let _: Option<crate::widget::toast::Toast> = ctrl.handle_load_event(
            ReplayLoadEvent::DataLoadFailed("err".to_string()),
            &mut dashboard,
            win,
        );
    }
}
