use std::collections::HashMap;

use exchange::Trade;
use exchange::adapter::StreamKind;

/// API から iced app へ送るコマンド
#[derive(Debug, Clone)]
pub enum ReplayCommand {
    GetStatus,
    Toggle,
    Play { start: String, end: String },
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

/// リプレイモードの状態を管理する
pub struct ReplayState {
    /// ライブ / リプレイの切替
    pub mode: ReplayMode,
    /// リプレイ範囲の設定（UI入力）
    pub range_input: ReplayRangeInput,
    /// リプレイ実行中の状態（再生開始後に Some になる）
    pub playback: Option<PlaybackState>,
    /// 前回の Tick 時刻（フレーム間経過時間を計算するため）
    pub last_tick: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    Live,
    Replay,
}

#[derive(Default)]
pub struct ReplayRangeInput {
    pub start: String,
    pub end: String,
}

pub struct PlaybackState {
    /// リプレイ範囲（パース済み、Unix ms）
    pub start_time: u64,
    pub end_time: u64,
    /// 現在の仮想時刻（Unix ms）
    pub current_time: u64,
    /// 再生状態
    pub status: PlaybackStatus,
    /// 再生速度倍率
    pub speed: f64,
    /// プリフェッチ済み Trades バッファ
    pub trade_buffers: HashMap<StreamKind, TradeBuffer>,
    /// DataLoaded 後に復帰するステータス（StepBackward 時は Paused にする）
    pub resume_status: PlaybackStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Loading,
    Playing,
    Paused,
}

pub struct TradeBuffer {
    pub trades: Vec<Trade>,
    /// 次に注入するインデックス
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub enum ReplayMessage {
    /// ライブ/リプレイ切替
    ToggleMode,
    /// 開始日時の入力変更
    StartTimeChanged(String),
    /// 終了日時の入力変更
    EndTimeChanged(String),
    /// 再生ボタン押下（最初から開始）
    Play,
    /// 一時停止から再開
    Resume,
    /// 停止ボタン押下
    Pause,
    /// 進むボタン（1分早送り）
    StepForward,
    /// Trades バッチ受信（Straw ストリームから逐次到着）
    TradesBatchReceived(StreamKind, Vec<Trade>),
    /// 全 Trades のフェッチ完了
    TradesFetchCompleted(StreamKind),
    /// 再生速度変更
    CycleSpeed,
    /// 巻き戻し（1分前にジャンプ）
    StepBackward,
    /// 全データのプリフェッチ完了
    DataLoaded,
    /// データプリフェッチ失敗
    DataLoadFailed(String),
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            mode: ReplayMode::Live,
            range_input: ReplayRangeInput::default(),
            playback: None,
            last_tick: None,
        }
    }
}

impl ReplayState {
    /// モードをトグルする。Replay→Live の場合は PlaybackState を破棄し入力をリセットする。
    pub fn toggle_mode(&mut self) {
        match self.mode {
            ReplayMode::Live => {
                self.mode = ReplayMode::Replay;
            }
            ReplayMode::Replay => {
                self.mode = ReplayMode::Live;
                self.playback = None;
                self.range_input = ReplayRangeInput::default();
            }
        }
    }

    /// リプレイモードかどうか
    pub fn is_replay(&self) -> bool {
        self.mode == ReplayMode::Replay
    }

    /// 現在の状態を API レスポンス用に変換
    pub fn to_status(&self) -> ReplayStatus {
        let mode = match self.mode {
            ReplayMode::Live => "Live".to_string(),
            ReplayMode::Replay => "Replay".to_string(),
        };

        let range_start = self.range_input.start.clone();
        let range_end = self.range_input.end.clone();

        match &self.playback {
            Some(pb) => ReplayStatus {
                mode,
                status: Some(match pb.status {
                    PlaybackStatus::Loading => "Loading".to_string(),
                    PlaybackStatus::Playing => "Playing".to_string(),
                    PlaybackStatus::Paused => "Paused".to_string(),
                }),
                current_time: Some(pb.current_time),
                speed: Some(pb.speed_label()),
                start_time: Some(pb.start_time),
                end_time: Some(pb.end_time),
                range_start,
                range_end,
            },
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

impl TradeBuffer {
    /// cursor から current_time 以前の trades を取り出す（スライス参照を返す）。
    /// cursor を進めて次回の呼び出しに備える。
    pub fn drain_until(&mut self, current_time: u64) -> &[Trade] {
        let start = self.cursor;
        while self.cursor < self.trades.len() && self.trades[self.cursor].time <= current_time {
            self.cursor += 1;
        }
        &self.trades[start..self.cursor]
    }

    /// バッファの全 trades を消費済みかどうか
    #[allow(dead_code)]
    pub fn is_exhausted(&self) -> bool {
        self.cursor >= self.trades.len()
    }
}

/// 利用可能な再生速度
const SPEEDS: &[f64] = &[1.0, 2.0, 5.0, 10.0];

impl PlaybackState {
    /// 再生速度を次の段階にサイクルする (1x → 2x → 5x → 10x → 1x)
    pub fn cycle_speed(&mut self) {
        let current_idx = SPEEDS
            .iter()
            .position(|&s| (s - self.speed).abs() < 0.01)
            .unwrap_or(0);
        self.speed = SPEEDS[(current_idx + 1) % SPEEDS.len()];
    }

    /// 現在の速度を表示用文字列で返す
    pub fn speed_label(&self) -> String {
        if self.speed == self.speed.floor() {
            format!("{}x", self.speed as u32)
        } else {
            format!("{:.1}x", self.speed)
        }
    }

    /// フレーム経過時間に基づいて current_time を進める。
    /// 戻り値: 進めた後の current_time
    pub fn advance_time(&mut self, elapsed_ms: f64) -> u64 {
        let delta = (elapsed_ms * self.speed) as u64;
        self.current_time = (self.current_time + delta).min(self.end_time);
        self.current_time
    }
}

/// 現在時刻の表示文字列を生成する。
/// ライブモード: 現在時刻、リプレイモード: 仮想時刻（playback の current_time）
pub fn format_current_time(replay: &ReplayState, timezone: data::UserTimezone) -> String {
    let timestamp_ms: i64 = match (&replay.mode, &replay.playback) {
        (ReplayMode::Replay, Some(pb)) => pb.current_time as i64,
        _ => chrono::Utc::now().timestamp_millis(),
    };

    timezone
        .format_with_kind(
            timestamp_ms,
            data::config::timezone::TimeLabelKind::Custom("%Y-%m-%d %H:%M:%S"),
        )
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::unit::{Price, Qty};

    fn test_trade(time: u64) -> Trade {
        Trade {
            time,
            price: Price { units: 100_000_000 },
            qty: Qty { units: 1 },
            is_sell: false,
        }
    }

    #[test]
    fn default_state_is_live_mode() {
        let state = ReplayState::default();
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
        assert!(state.playback.is_none());
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
    fn toggle_mode_switches_replay_to_live_and_resets() {
        let mut state = ReplayState::default();
        state.toggle_mode(); // Live → Replay
        state.range_input.start = "2026-04-01 09:00".to_string();
        state.range_input.end = "2026-04-01 15:00".to_string();

        state.toggle_mode(); // Replay → Live
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
        assert!(state.range_input.start.is_empty());
        assert!(state.range_input.end.is_empty());
        assert!(state.playback.is_none());
    }

    #[test]
    fn format_current_time_uses_playback_time_in_replay() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: Some(PlaybackState {
                start_time: 1743487200000, // 2025-04-01 06:00:00 UTC
                end_time: 1743508800000,
                current_time: 1743487200000,
                status: PlaybackStatus::Playing,
                speed: 1.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };

        let result = format_current_time(&state, data::UserTimezone::Utc);
        assert_eq!(result, "2025-04-01 06:00:00");
    }

    #[test]
    fn parse_replay_range_valid_input() {
        let (start, end) = parse_replay_range("2026-04-01 09:00", "2026-04-01 15:00").unwrap();
        // 6 hours apart
        assert_eq!(end - start, 6 * 60 * 60 * 1000);
        // Verify the parsed timestamps are reasonable (2026 era)
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
    fn drain_trades_until_returns_trades_up_to_time() {
        let mut buffer = TradeBuffer {
            trades: vec![
                test_trade(100),
                test_trade(200),
                test_trade(300),
                test_trade(400),
            ],
            cursor: 0,
        };

        let drained = buffer.drain_until(250);
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].time, 100);
        assert_eq!(drained[1].time, 200);

        // 次の呼び出しでは残りから
        let drained2 = buffer.drain_until(350);
        assert_eq!(drained2.len(), 1);
        assert_eq!(drained2[0].time, 300);
    }

    #[test]
    fn drain_trades_empty_when_no_matching() {
        let mut buffer = TradeBuffer {
            trades: vec![test_trade(500)],
            cursor: 0,
        };

        let drained = buffer.drain_until(100);
        assert_eq!(drained.len(), 0);
        assert!(!buffer.is_exhausted());
    }

    #[test]
    fn trade_buffer_exhausted_after_all_drained() {
        let mut buffer = TradeBuffer {
            trades: vec![test_trade(100)],
            cursor: 0,
        };

        buffer.drain_until(200);
        assert!(buffer.is_exhausted());
    }

    #[test]
    fn advance_time_respects_speed_and_end_time() {
        let mut pb = PlaybackState {
            start_time: 1000,
            end_time: 2000,
            current_time: 1000,
            status: PlaybackStatus::Playing,
            speed: 2.0,
            trade_buffers: HashMap::new(),
        };

        // 16ms elapsed at 2x speed = 32ms advance
        let t = pb.advance_time(16.0);
        assert_eq!(t, 1032);

        // Jump to near end
        pb.current_time = 1990;
        let t = pb.advance_time(16.0);
        // Would be 1990+32=2022, but clamped to end_time=2000
        assert_eq!(t, 2000);
    }

    #[test]
    fn cycle_speed_rotates_through_presets() {
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1000,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: HashMap::new(),
        };

        pb.cycle_speed();
        assert_eq!(pb.speed_label(), "2x");
        pb.cycle_speed();
        assert_eq!(pb.speed_label(), "5x");
        pb.cycle_speed();
        assert_eq!(pb.speed_label(), "10x");
        pb.cycle_speed();
        assert_eq!(pb.speed_label(), "1x"); // wraps around
    }

    #[test]
    fn format_current_time_uses_realtime_in_live_mode() {
        let state = ReplayState::default();
        let result = format_current_time(&state, data::UserTimezone::Utc);
        // ライブモードでは現在時刻が返る。正確な値はテストできないが、空でないことを確認
        assert!(!result.is_empty());
        // フォーマットが "YYYY-MM-DD HH:MM:SS" 形式であることを確認
        assert_eq!(result.len(), 19);
    }

    // ── to_status() tests ──

    #[test]
    fn to_status_live_mode_no_playback() {
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
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: Some(PlaybackState {
                start_time: 1000,
                end_time: 2000,
                current_time: 1500,
                status: PlaybackStatus::Playing,
                speed: 2.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };
        let status = state.to_status();
        assert_eq!(status.mode, "Replay");
        assert_eq!(status.status.as_deref(), Some("Playing"));
        assert_eq!(status.current_time, Some(1500));
        assert_eq!(status.speed.as_deref(), Some("2x"));
        assert_eq!(status.start_time, Some(1000));
        assert_eq!(status.end_time, Some(2000));
    }

    #[test]
    fn to_status_replay_loading() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: Some(PlaybackState {
                start_time: 0,
                end_time: 1000,
                current_time: 0,
                status: PlaybackStatus::Loading,
                speed: 1.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };
        let status = state.to_status();
        assert_eq!(status.status.as_deref(), Some("Loading"));
    }

    #[test]
    fn to_status_replay_paused() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: Some(PlaybackState {
                start_time: 0,
                end_time: 1000,
                current_time: 500,
                status: PlaybackStatus::Paused,
                speed: 5.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };
        let status = state.to_status();
        assert_eq!(status.status.as_deref(), Some("Paused"));
        assert_eq!(status.speed.as_deref(), Some("5x"));
    }

    #[test]
    fn to_status_includes_range_input() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-10 09:00".to_string(),
                end: "2026-04-10 15:00".to_string(),
            },
            playback: None,
            last_tick: None,
        };
        let status = state.to_status();
        assert_eq!(status.mode, "Replay");
        assert!(status.status.is_none());
        assert_eq!(status.range_start, "2026-04-10 09:00");
        assert_eq!(status.range_end, "2026-04-10 15:00");
    }

    // ── to_status() serialization test ──

    #[test]
    fn to_status_live_serializes_without_optional_fields() {
        let state = ReplayState::default();
        let status = state.to_status();
        let json = serde_json::to_string(&status).unwrap();
        // Live mode: only "mode" should be present (skip_serializing_if = None)
        assert!(json.contains(r#""mode":"Live""#));
        assert!(!json.contains("status"));
        assert!(!json.contains("current_time"));
        assert!(!json.contains("speed"));
    }

    #[test]
    fn to_status_replay_serializes_all_fields() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: Some(PlaybackState {
                start_time: 1000,
                end_time: 2000,
                current_time: 1500,
                status: PlaybackStatus::Playing,
                speed: 1.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };
        let json = serde_json::to_string(&state.to_status()).unwrap();
        assert!(json.contains(r#""mode":"Replay""#));
        assert!(json.contains(r#""status":"Playing""#));
        assert!(json.contains(r#""current_time":1500"#));
        assert!(json.contains(r#""speed":"1x""#));
    }

    // ── TradeBuffer edge cases ──

    #[test]
    fn empty_trade_buffer_is_exhausted() {
        let buffer = TradeBuffer {
            trades: vec![],
            cursor: 0,
        };
        assert!(buffer.is_exhausted());
    }

    #[test]
    fn drain_empty_buffer_returns_empty_slice() {
        let mut buffer = TradeBuffer {
            trades: vec![],
            cursor: 0,
        };
        let drained = buffer.drain_until(9999);
        assert!(drained.is_empty());
        assert!(buffer.is_exhausted());
    }

    #[test]
    fn drain_trades_same_time_returns_all() {
        let mut buffer = TradeBuffer {
            trades: vec![test_trade(100), test_trade(100), test_trade(100)],
            cursor: 0,
        };
        let drained = buffer.drain_until(100);
        assert_eq!(drained.len(), 3);
        assert!(buffer.is_exhausted());
    }

    #[test]
    fn drain_trades_sequential_exhaustion() {
        let mut buffer = TradeBuffer {
            trades: vec![test_trade(10), test_trade(20), test_trade(30)],
            cursor: 0,
        };

        let d1 = buffer.drain_until(10);
        assert_eq!(d1.len(), 1);
        assert!(!buffer.is_exhausted());

        let d2 = buffer.drain_until(20);
        assert_eq!(d2.len(), 1);
        assert!(!buffer.is_exhausted());

        let d3 = buffer.drain_until(30);
        assert_eq!(d3.len(), 1);
        assert!(buffer.is_exhausted());

        // After exhaustion, further drains return empty
        let d4 = buffer.drain_until(9999);
        assert!(d4.is_empty());
    }

    // ── advance_time edge cases ──

    #[test]
    fn advance_time_zero_elapsed_no_change() {
        let mut pb = PlaybackState {
            start_time: 1000,
            end_time: 2000,
            current_time: 1500,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: HashMap::new(),
        };
        let t = pb.advance_time(0.0);
        assert_eq!(t, 1500);
    }

    #[test]
    fn advance_time_already_at_end() {
        let mut pb = PlaybackState {
            start_time: 1000,
            end_time: 2000,
            current_time: 2000,
            status: PlaybackStatus::Playing,
            speed: 10.0,
            trade_buffers: HashMap::new(),
        };
        let t = pb.advance_time(1000.0);
        assert_eq!(t, 2000); // stays at end_time
    }

    #[test]
    fn advance_time_large_elapsed_clamped() {
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 100,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: HashMap::new(),
        };
        let t = pb.advance_time(999999.0);
        assert_eq!(t, 100); // clamped to end_time
    }

    // ── speed_label tests ──

    #[test]
    fn speed_label_all_presets() {
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1000,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: HashMap::new(),
        };
        assert_eq!(pb.speed_label(), "1x");
        pb.speed = 2.0;
        assert_eq!(pb.speed_label(), "2x");
        pb.speed = 5.0;
        assert_eq!(pb.speed_label(), "5x");
        pb.speed = 10.0;
        assert_eq!(pb.speed_label(), "10x");
    }

    #[test]
    fn speed_label_fractional() {
        let pb = PlaybackState {
            start_time: 0,
            end_time: 1000,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 1.5,
            trade_buffers: HashMap::new(),
        };
        assert_eq!(pb.speed_label(), "1.5x");
    }

    // ── parse_replay_range edge cases ──

    #[test]
    fn parse_replay_range_same_start_and_end() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-01 09:00");
        assert_eq!(result, Err(ParseRangeError::StartAfterEnd));
    }

    #[test]
    fn parse_replay_range_with_seconds_format_rejected() {
        // Our format is "%Y-%m-%d %H:%M", seconds should fail
        let result = parse_replay_range("2026-04-01 09:00:00", "2026-04-01 15:00");
        assert_eq!(result, Err(ParseRangeError::InvalidStartFormat));
    }

    #[test]
    fn parse_replay_range_empty_strings() {
        let result = parse_replay_range("", "");
        assert_eq!(result, Err(ParseRangeError::InvalidStartFormat));
    }

    #[test]
    fn parse_replay_range_1_minute_apart_is_ok() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-01 09:01");
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(end - start, 60_000); // 1 minute in ms
    }

    // ── toggle_mode edge cases ──

    #[test]
    fn toggle_mode_with_active_playback_clears_it() {
        let mut state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-01 09:00".to_string(),
                end: "2026-04-01 15:00".to_string(),
            },
            playback: Some(PlaybackState {
                start_time: 1000,
                end_time: 2000,
                current_time: 1500,
                status: PlaybackStatus::Playing,
                speed: 5.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };

        state.toggle_mode(); // Replay → Live
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(state.playback.is_none());
        assert!(state.range_input.start.is_empty());
    }

    #[test]
    fn toggle_mode_round_trip() {
        let mut state = ReplayState::default();
        assert_eq!(state.mode, ReplayMode::Live);

        state.toggle_mode(); // Live → Replay
        assert_eq!(state.mode, ReplayMode::Replay);

        state.toggle_mode(); // Replay → Live
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
    }

    // ── cycle_speed edge case ──

    #[test]
    fn cycle_speed_from_unknown_value_resets_to_second_preset() {
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1000,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 99.0, // not in SPEEDS
            trade_buffers: HashMap::new(),
        };
        pb.cycle_speed();
        // unwrap_or(0) → (0+1) % 4 = 1 → 2.0
        assert_eq!(pb.speed, 2.0);
    }

    // ── format_current_time edge case ──

    #[test]
    fn format_current_time_replay_no_playback_uses_realtime() {
        // mode=Replay but playback=None (e.g., toggled to Replay but not yet started play)
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput::default(),
            playback: None,
            last_tick: None,
        };
        let result = format_current_time(&state, data::UserTimezone::Utc);
        // Should fall through to realtime (the _ arm)
        assert!(!result.is_empty());
        assert_eq!(result.len(), 19);
    }

    // ════════════════════════════════════════════════════════
    // 永続化テスト: ReplayState ↔ ReplayConfig 変換
    // ════════════════════════════════════════════════════════

    /// save_state_to_disk() と同等の変換ロジック (ReplayState → ReplayConfig)
    fn to_replay_config(state: &ReplayState) -> data::ReplayConfig {
        data::ReplayConfig {
            mode: match state.mode {
                ReplayMode::Live => "live".into(),
                ReplayMode::Replay => "replay".into(),
            },
            range_start: state.range_input.start.clone(),
            range_end: state.range_input.end.clone(),
        }
    }

    /// Flowsurface::new() と同等の変換ロジック (ReplayConfig → ReplayState)
    fn from_replay_config(cfg: data::ReplayConfig) -> ReplayState {
        let replay_mode = match cfg.mode.as_str() {
            "replay" => ReplayMode::Replay,
            _ => ReplayMode::Live,
        };
        ReplayState {
            mode: replay_mode,
            range_input: ReplayRangeInput {
                start: cfg.range_start,
                end: cfg.range_end,
            },
            playback: None,
            last_tick: None,
        }
    }

    // ── 保存方向: ReplayState → ReplayConfig ──

    #[test]
    fn persist_live_mode_produces_live_config() {
        let state = ReplayState::default();
        let cfg = to_replay_config(&state);
        assert_eq!(cfg.mode, "live");
        assert!(cfg.range_start.is_empty());
        assert!(cfg.range_end.is_empty());
    }

    #[test]
    fn persist_replay_mode_with_ranges() {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-10 09:00".into(),
                end: "2026-04-10 15:00".into(),
            },
            playback: None,
            last_tick: None,
        };
        let cfg = to_replay_config(&state);
        assert_eq!(cfg.mode, "replay");
        assert_eq!(cfg.range_start, "2026-04-10 09:00");
        assert_eq!(cfg.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn persist_replay_mode_with_active_playback_ignores_playback() {
        // playback (current_time, speed, trade_buffers 等) は保存対象外
        let state = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-10 09:00".into(),
                end: "2026-04-10 15:00".into(),
            },
            playback: Some(PlaybackState {
                start_time: 1000,
                end_time: 2000,
                current_time: 1500,
                status: PlaybackStatus::Playing,
                speed: 5.0,
                trade_buffers: HashMap::new(),
            }),
            last_tick: None,
        };
        let cfg = to_replay_config(&state);
        // mode と range だけが保存される
        assert_eq!(cfg.mode, "replay");
        assert_eq!(cfg.range_start, "2026-04-10 09:00");
        assert_eq!(cfg.range_end, "2026-04-10 15:00");
    }

    // ── 復元方向: ReplayConfig → ReplayState ──

    #[test]
    fn restore_live_config_produces_live_state() {
        let cfg = data::ReplayConfig::default();
        let state = from_replay_config(cfg);
        assert_eq!(state.mode, ReplayMode::Live);
        assert!(!state.is_replay());
        assert!(state.range_input.start.is_empty());
        assert!(state.range_input.end.is_empty());
        assert!(state.playback.is_none());
        assert!(state.last_tick.is_none());
    }

    #[test]
    fn restore_replay_config_produces_replay_state_with_ranges() {
        let cfg = data::ReplayConfig {
            mode: "replay".into(),
            range_start: "2026-04-10 09:00".into(),
            range_end: "2026-04-10 15:00".into(),
        };
        let state = from_replay_config(cfg);
        assert_eq!(state.mode, ReplayMode::Replay);
        assert!(state.is_replay());
        assert_eq!(state.range_input.start, "2026-04-10 09:00");
        assert_eq!(state.range_input.end, "2026-04-10 15:00");
        // playback は復元しない（ユーザーが Play を押して開始する形）
        assert!(state.playback.is_none());
        assert!(state.last_tick.is_none());
    }

    #[test]
    fn restore_unknown_mode_falls_back_to_live() {
        let cfg = data::ReplayConfig {
            mode: "unknown".into(),
            range_start: "".into(),
            range_end: "".into(),
        };
        let state = from_replay_config(cfg);
        assert_eq!(state.mode, ReplayMode::Live);
    }

    #[test]
    fn restore_empty_mode_falls_back_to_live() {
        let cfg = data::ReplayConfig {
            mode: "".into(),
            range_start: "".into(),
            range_end: "".into(),
        };
        let state = from_replay_config(cfg);
        assert_eq!(state.mode, ReplayMode::Live);
    }

    // ── ラウンドトリップ: ReplayState → ReplayConfig → JSON → ReplayConfig → ReplayState ──

    #[test]
    fn roundtrip_live_mode() {
        let original = ReplayState::default();
        let cfg = to_replay_config(&original);
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg_restored: data::ReplayConfig = serde_json::from_str(&json).unwrap();
        let restored = from_replay_config(cfg_restored);

        assert_eq!(restored.mode, original.mode);
        assert_eq!(restored.range_input.start, original.range_input.start);
        assert_eq!(restored.range_input.end, original.range_input.end);
        assert!(restored.playback.is_none());
    }

    #[test]
    fn roundtrip_replay_mode_with_ranges() {
        let original = ReplayState {
            mode: ReplayMode::Replay,
            range_input: ReplayRangeInput {
                start: "2026-04-10 09:00".into(),
                end: "2026-04-10 15:00".into(),
            },
            playback: None,
            last_tick: None,
        };
        let cfg = to_replay_config(&original);
        let json = serde_json::to_string(&cfg).unwrap();
        let cfg_restored: data::ReplayConfig = serde_json::from_str(&json).unwrap();
        let restored = from_replay_config(cfg_restored);

        assert_eq!(restored.mode, ReplayMode::Replay);
        assert_eq!(restored.range_input.start, "2026-04-10 09:00");
        assert_eq!(restored.range_input.end, "2026-04-10 15:00");
    }

    #[test]
    fn roundtrip_via_full_state_json() {
        // State 全体のシリアライズ→デシリアライズで replay が保たれることを確認
        let state_json = serde_json::json!({
            "replay": {
                "mode": "replay",
                "range_start": "2026-04-10 09:00",
                "range_end": "2026-04-10 15:00"
            }
        });
        let state: data::State = serde_json::from_value(state_json).unwrap();
        let restored = from_replay_config(state.replay);
        assert_eq!(restored.mode, ReplayMode::Replay);
        assert_eq!(restored.range_input.start, "2026-04-10 09:00");
        assert_eq!(restored.range_input.end, "2026-04-10 15:00");
    }

    // ── 後方互換テスト ──

    #[test]
    fn backward_compat_no_replay_key_restores_as_live() {
        let state: data::State = serde_json::from_str("{}").unwrap();
        let restored = from_replay_config(state.replay);
        assert_eq!(restored.mode, ReplayMode::Live);
        assert!(restored.range_input.start.is_empty());
        assert!(restored.range_input.end.is_empty());
    }

    #[test]
    fn backward_compat_empty_replay_object_restores_as_live() {
        let json = r#"{"replay":{}}"#;
        let state: data::State = serde_json::from_str(json).unwrap();
        let restored = from_replay_config(state.replay);
        assert_eq!(restored.mode, ReplayMode::Live);
    }

    // ── to_status() が復元後の状態を正しく反映するか ──

    #[test]
    fn restored_replay_state_status_shows_replay_no_playback() {
        let cfg = data::ReplayConfig {
            mode: "replay".into(),
            range_start: "2026-04-10 09:00".into(),
            range_end: "2026-04-10 15:00".into(),
        };
        let state = from_replay_config(cfg);
        let status = state.to_status();
        // mode=Replay だが playback=None → status/current_time/speed は None
        assert_eq!(status.mode, "Replay");
        assert!(status.status.is_none());
        assert!(status.current_time.is_none());
        assert!(status.speed.is_none());
    }

    #[test]
    fn restored_live_state_status_shows_live() {
        let cfg = data::ReplayConfig::default();
        let state = from_replay_config(cfg);
        let status = state.to_status();
        assert_eq!(status.mode, "Live");
        assert!(status.status.is_none());
    }
}
