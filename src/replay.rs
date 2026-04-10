use std::collections::HashMap;

use exchange::Trade;
use exchange::adapter::StreamKind;

/// リプレイモードの状態を管理する
pub struct ReplayState {
    /// ライブ / リプレイの切替
    pub mode: ReplayMode,
    /// リプレイ範囲の設定（UI入力）
    pub range_input: ReplayRangeInput,
    /// リプレイ実行中の状態（再生開始後に Some になる）
    pub playback: Option<PlaybackState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayMode {
    Live,
    Replay,
}

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
    /// 再生ボタン押下
    Play,
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
        }
    }
}

impl Default for ReplayRangeInput {
    fn default() -> Self {
        Self {
            start: String::new(),
            end: String::new(),
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
}

/// 最大リプレイ範囲（6時間）
const MAX_REPLAY_DURATION_MS: u64 = 6 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRangeError {
    InvalidStartFormat,
    InvalidEndFormat,
    StartAfterEnd,
    RangeTooLong,
}

impl std::fmt::Display for ParseRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseRangeError::InvalidStartFormat => write!(f, "Invalid start time format"),
            ParseRangeError::InvalidEndFormat => write!(f, "Invalid end time format"),
            ParseRangeError::StartAfterEnd => write!(f, "Start time must be before end time"),
            ParseRangeError::RangeTooLong => write!(f, "Range must be 6 hours or less"),
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
    if end_ms - start_ms > MAX_REPLAY_DURATION_MS {
        return Err(ParseRangeError::RangeTooLong);
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
    fn parse_replay_range_exceeds_6_hours() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-01 15:01");
        assert_eq!(result, Err(ParseRangeError::RangeTooLong));
    }

    #[test]
    fn parse_replay_range_exactly_6_hours_is_ok() {
        let result = parse_replay_range("2026-04-01 09:00", "2026-04-01 15:00");
        assert!(result.is_ok());
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
}
