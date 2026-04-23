use data::UserTimezone;
use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::{
    ReplayMessage, ReplayMode, ReplayRangeInput, ReplaySession, ReplayState, ReplayStatus,
};

mod agent;
pub mod api;
mod session;
mod tick;

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
    ///
    /// ADR-0001 §8: 起動時 fixture 自動 Play は廃止済み。Replay モードで復元しても
    /// session は Idle のまま保持する。
    pub fn from_saved(mode: ReplayMode, range_start: String, range_end: String) -> Self {
        Self {
            state: ReplayState {
                mode,
                range_input: ReplayRangeInput {
                    start: range_start,
                    end: range_end,
                },
                session: ReplaySession::Idle,
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

    /// ロード中かどうか
    pub fn is_loading(&self) -> bool {
        self.state.is_loading()
    }

    /// クロックが存在するかどうか（UI の有効化判定に使用）
    pub fn has_clock(&self) -> bool {
        !matches!(self.state.session, ReplaySession::Idle)
    }

    /// セッションがアクティブ（再生可能）かどうか
    pub fn is_active(&self) -> bool {
        matches!(self.state.session, ReplaySession::Active { .. })
    }

    /// 現在の再生モード（永続化用）
    pub fn mode(&self) -> ReplayMode {
        self.state.mode
    }

    /// 範囲入力の開始テキスト
    pub fn range_input_start(&self) -> &str {
        &self.state.range_input.start
    }

    /// 範囲入力の終了テキスト
    pub fn range_input_end(&self) -> &str {
        &self.state.range_input.end
    }

    #[cfg(test)]
    pub fn set_range_start(&mut self, s: String) {
        self.state.range_input.start = s;
    }

    #[cfg(test)]
    pub fn set_range_end(&mut self, s: String) {
        self.state.range_input.end = s;
    }

    /// 現在の状態を API レスポンス用に変換
    pub fn to_status(&self) -> ReplayStatus {
        self.state.to_status()
    }

    /// 現在時刻の表示文字列を生成する（ヘッダー表示用）
    pub fn format_current_time(&self, timezone: UserTimezone) -> String {
        super::format_current_time(&self.state, timezone)
    }

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
}

// ── seek ヘルパー（session.rs / tests から呼ばれる） ──────────────────────────

impl ReplayController {
    /// Pause → Seek → ChartReset → KlineInject を一括実行する。
    /// handle_range_input_change 等から呼ぶ。
    ///
    /// # 対象外
    /// - `ReloadKlineStream`: reset_charts → ロード → 注入の順序が異なる
    fn seek_to(
        &mut self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        match &mut self.state.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                clock.seek(target_ms);
            }
            ReplaySession::Idle => {}
        }
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(target_ms, dashboard, main_window_id);
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
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iced::window;

    use super::*;
    use crate::replay::clock::StepClock;
    use crate::replay::{ReplayLoadEvent, ReplaySession, ReplayUserMessage};
    use crate::screen::dashboard::Dashboard;

    const START_MS: u64 = 1_000_000;
    const END_MS: u64 = 4_000_000;
    const STEP_MS: u64 = 1_000_000;

    fn make_playing_controller() -> ReplayController {
        let mut ctrl = ReplayController::default();
        let clock = StepClock::new(START_MS, END_MS, STEP_MS);
        ctrl.state.session = ReplaySession::Active {
            clock,
            store: crate::replay::store::EventStore::new(),
            active_streams: std::collections::HashSet::new(),
        };
        ctrl
    }

    fn get_active_clock(ctrl: &ReplayController) -> &crate::replay::clock::StepClock {
        match &ctrl.state.session {
            ReplaySession::Active { clock, .. } => clock,
            _ => panic!("expected Active session"),
        }
    }

    fn get_active_clock_mut(ctrl: &mut ReplayController) -> &mut crate::replay::clock::StepClock {
        match &mut ctrl.state.session {
            ReplaySession::Active { clock, .. } => clock,
            _ => panic!("expected Active session"),
        }
    }

    fn make_mid_range_paused_controller() -> ReplayController {
        let mut ctrl = ReplayController::default();
        let mut clock = StepClock::new(START_MS, END_MS, STEP_MS);
        clock.seek(START_MS + STEP_MS);
        ctrl.state.session = ReplaySession::Active {
            clock,
            store: crate::replay::store::EventStore::new(),
            active_streams: std::collections::HashSet::new(),
        };
        ctrl
    }

    // ── P2: play_with_range removed ───────────────────────────────────────────

    // ── P1: seek_to ───────────────────────────────────────────────────────────
    #[test]
    fn seek_to_positions_clock_at_target() {
        let mut ctrl = make_playing_controller();
        let mut dashboard = Dashboard::default();
        let win = window::Id::unique();
        ctrl.seek_to(END_MS, &mut dashboard, win);
        assert_eq!(ctrl.state.current_time(), END_MS);
    }

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

    #[test]
    fn session_transitions_to_idle_on_data_load_failed() {
        use std::collections::HashSet;
        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Loading {
            clock: crate::replay::clock::StepClock::new(1_000_000, 4_000_000, 60_000),
            pending_count: 2,
            store: crate::replay::store::EventStore::new(),
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

    #[test]
    fn session_transitions_loading_to_active_when_last_stream_loaded() {
        use std::collections::HashSet;
        let stream = crate::replay::testutil::kline_stream();
        let mut active = HashSet::new();
        active.insert(stream);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Loading {
            clock: crate::replay::clock::StepClock::new(1_000_000, 4_000_000, 60_000),
            pending_count: 1,
            store: crate::replay::store::EventStore::new(),
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

    #[test]
    fn session_idle_all_status_false() {
        let ctrl = ReplayController::default();
        assert!(!ctrl.is_loading());
        assert!(!ctrl.has_clock());
    }

    // ── P4: handle_load_event ─────────────────────────────────────────────────

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
            store: crate::replay::store::EventStore::new(),
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

    // ── get_api_state ──────────────────────────────────────────────────────

    #[test]
    fn get_api_state_returns_none_when_idle() {
        let ctrl = ReplayController::default();
        assert!(ctrl.get_api_state(50).is_none());
    }

    #[test]
    fn get_api_state_returns_none_when_loading() {
        use crate::replay::store::EventStore;
        use std::collections::HashSet;

        let mut ctrl = ReplayController::default();
        let clock = StepClock::new(0, 100_000, 60_000);
        ctrl.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams: HashSet::new(),
        };
        assert!(ctrl.get_api_state(50).is_none());
    }

    #[test]
    fn get_api_state_returns_current_time_when_active_empty_store() {
        use crate::replay::store::EventStore;
        use std::collections::HashSet;

        let mut ctrl = ReplayController::default();
        let clock = StepClock::new(180_000, 3_600_000, 60_000);
        ctrl.state.session = ReplaySession::Active {
            clock,
            store: EventStore::new(),
            active_streams: HashSet::new(),
        };
        let data = ctrl
            .get_api_state(50)
            .expect("should return Some when Active");
        assert_eq!(data.current_time_ms, 180_000);
        assert!(data.klines.is_empty());
        assert!(data.trades.is_empty());
    }

    #[test]
    fn get_api_state_returns_klines_and_trades_from_active_store() {
        use crate::replay::store::{EventStore, LoadedData};
        use crate::replay::testutil::{dummy_kline, dummy_trade, kline_stream, trade_stream};
        use std::collections::HashSet;

        let kline_s = kline_stream();
        let trade_s = trade_stream();

        let mut store = EventStore::new();
        store.ingest_loaded(
            kline_s,
            0..200_000,
            LoadedData {
                klines: vec![dummy_kline(60_000), dummy_kline(120_000)],
                trades: vec![],
            },
        );
        store.ingest_loaded(
            trade_s,
            0..200_000,
            LoadedData {
                klines: vec![],
                trades: vec![dummy_trade(100_000), dummy_trade(150_000)],
            },
        );

        let clock = StepClock::new(180_000, 3_600_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(kline_s);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };

        let data = ctrl.get_api_state(50).expect("should return Some");
        assert_eq!(data.current_time_ms, 180_000);
        assert_eq!(data.klines.len(), 1, "one kline stream");
        assert_eq!(data.klines[0].1.len(), 2, "2 klines in store");
        assert_eq!(data.trades.len(), 1, "one trade stream");
        assert_eq!(data.trades[0].1.len(), 2, "2 trades in store");
    }

    #[test]
    fn get_api_state_limits_klines_to_n_most_recent() {
        use crate::replay::store::{EventStore, LoadedData};
        use crate::replay::testutil::{dummy_kline, kline_stream};
        use std::collections::HashSet;

        let kline_s = kline_stream();
        let klines: Vec<_> = (1u64..=60).map(|i| dummy_kline(i * 60_000)).collect();

        let mut store = EventStore::new();
        store.ingest_loaded(
            kline_s,
            0..4_000_000,
            LoadedData {
                klines,
                trades: vec![],
            },
        );

        let clock = StepClock::new(3_600_000, 7_200_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(kline_s);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };

        let data = ctrl.get_api_state(3).expect("should return Some");
        let returned = &data.klines[0].1;
        assert_eq!(returned.len(), 3, "limit=3 must cap klines");
        assert_eq!(returned[0].time, 58 * 60_000, "first of last 3");
        assert_eq!(returned[2].time, 60 * 60_000, "last of last 3");
    }

    #[test]
    fn get_api_state_stream_label_format() {
        use crate::replay::store::{EventStore, LoadedData};
        use crate::replay::testutil::{dummy_kline, kline_stream};
        use std::collections::HashSet;

        let kline_s = kline_stream();
        let mut store = EventStore::new();
        store.ingest_loaded(
            kline_s,
            0..200_000,
            LoadedData {
                klines: vec![dummy_kline(60_000)],
                trades: vec![],
            },
        );

        let clock = StepClock::new(180_000, 3_600_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(kline_s);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };

        let data = ctrl.get_api_state(50).expect("should return Some");
        assert_eq!(data.klines[0].0, "BinanceLinear:BTCUSDT:1m");
    }

    #[test]
    fn get_api_state_limits_trades_to_n_most_recent() {
        use crate::replay::store::{EventStore, LoadedData};
        use crate::replay::testutil::{dummy_trade, kline_stream, trade_stream};
        use std::collections::HashSet;

        let kline_s = kline_stream();
        let trade_s = trade_stream();
        let trades: Vec<_> = (1u64..=10)
            .map(|i| dummy_trade(600_000 - i * 10_000))
            .collect();

        let mut store = EventStore::new();
        store.ingest_loaded(
            trade_s,
            0..4_000_000,
            LoadedData {
                klines: vec![],
                trades,
            },
        );

        let clock = StepClock::new(600_000, 7_200_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(kline_s);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };

        let data = ctrl.get_api_state(3).expect("should return Some");
        let returned = &data.trades[0].1;
        assert_eq!(returned.len(), 3, "limit=3 must cap trades");
    }

    #[test]
    fn get_api_state_excludes_trade_outside_window() {
        use crate::replay::store::{EventStore, LoadedData};
        use crate::replay::testutil::{dummy_trade, kline_stream, trade_stream};
        use std::collections::HashSet;

        let kline_s = kline_stream();
        let trade_s = trade_stream();
        let now = 600_000u64;
        let inside = dummy_trade(now - api::TRADE_WINDOW_MS);
        let outside = dummy_trade(now - api::TRADE_WINDOW_MS - 1);

        let mut store = EventStore::new();
        store.ingest_loaded(
            trade_s,
            0..4_000_000,
            LoadedData {
                klines: vec![],
                trades: vec![outside, inside],
            },
        );

        let clock = StepClock::new(now, 7_200_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(kline_s);

        let mut ctrl = ReplayController::default();
        ctrl.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };

        let data = ctrl.get_api_state(50).expect("should return Some");
        let returned = &data.trades[0].1;
        assert_eq!(
            returned.len(),
            1,
            "only the inside-window trade should appear"
        );
        assert_eq!(returned[0].time, inside.time);
    }
}
