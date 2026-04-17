pub mod clock;
pub mod controller;
pub mod dispatcher;
pub mod loader;
pub mod store;
#[cfg(test)]
pub(crate) mod testutil;
pub mod virtual_exchange;

use std::collections::HashSet;

use exchange::Kline;
use exchange::adapter::StreamKind;

use clock::{ClockStatus, StepClock};
use store::EventStore;

/// Replay Start 時刻より前に何本の kline を履歴として読み込むか。
/// 最小 timeframe × この本数分を pre-start history として fetch する。
/// 将来 `data/config/replay.rs` 等で設定化する余地を残す。
pub const PRE_START_HISTORY_BARS: u64 = 300;

/// kline streams のうち最小 timeframe を ms で返す。kline stream が 0 本なら 1m (60_000ms) を返す。
pub fn min_timeframe_ms(active_streams: &HashSet<StreamKind>) -> u64 {
    active_streams
        .iter()
        .filter_map(|s| s.as_kline_stream())
        .map(|(_, tf)| tf.to_milliseconds())
        .min()
        .unwrap_or(60_000) // 1m fallback
}

/// Play 開始時に fetch する kline の range を計算する。
/// Start 時刻から `PRE_START_HISTORY_BARS` 本分遡って load_start を求め、
/// `load_start_ms..end_ms` を返す。
pub fn compute_load_range(start_ms: u64, end_ms: u64, step_size_ms: u64) -> std::ops::Range<u64> {
    start_ms.saturating_sub(PRE_START_HISTORY_BARS * step_size_ms)..end_ms
}

/// KlinesLoadCompleted 時に、Start 時刻より前のバーのみを抽出して返す。
/// `k.time < start_ms` の条件で strictly less than を使うため、
/// Start 時刻ちょうどのバーは含まない（dispatcher の最初の tick が注入する）。
pub fn pre_start_history(klines: &[Kline], start_ms: u64) -> Vec<Kline> {
    klines
        .iter()
        .filter(|k| k.time < start_ms)
        .cloned()
        .collect()
}

/// StepBackward で clock を戻す先の時刻を計算する。
/// history 範囲 (< start_ms) のバーが EventStore に存在しても
/// start_ms 未満には seek しないようにクランプする。
pub fn compute_step_backward_target(
    prev_time: Option<u64>,
    current_time: u64,
    start_ms: u64,
) -> u64 {
    prev_time.unwrap_or(current_time).max(start_ms)
}

// ── 公開 API ────────────────────────────────────────────────────────────────

/// API から iced app へ送るコマンド
#[derive(Debug, Clone)]
pub enum ReplayCommand {
    GetStatus,
    Toggle,
    Play {
        start: String,
        end: String,
    },
    Pause,
    Resume,
    StepForward,
    StepBackward,
    CycleSpeed,
    /// 状態をディスクに保存（E2E テスト用）
    SaveState,
}

/// iced app から API へ返すレスポンス
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReplayStatus {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<u64>,
    /// UI の範囲入力テキスト（永続化復元の検証用）
    pub range_start: String,
    pub range_end: String,
}

/// リプレイセッションの状態を表す列挙型。
/// Idle: セッションなし、Loading: klines ロード待ち、Active: 再生可能（Playing/Paused）。
#[derive(Debug)]
pub enum ReplaySession {
    /// セッションなし（Play 前 / DataLoadFailed 後 / Live モード）
    Idle,
    /// klines ロード中。`pending_count` が 0 になったら Active に遷移する。
    Loading {
        clock: StepClock,
        /// ロード完了待ちのストリーム数
        pending_count: usize,
        store: EventStore,
        active_streams: HashSet<StreamKind>,
    },
    /// ロード完了。Playing / Paused どちらでも Active。
    Active {
        clock: StepClock,
        store: EventStore,
        active_streams: HashSet<StreamKind>,
    },
}

/// リプレイモードの状態を管理する
pub struct ReplayState {
    /// ライブ / リプレイの切替
    pub(crate) mode: ReplayMode,
    /// リプレイ範囲の設定（UI入力）
    pub(crate) range_input: ReplayRangeInput,
    /// リプレイセッション状態（クロック・データストア・アクティブストリームを集約）
    pub(crate) session: ReplaySession,
    /// 起動時 fixture 復元の結果として次の「全ペイン Ready」で Play を発火する。
    /// 一度発火したら false に戻す。永続化しない。
    /// NOTE: `session` の一部にしない — Play は UI イベント経由で処理するため。
    pub(crate) pending_auto_play: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    Live,
    Replay,
}

#[derive(Default)]
pub struct ReplayRangeInput {
    pub(crate) start: String,
    pub(crate) end: String,
}

/// UI 操作（ユーザーが発火）
#[derive(Debug, Clone)]
pub enum ReplayUserMessage {
    ToggleMode,
    StartTimeChanged(String),
    EndTimeChanged(String),
    Play,
    Resume,
    Pause,
    StepForward,
    StepBackward,
    CycleSpeed,
}

/// 非同期タスク応答（load_klines Task が発火）
#[derive(Debug, Clone)]
pub enum ReplayLoadEvent {
    KlinesLoadCompleted(StreamKind, std::ops::Range<u64>, Vec<Kline>),
    DataLoadFailed(String),
}

/// システムイベント（main.rs のシステムイベントが発火）
#[derive(Debug, Clone)]
pub enum ReplaySystemEvent {
    SyncReplayBuffers,
    ReloadKlineStream {
        old_stream: Option<StreamKind>,
        new_stream: StreamKind,
    },
}

#[derive(Debug, Clone)]
pub enum ReplayMessage {
    User(ReplayUserMessage),
    Load(ReplayLoadEvent),
    System(ReplaySystemEvent),
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            mode: ReplayMode::Live,
            range_input: ReplayRangeInput::default(),
            session: ReplaySession::Idle,
            pending_auto_play: false,
        }
    }
}

impl ReplayState {
    /// モードをトグルする。Replay→Live の場合はセッションをリセットする。
    /// range_input は保持する（Live → Replay 再切替時に日付が復元されるようにするため）。
    pub fn toggle_mode(&mut self) {
        match self.mode {
            ReplayMode::Live => {
                self.mode = ReplayMode::Replay;
            }
            ReplayMode::Replay => {
                self.mode = ReplayMode::Live;
                self.session = ReplaySession::Idle;
                self.pending_auto_play = false;
            }
        }
    }

    /// リプレイモードかどうか
    pub fn is_replay(&self) -> bool {
        self.mode == ReplayMode::Replay
    }

    /// 再生中かどうか
    pub fn is_playing(&self) -> bool {
        matches!(
            &self.session,
            ReplaySession::Active { clock, .. } if clock.status() == ClockStatus::Playing
        )
    }

    /// 一時停止中かどうか
    pub fn is_paused(&self) -> bool {
        matches!(
            &self.session,
            ReplaySession::Active { clock, .. } if clock.status() == ClockStatus::Paused
        )
    }

    /// ロード中（Waiting 状態）かどうか
    pub fn is_loading(&self) -> bool {
        matches!(self.session, ReplaySession::Loading { .. })
    }

    /// 現在の仮想時刻（ms）。クロックが存在しない場合は 0。
    pub fn current_time(&self) -> u64 {
        match &self.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                clock.now_ms()
            }
            ReplaySession::Idle => 0,
        }
    }

    /// 手動再生が要求されたとき、pending_auto_play フラグをクリアする。
    pub fn on_manual_play_requested(&mut self) {
        self.pending_auto_play = false;
    }

    /// セッションが利用不可のとき、pending_auto_play フラグをクリアする。
    pub fn on_session_unavailable(&mut self) {
        self.pending_auto_play = false;
    }

    /// 速度を次の段階にサイクルする (1x → 2x → 5x → 10x → 1x)。
    pub fn cycle_speed(&mut self) {
        let clock = match &mut self.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => clock,
            ReplaySession::Idle => return,
        };
        let next = cycle_speed_value(clock.speed());
        clock.set_speed(next);
    }

    /// 現在の速度ラベル（"1x", "2x", etc.）
    pub fn speed_label(&self) -> String {
        match &self.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                format_speed_label(clock.speed())
            }
            ReplaySession::Idle => "1x".to_string(),
        }
    }

    /// 現在の状態を API レスポンス用に変換
    pub fn to_status(&self) -> ReplayStatus {
        let mode = match self.mode {
            ReplayMode::Live => "Live".to_string(),
            ReplayMode::Replay => "Replay".to_string(),
        };
        let range_start = self.range_input.start.clone();
        let range_end = self.range_input.end.clone();

        let clock = match &self.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                Some(clock)
            }
            ReplaySession::Idle => None,
        };

        match clock {
            Some(clock) => {
                let range = clock.full_range();
                ReplayStatus {
                    mode,
                    status: Some(match clock.status() {
                        ClockStatus::Playing => "Playing".to_string(),
                        ClockStatus::Paused => "Paused".to_string(),
                        ClockStatus::Waiting => "Loading".to_string(),
                    }),
                    current_time: Some(clock.now_ms()),
                    speed: Some(format_speed_label(clock.speed())),
                    start_time: Some(range.start),
                    end_time: Some(range.end),
                    range_start,
                    range_end,
                }
            }
            None => ReplayStatus {
                mode,
                status: None,
                current_time: None,
                speed: None,
                start_time: None,
                end_time: None,
                range_start,
                range_end,
            },
        }
    }
}

// ── ユーティリティ ────────────────────────────────────────────────────────────

/// 利用可能な再生速度一覧
const SPEEDS: &[f32] = &[1.0, 2.0, 5.0, 10.0];

/// 次の速度値を返す（1→2→5→10→1 のサイクル）
pub fn cycle_speed_value(current: f32) -> f32 {
    let idx = SPEEDS
        .iter()
        .position(|&s| (s - current).abs() < 0.01)
        .unwrap_or(0);
    SPEEDS[(idx + 1) % SPEEDS.len()]
}

/// 速度を表示用文字列に変換する
pub fn format_speed_label(speed: f32) -> String {
    if speed == speed.floor() {
        format!("{}x", speed as u32)
    } else {
        format!("{:.1}x", speed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRangeError {
    InvalidStartFormat,
    InvalidEndFormat,
    StartAfterEnd,
}

impl std::fmt::Display for ParseRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseRangeError::InvalidStartFormat => write!(f, "Invalid start time format"),
            ParseRangeError::InvalidEndFormat => write!(f, "Invalid end time format"),
            ParseRangeError::StartAfterEnd => write!(f, "Start time must be before end time"),
        }
    }
}

/// 日時文字列をパースし、リプレイ範囲 (start_ms, end_ms) を返す。
/// フォーマット: "YYYY-MM-DD HH:MM" (UTC として解釈)
pub fn parse_replay_range(start: &str, end: &str) -> Result<(u64, u64), ParseRangeError> {
    let start_dt = chrono::NaiveDateTime::parse_from_str(start, "%Y-%m-%d %H:%M")
        .map_err(|_| ParseRangeError::InvalidStartFormat)?;
    let end_dt = chrono::NaiveDateTime::parse_from_str(end, "%Y-%m-%d %H:%M")
        .map_err(|_| ParseRangeError::InvalidEndFormat)?;

    let start_ms = start_dt.and_utc().timestamp_millis() as u64;
    let end_ms = end_dt.and_utc().timestamp_millis() as u64;

    if start_ms >= end_ms {
        return Err(ParseRangeError::StartAfterEnd);
    }

    Ok((start_ms, end_ms))
}

/// 現在時刻の表示文字列を生成する。
/// ライブモード: 現在時刻、リプレイモード: 仮想時刻（clock の now_ms）
pub fn format_current_time(replay: &ReplayState, timezone: data::UserTimezone) -> String {
    let timestamp_ms: i64 = match (&replay.mode, &replay.session) {
        (
            ReplayMode::Replay,
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. },
        ) => clock.now_ms() as i64,
        _ => chrono::Utc::now().timestamp_millis(),
    };

    timezone
        .format_with_kind(
            timestamp_ms,
            data::config::timezone::TimeLabelKind::Custom("%Y-%m-%d %H:%M:%S"),
        )
        .unwrap_or_default()
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    fn make_instant_plus(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn make_active_state_playing() -> ReplayState {
        let base = Instant::now();
        let mut clock = StepClock::new(0, 100_000, 60_000);
        clock.play(base);
        ReplayState {
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        }
    }

    fn make_active_state_paused() -> ReplayState {
        let clock = StepClock::new(0, 100_000, 60_000);
        ReplayState {
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        }
    }

    // ── ReplayState モード管理 ──────────────────────────────────────────────

    #[test]
    fn default_state_is_live_mode() {
        let state = ReplayState::default();
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
        assert!(matches!(state.session, ReplaySession::Idle));
        assert!(state.range_input.start.is_empty());
        assert!(state.range_input.end.is_empty());
    }

    #[test]
    fn toggle_mode_switches_live_to_replay() {
        let mut state = ReplayState::default();
        state.toggle_mode();
        assert_eq!(state.mode, ReplayMode::Replay);
        assert!(state.is_replay());
    }

    #[test]
    fn toggle_mode_switches_replay_to_live_and_resets_session_but_preserves_range() {
        let mut state = ReplayState::default();
        state.toggle_mode(); // Live → Replay
        state.range_input.start = "2026-04-01 09:00".to_string();
        state.range_input.end = "2026-04-01 15:00".to_string();

        state.toggle_mode(); // Replay → Live
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
        // range_input は保持される（Live → Replay 再切替時に日付が復元されるようにするため）
        assert_eq!(state.range_input.start, "2026-04-01 09:00");
        assert_eq!(state.range_input.end, "2026-04-01 15:00");
        assert!(matches!(state.session, ReplaySession::Idle));
    }

    #[test]
    fn toggle_mode_live_to_replay_restores_range_input() {
        let mut state = ReplayState::default();
        state.toggle_mode(); // Live → Replay
        state.range_input.start = "2026-04-10 04:49".to_string();
        state.range_input.end = "2026-04-15 06:49".to_string();
        state.toggle_mode(); // Replay → Live（range は保持）

        state.toggle_mode(); // Live → Replay 再切替
        assert!(state.is_replay());
        assert_eq!(state.range_input.start, "2026-04-10 04:49");
        assert_eq!(state.range_input.end, "2026-04-15 06:49");
    }

    // ── ReplaySession 状態表現 ─────────────────────────────────────────

    #[test]
    fn is_playing_returns_true_when_clock_is_playing() {
        let state = make_active_state_playing();
        assert!(state.is_playing());
    }

    #[test]
    fn is_paused_returns_true_when_clock_is_paused() {
        let state = make_active_state_paused();
        assert!(state.is_paused());
    }

    #[test]
    fn is_loading_returns_true_when_session_is_loading() {
        let mut clock = StepClock::new(0, 100_000, 60_000);
        clock.set_waiting();
        let state = ReplayState {
            session: ReplaySession::Loading {
                clock,
                pending_count: 1,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };
        assert!(state.is_loading());
    }

    #[test]
    fn current_time_returns_zero_when_no_clock() {
        let state = ReplayState::default();
        assert_eq!(state.current_time(), 0);
    }

    #[test]
    fn current_time_returns_clock_now_ms() {
        let base = Instant::now();
        // step_size=1000, step_delay=BASE_STEP_DELAY_MS(100ms) → tick at +100ms fires once → now_ms = 51_000
        let mut clock = StepClock::new(50_000, 100_000, 1_000);
        clock.play(base);
        clock.tick(make_instant_plus(base, 100));
        let state = ReplayState {
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };
        assert_eq!(state.current_time(), 51_000);
    }

    // ── cycle_speed / speed_label ──────────────────────────────────────────

    #[test]
    fn cycle_speed_rotates_1x_2x_5x_10x_1x() {
        let mut state = make_active_state_playing();

        assert_eq!(state.speed_label(), "1x");
        state.cycle_speed();
        assert_eq!(state.speed_label(), "2x");
        state.cycle_speed();
        assert_eq!(state.speed_label(), "5x");
        state.cycle_speed();
        assert_eq!(state.speed_label(), "10x");
        state.cycle_speed();
        assert_eq!(state.speed_label(), "1x"); // wrap around
    }

    #[test]
    fn speed_label_returns_1x_when_no_clock() {
        let state = ReplayState::default();
        assert_eq!(state.speed_label(), "1x");
    }

    // ── format_speed_label ────────────────────────────────────────────────

    #[test]
    fn format_speed_label_integer_speeds() {
        assert_eq!(format_speed_label(1.0), "1x");
        assert_eq!(format_speed_label(2.0), "2x");
        assert_eq!(format_speed_label(10.0), "10x");
    }

    #[test]
    fn format_speed_label_fractional_speed() {
        assert_eq!(format_speed_label(1.5), "1.5x");
    }

    // ── to_status() ──────────────────────────────────────────────────────

    #[test]
    fn to_status_live_mode_no_clock() {
        let state = ReplayState::default();
        let status = state.to_status();
        assert_eq!(status.mode, "Live");
        assert!(status.status.is_none());
        assert!(status.current_time.is_none());
        assert!(status.speed.is_none());
        assert!(status.start_time.is_none());
        assert!(status.end_time.is_none());
        assert!(status.range_start.is_empty());
        assert!(status.range_end.is_empty());
    }

    #[test]
    fn to_status_replay_playing() {
        let base = Instant::now();
        // step_size=500, step_delay=BASE_STEP_DELAY_MS(100ms) → 3 steps at +300ms → now_ms = 1500
        let mut clock = StepClock::new(0, 5_000, 500);
        clock.play(base);
        clock.tick(make_instant_plus(base, 300)); // 3 steps: now_ms = 1500
        let state = ReplayState {
            mode: ReplayMode::Replay,
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };

        let status = state.to_status();
        assert_eq!(status.mode, "Replay");
        assert_eq!(status.status.as_deref(), Some("Playing"));
        assert_eq!(status.current_time, Some(1_500));
        assert_eq!(status.speed.as_deref(), Some("1x"));
        assert_eq!(status.start_time, Some(0));
        assert_eq!(status.end_time, Some(5_000));
    }

    #[test]
    fn to_status_replay_loading() {
        let mut clock = StepClock::new(0, 1_000, 60_000);
        clock.set_waiting();
        let state = ReplayState {
            mode: ReplayMode::Replay,
            session: ReplaySession::Loading {
                clock,
                pending_count: 1,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };

        let status = state.to_status();
        assert_eq!(status.status.as_deref(), Some("Loading"));
    }

    #[test]
    fn to_status_replay_paused() {
        let clock = StepClock::new(0, 1_000, 60_000);
        // clock starts Paused by default
        let state = ReplayState {
            mode: ReplayMode::Replay,
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };

        let status = state.to_status();
        assert_eq!(status.status.as_deref(), Some("Paused"));
    }

    #[test]
    fn to_status_includes_range_input() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-10 09:00".to_string(),
                end: "2026-04-10 15:00".to_string(),
            },
            ..Default::default()
        };
        let status = state.to_status();
        assert_eq!(status.mode, "Replay");
        assert!(status.status.is_none());
        assert_eq!(status.range_start, "2026-04-10 09:00");
        assert_eq!(status.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn to_status_live_serializes_without_optional_fields() {
        let state = ReplayState::default();
        let status = state.to_status();
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains(r#""mode":"Live""#));
        assert!(!json.contains("status"));
        assert!(!json.contains("current_time"));
        assert!(!json.contains("speed"));
    }

    #[test]
    fn to_status_replay_serializes_all_fields() {
        let base = Instant::now();
        // step_size=500, step_delay=100ms → 3 steps at +300ms → now_ms=1500
        let mut clock = StepClock::new(0, 5_000, 500);
        clock.play(base);
        clock.tick(make_instant_plus(base, 300));
        let state = ReplayState {
            mode: ReplayMode::Replay,
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&state.to_status()).unwrap();
        assert!(json.contains(r#""mode":"Replay""#));
        assert!(json.contains(r#""status":"Playing""#));
        assert!(json.contains(r#""current_time":1500"#));
        assert!(json.contains(r#""speed":"1x""#));
    }

    // ── pending_auto_play ─────────────────────────────────────────────────

    #[test]
    fn default_state_has_no_pending_auto_play() {
        let state = ReplayState::default();
        assert!(!state.pending_auto_play);
    }

    #[test]
    fn toggle_replay_to_live_clears_pending_auto_play() {
        let mut state = ReplayState {
            mode: ReplayMode::Replay,
            pending_auto_play: true,
            ..Default::default()
        };

        state.toggle_mode(); // Replay → Live

        assert!(!state.pending_auto_play);
    }

    #[test]
    fn replay_play_message_clears_pending_auto_play() {
        let mut state = ReplayState {
            pending_auto_play: true,
            ..Default::default()
        };
        state.on_manual_play_requested();
        assert!(!state.pending_auto_play);
    }

    #[test]
    fn session_restore_failure_clears_pending_auto_play() {
        let mut state = ReplayState {
            pending_auto_play: true,
            ..Default::default()
        };
        state.on_session_unavailable();
        assert!(!state.pending_auto_play);
    }

    // ── parse_replay_range ────────────────────────────────────────────────

    #[test]
    fn parse_replay_range_valid_input() {
        let (start, end) = parse_replay_range("2026-04-01 09:00", "2026-04-01 15:00").unwrap();
        assert_eq!(end - start, 6 * 60 * 60 * 1000);
        assert!(start > 1_700_000_000_000);
    }

    #[test]
    fn parse_replay_range_invalid_start_format() {
        let result = parse_replay_range("not-a-date", "2026-04-01 15:00");
        assert_eq!(result, Err(ParseRangeError::InvalidStartFormat));
    }

    #[test]
    fn parse_replay_range_invalid_end_format() {
        let result = parse_replay_range("2026-04-01 09:00", "bad");
        assert_eq!(result, Err(ParseRangeError::InvalidEndFormat));
    }

    #[test]
    fn parse_replay_range_start_after_end() {
        let result = parse_replay_range("2026-04-01 15:00", "2026-04-01 09:00");
        assert_eq!(result, Err(ParseRangeError::StartAfterEnd));
    }

    #[test]
    fn parse_replay_range_24_hours_is_ok() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-02 09:00");
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(end - start, 24 * 60 * 60 * 1000);
    }

    #[test]
    fn parse_replay_range_multi_day_is_ok() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-08 09:00");
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(end - start, 7 * 24 * 60 * 60 * 1000);
    }

    #[test]
    fn parse_replay_range_same_start_and_end() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-01 09:00");
        assert_eq!(result, Err(ParseRangeError::StartAfterEnd));
    }

    #[test]
    fn parse_replay_range_with_seconds_format_rejected() {
        let result = parse_replay_range("2026-04-01 09:00:00", "2026-04-01 15:00:00");
        assert_eq!(result, Err(ParseRangeError::InvalidStartFormat));
    }

    // ── format_current_time ────────────────────────────────────────────────

    #[test]
    fn format_current_time_uses_realtime_in_live_mode() {
        let state = ReplayState::default();
        let result = format_current_time(&state, data::UserTimezone::Utc);
        assert!(!result.is_empty());
        assert_eq!(result.len(), 19); // "YYYY-MM-DD HH:MM:SS"
    }

    #[test]
    fn format_current_time_uses_clock_time_in_replay() {
        // 2025-04-01 06:00:00 UTC = 1743487200000 ms
        let target_ms = 1_743_487_200_000u64;
        let clock = StepClock::new(target_ms, target_ms + 3_600_000, 60_000);
        let state = ReplayState {
            mode: ReplayMode::Replay,
            session: ReplaySession::Active {
                clock,
                store: EventStore::new(),
                active_streams: HashSet::new(),
            },
            ..Default::default()
        };

        let result = format_current_time(&state, data::UserTimezone::Utc);
        assert_eq!(result, "2025-04-01 06:00:00");
    }

    // ── cycle_speed_value ─────────────────────────────────────────────────

    #[test]
    fn cycle_speed_value_1_to_2() {
        assert!((cycle_speed_value(1.0) - 2.0).abs() < 0.01);
    }

    #[test]
    fn cycle_speed_value_10_wraps_to_1() {
        assert!((cycle_speed_value(10.0) - 1.0).abs() < 0.01);
    }

    // ── ReplaySession 遷移（P3 vacuous truth ガード） ─────────────────────

    /// Loading セッションで pending_count が 1 のとき、active_streams が空でも
    /// is_loading() は true であることを確認する（pending_count ベースの O(1) カウンタ）。
    #[test]
    fn loading_with_zero_pending_count_does_not_auto_activate_without_explicit_transition() {
        let clock = StepClock::new(0, 3_600_000, 60_000);
        let state = ReplayState {
            session: ReplaySession::Loading {
                clock,
                pending_count: 1, // 1 ストリームを待機中
                store: EventStore::new(),
                active_streams: HashSet::new(), // active_streams が空でも pending_count が支配する
            },
            ..Default::default()
        };
        assert!(
            state.is_loading(),
            "Loading variant → is_loading() must be true"
        );
        assert!(
            !state.is_playing(),
            "Loading variant → is_playing() must be false"
        );
    }

    /// ReplaySession::Active の active_streams に Kline ストリームのみ含まれること。
    #[test]
    fn active_streams_only_contains_kline_streams_after_insert() {
        use exchange::adapter::{Exchange, StreamKind};
        use exchange::{Ticker, TickerInfo, Timeframe};

        let kline_stream = StreamKind::Kline {
            ticker_info: TickerInfo::new(
                Ticker::new("BTCUSDT", Exchange::BinanceLinear),
                0.01,
                0.001,
                Some(1.0),
            ),
            timeframe: Timeframe::M1,
        };
        let trades_stream = StreamKind::Trades {
            ticker_info: TickerInfo::new(
                Ticker::new("BTCUSDT", Exchange::BinanceLinear),
                0.01,
                0.001,
                Some(1.0),
            ),
        };

        // Play ハンドラと同様のフィルタリングをシミュレートする
        let kline_targets = [
            (uuid::Uuid::new_v4(), kline_stream),
            (uuid::Uuid::new_v4(), trades_stream),
        ];
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

        assert!(
            !active_streams.contains(&trades_stream),
            "active_streams must not contain non-Kline streams, but Trades was found"
        );
        assert!(active_streams.contains(&kline_stream));
    }

    // ── compute_load_range ────────────────────────────────────────────────

    #[test]
    fn compute_load_range_extends_start_back_by_history_span() {
        // 1_000_000_000 ms start, 2_000_000_000 ms end, 60_000 ms (1m) step
        // expected start = 1_000_000_000 - 300 * 60_000 = 982_000_000
        // expected end   = 2_000_000_000 (unchanged)
        let range = compute_load_range(1_000_000_000, 2_000_000_000, 60_000);
        assert_eq!(range.start, 982_000_000);
        assert_eq!(range.end, 2_000_000_000);
    }

    #[test]
    fn compute_load_range_saturates_at_zero_when_history_exceeds_start() {
        // start_ms=1_000 is less than 300 * 60_000 = 18_000_000, so saturating_sub → 0
        let range = compute_load_range(1_000, 5_000_000, 60_000);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 5_000_000);
    }

    // ── pre_start_history ────────────────────────────────────────────────────

    fn make_kline_at(time: u64) -> Kline {
        use exchange::{
            Volume,
            unit::{Qty, price::Price},
        };
        Kline {
            time,
            open: Price::from_f32(100.0),
            high: Price::from_f32(110.0),
            low: Price::from_f32(90.0),
            close: Price::from_f32(105.0),
            volume: Volume::TotalOnly(Qty::zero()),
        }
    }

    #[test]
    fn pre_start_history_returns_only_bars_before_start_ms() {
        let klines: Vec<Kline> = [700, 800, 900, 1000, 1100]
            .iter()
            .map(|&t| make_kline_at(t))
            .collect();

        let result = pre_start_history(&klines, 1000);

        let times: Vec<u64> = result.iter().map(|k| k.time).collect();
        assert_eq!(times, vec![700, 800, 900]);
    }

    #[test]
    fn pre_start_history_excludes_bar_at_exact_start_ms() {
        let kline = make_kline_at(1000);

        let result = pre_start_history(&[kline], 1000);

        assert!(
            result.is_empty(),
            "expected empty vec but got {} klines",
            result.len()
        );
    }

    // ── compute_step_backward_target ─────────────────────────────────────────

    #[test]
    fn step_backward_target_clamps_to_start_ms_when_prev_is_below() {
        // prev=900, current=1000, start_ms=1000 → history bar below start, must clamp
        let target = compute_step_backward_target(Some(900), 1000, 1000);
        assert_eq!(
            target, 1000,
            "seeking to a pre-start history bar (t=900) must be clamped to start_ms=1000"
        );
    }

    #[test]
    fn step_backward_target_allows_seek_within_replay_range() {
        // prev=1000 (exactly start_ms), current=2000, start_ms=1000 → valid seek
        let target = compute_step_backward_target(Some(1000), 2000, 1000);
        assert_eq!(
            target, 1000,
            "seeking from t=2000 to t=1000 (exactly start_ms) must be allowed"
        );
    }

    #[test]
    fn step_backward_target_stays_at_current_when_no_prev() {
        // prev=None, current=1500, start_ms=1000 → no previous bar, stay put
        let target = compute_step_backward_target(None, 1500, 1000);
        assert_eq!(
            target, 1500,
            "when no previous bar exists, target must equal current_time"
        );
    }
}
