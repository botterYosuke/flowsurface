use std::collections::{HashMap, HashSet};

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
    /// 次バー発火までの仮想時間累積（ms）。
    /// `process_tick` が毎 Tick `elapsed_ms * pb.speed` を加算し、`comparison_threshold`
    /// に到達したら `pb.current_time` を進めて、しきい値ぶん減算する（§2.1.1 案 C）。
    pub virtual_elapsed_ms: f64,
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
    /// バックフィル中の trade stream 集合（§2.3.1.1 穴 B）。
    /// 毎 Tick の `drain_until` 対象から除外され、`TradesFetchCompleted` 受信時に
    /// `advance_cursor_to(current_time)` の直後に削除される。
    pub pending_trade_streams: HashSet<StreamKind>,
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
    /// mid-replay で stream 構成が変わった際に発火（§2.6.5）。
    /// `pending_trade_streams` + `replay_kline_buffer` の差分を計算し、不足 stream に対して
    /// バックフィルを発火する。`refresh_streams()` / `Effect::RequestFetch` 末尾で chain される。
    SyncReplayBuffers,
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            mode: ReplayMode::Live,
            range_input: ReplayRangeInput::default(),
            playback: None,
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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

    /// cursor を `target_time` 以前の位置まで早送りする。
    /// 既に cursor が先にあれば no-op（単調増加を保証）。
    ///
    /// 戻り値: 早送りによってスキップした trades 件数（捨てて良い数）。
    ///
    /// 用途（§2.3.1 mid-replay バックフィル）:
    /// - 新規ペインに対して `fetch_trades_batched()` を発火し `start → end` 全量の trades が
    ///   届いた後、`TradesFetchCompleted` を受けて `advance_cursor_to(pb.current_time)` を呼ぶ
    /// - これにより過去分の trades を UI に流さず、以降は通常の `drain_until` 経路に合流する
    pub fn advance_cursor_to(&mut self, target_time: u64) -> usize {
        let found = self
            .trades
            .iter()
            .position(|t| t.time > target_time)
            .unwrap_or(self.trades.len());
        let new_cursor = found.max(self.cursor);
        let skipped = new_cursor - self.cursor;
        self.cursor = new_cursor;
        skipped
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

    /// `TradesBatchReceived` ハンドラ用の受け入れ判定付き追記（review 🔴 #2）。
    ///
    /// 登録済み stream のみバッチを受け入れる。**未登録 stream は黙って捨てる**（`or_insert_with`
    /// で復活させない）。これにより mid-replay でペインを削除した直後に到着する残存 fetch タスクの
    /// バッチが orphan buffer を自己復活させるループを防ぐ。
    ///
    /// 戻り値: 受け入れた場合 `true`、orphan として捨てた場合 `false`。
    pub fn ingest_trades_batch(&mut self, stream: StreamKind, batch: Vec<Trade>) -> bool {
        match self.trade_buffers.get_mut(&stream) {
            Some(buffer) => {
                buffer.trades.extend(batch);
                true
            }
            None => false,
        }
    }

    /// 現在の dashboard の trade stream 集合と `trade_buffers` を突き合わせ、
    /// 追加すべき stream と削除すべき stream を計算する（review 🟡 #4）。
    ///
    /// 呼び出し側（`SyncReplayBuffers` ハンドラ）はこの結果を使って `trade_buffers` /
    /// `pending_trade_streams` の更新とバックフィルタスク発火を行う。順序保持のため `Vec`。
    pub fn diff_trade_streams(&self, current: &[StreamKind]) -> TradeStreamDiff {
        let mut new_streams = Vec::new();
        for stream in current {
            if !self.trade_buffers.contains_key(stream) && !new_streams.contains(stream) {
                new_streams.push(*stream);
            }
        }
        let mut orphan_streams = Vec::new();
        for stream in self.trade_buffers.keys() {
            if !current.contains(stream) {
                orphan_streams.push(*stream);
            }
        }
        TradeStreamDiff {
            new_streams,
            orphan_streams,
        }
    }
}

/// `PlaybackState::diff_trade_streams` の戻り値。
#[derive(Debug, Clone, Default)]
pub struct TradeStreamDiff {
    /// `trade_buffers` に存在しないが `current` に含まれる stream（バックフィル対象）
    pub new_streams: Vec<StreamKind>,
    /// `trade_buffers` に存在するが `current` に含まれない stream（削除対象）
    pub orphan_streams: Vec<StreamKind>,
}

/// 統一 Tick ハンドラで使う「粗 timeframe」判定の境界（§2.1.1 案 C）。
/// `delta_to_next >= COARSE_CUTOFF_MS` の場合は粗補正モード（= 1 バー/sec × speed）。
pub const COARSE_CUTOFF_MS: u64 = 3_600_000;

/// 粗補正モードでの比較しきい値（1 バー = 1 秒 × speed）。
pub const COARSE_BAR_MS: u64 = 1_000;

/// 次バー発火の状態（§2.1 / §2.2）。
///
/// - `Ready(t)`: ready chart の `next_time_after` の min が確定している。通常再生。
/// - `Pending`: ready chart が存在しないが、バックフィル中 chart が 1 つ以上ある。待機。
/// - `Terminal`: ready chart が全て終端、かつバックフィル中 chart も無い。Paused へ遷移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireStatus {
    Ready(u64),
    Pending,
    Terminal,
}

/// `process_tick` の戻り値。
pub struct TickResult {
    /// ジャンプ発火時の current_time（None ならチャート更新不要）
    pub current_time: Option<u64>,
    /// drain した trades（None なら trade_buffers が空でドレイン不要）
    pub trades_collected: Option<Vec<(StreamKind, Vec<Trade>, u64)>>,
}

/// `pb.trade_buffers` の全 stream を `drain_until(pb.current_time)` する。
/// `pending_trade_streams` に含まれる stream はバックフィル中のためスキップする（§2.3.1.1 穴 B）。
///
/// 戻り値:
/// - `None`: drain 対象 stream の全 buffer が空（Tachibana ケース）で drain する必要が無い
/// - `Some(collected)`: `(stream, trades, update_t)` のリスト
fn drain_all_trade_buffers(
    pb: &mut PlaybackState,
) -> Option<Vec<(StreamKind, Vec<Trade>, u64)>> {
    let all_empty = pb
        .trade_buffers
        .iter()
        .filter(|(stream, _)| !pb.pending_trade_streams.contains(stream))
        .all(|(_, b)| b.trades.is_empty());
    if all_empty {
        return None;
    }
    let streams: Vec<_> = pb
        .trade_buffers
        .keys()
        .copied()
        .filter(|s| !pb.pending_trade_streams.contains(s))
        .collect();
    let mut collected = Vec::new();
    for stream in streams {
        if let Some(buffer) = pb.trade_buffers.get_mut(&stream) {
            let drained = buffer.drain_until(pb.current_time);
            if !drained.is_empty() {
                let update_t = drained.last().map_or(pb.current_time, |t| t.time);
                collected.push((stream, drained.to_vec(), update_t));
            }
        }
    }
    Some(collected)
}

/// 統一 Tick ハンドラ（§2.1 案 C）。
///
/// D1 / 非 D1 を問わず、`fire_status` をもとに 1 経路で再生を進める。
/// - `Terminal` → `pb.status = Paused`、`virtual_elapsed_ms` をクリア
/// - `Pending`  → 何もしない（`virtual_elapsed_ms` 据え置き、drain も行わない）
/// - `Ready(t)` → `delta = t - current_time` に応じて threshold を切り替え、
///   `virtual_elapsed_ms * speed` が threshold に達したら `current_time = t` にジャンプ
///
/// Trade drain は「`Ready` 経路で常に毎 Tick」実行する。ジャンプ発生時は
/// 「drain → advance → drain」の 2 段階（drain_until は cursor ベースで冪等）。
pub fn process_tick(
    pb: &mut PlaybackState,
    virtual_elapsed_ms: &mut f64,
    elapsed_ms: f64,
    fire_status: FireStatus,
) -> TickResult {
    match fire_status {
        FireStatus::Terminal => {
            pb.status = PlaybackStatus::Paused;
            *virtual_elapsed_ms = 0.0;
            TickResult {
                current_time: None,
                trades_collected: None,
            }
        }
        FireStatus::Pending => TickResult {
            current_time: None,
            trades_collected: None,
        },
        FireStatus::Ready(next_fire) => {
            // 1. まず現在の current_time 時点で drain（穴 A: 未達 Tick でも流す）
            let mut collected = drain_all_trade_buffers(pb).unwrap_or_default();

            let delta_to_next = next_fire.saturating_sub(pb.current_time);
            let threshold_ms = if delta_to_next >= COARSE_CUTOFF_MS {
                COARSE_BAR_MS
            } else {
                delta_to_next
            };

            *virtual_elapsed_ms += elapsed_ms * pb.speed;

            let threshold_f = threshold_ms as f64;
            if *virtual_elapsed_ms + 1e-6 < threshold_f {
                let trades_collected = if collected.is_empty() { None } else { Some(collected) };
                return TickResult {
                    current_time: None,
                    trades_collected,
                };
            }

            // 2. ジャンプ: current_time を進めて、新しい時刻で再度 drain
            *virtual_elapsed_ms -= threshold_f;
            pb.current_time = next_fire;

            if let Some(after_jump) = drain_all_trade_buffers(pb) {
                collected.extend(after_jump);
            }

            let trades_collected = if collected.is_empty() { None } else { Some(collected) };
            TickResult {
                current_time: Some(next_fire),
                trades_collected,
            }
        }
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
            virtual_elapsed_ms: 0.0,
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
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
            virtual_elapsed_ms: 0.0,
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
            virtual_elapsed_ms: 0.0,
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
            virtual_elapsed_ms: 0.0,
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
                resume_status: PlaybackStatus::Playing,
                pending_trade_streams: HashSet::new(),
            }),
            last_tick: None,
            virtual_elapsed_ms: 0.0,
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
            virtual_elapsed_ms: 0.0,
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

    // ── process_tick: 統一 Tick ハンドラ（§2.1 案 C） ──

    fn mock_trade_stream() -> StreamKind {
        use exchange::adapter::Exchange;
        let ticker_info = exchange::TickerInfo::new(
            exchange::Ticker::new("BTCUSDT", Exchange::BinanceSpot),
            1.0,
            1.0,
            None,
        );
        StreamKind::Trades { ticker_info }
    }

    #[test]
    fn replay_state_default_virtual_elapsed_is_zero() {
        let state = ReplayState::default();
        assert_eq!(state.virtual_elapsed_ms, 0.0);
    }

    // ── TradeBuffer::advance_cursor_to（§2.3.1 mid-replay バックフィル対応） ──

    #[test]
    fn advance_cursor_to_forwards_past_trades_and_returns_skipped_count() {
        let mut buf = TradeBuffer {
            trades: vec![test_trade(10), test_trade(20), test_trade(30), test_trade(40)],
            cursor: 0,
        };
        let skipped = buf.advance_cursor_to(25);
        assert_eq!(buf.cursor, 2, "cursor が time>25 の最初の index に進む");
        assert_eq!(skipped, 2);
    }

    #[test]
    fn advance_cursor_to_is_monotonic_does_not_rewind() {
        // cursor が既に target_time を越えていれば no-op（単調増加ガード）
        let mut buf = TradeBuffer {
            trades: vec![test_trade(10), test_trade(20), test_trade(30)],
            cursor: 3,
        };
        let skipped = buf.advance_cursor_to(15);
        assert_eq!(buf.cursor, 3, "既に先にいるので巻き戻さない");
        assert_eq!(skipped, 0);
    }

    #[test]
    fn advance_cursor_to_target_after_all_trades_goes_to_end() {
        let mut buf = TradeBuffer {
            trades: vec![test_trade(10), test_trade(20)],
            cursor: 0,
        };
        let skipped = buf.advance_cursor_to(100);
        assert_eq!(buf.cursor, 2);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn advance_cursor_to_from_partial_cursor_advances_delta() {
        let mut buf = TradeBuffer {
            trades: vec![test_trade(10), test_trade(20), test_trade(30), test_trade(40)],
            cursor: 1,
        };
        let skipped = buf.advance_cursor_to(35);
        assert_eq!(buf.cursor, 3);
        assert_eq!(skipped, 2, "cursor=1 から cursor=3 まで 2 件スキップ");
    }

    #[test]
    fn advance_cursor_to_empty_buffer_is_noop() {
        let mut buf = TradeBuffer {
            trades: Vec::new(),
            cursor: 0,
        };
        let skipped = buf.advance_cursor_to(100);
        assert_eq!(buf.cursor, 0);
        assert_eq!(skipped, 0);
    }

    // ── pending_trade_streams（§2.3.1.1 穴 B）: バックフィル中 stream の drain 除外 ──

    #[test]
    fn process_tick_skips_drain_for_pending_trade_streams() {
        // pending_trade_streams に含まれる stream は drain されない（過去 trades が既存 pane に流入しない）
        // 含まれない stream は通常通り drain される
        let pending_stream = mock_trade_stream();

        // 2 つ目の stream（別 ticker）を用意
        use exchange::adapter::Exchange;
        let normal_ticker = exchange::TickerInfo::new(
            exchange::Ticker::new("ETHUSDT", Exchange::BinanceSpot),
            1.0,
            1.0,
            None,
        );
        let normal_stream = StreamKind::Trades {
            ticker_info: normal_ticker,
        };

        let mut buffers = HashMap::new();
        buffers.insert(
            pending_stream,
            TradeBuffer {
                trades: vec![test_trade(10), test_trade(50)],
                cursor: 0,
            },
        );
        buffers.insert(
            normal_stream,
            TradeBuffer {
                trades: vec![test_trade(10), test_trade(50)],
                cursor: 0,
            },
        );

        let mut pending = std::collections::HashSet::new();
        pending.insert(pending_stream);

        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1_000_000,
            current_time: 100,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: buffers,
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: pending,
        };
        let mut virtual_elapsed = 0.0_f64;

        // Ready 経路で未達 Tick（drain のみ、ジャンプ無し）
        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(60_100),
        );

        assert!(result.current_time.is_none());
        let collected = result.trades_collected.expect("normal stream の drain は実行される");
        assert_eq!(collected.len(), 1, "pending 除外され、normal 1 件のみ");
        let (stream, _, _) = &collected[0];
        assert_eq!(
            *stream, normal_stream,
            "pending ではない normal_stream の drain のみ"
        );

        // pending_stream の cursor は動いていない
        assert_eq!(
            pb.trade_buffers[&pending_stream].cursor, 0,
            "pending stream の cursor は触られない"
        );
    }

    #[test]
    fn pending_trade_stream_cleared_on_advance_cursor_to_simulation() {
        // §2.3.1.1: TradesFetchCompleted 受信時の挙動を模擬する：
        // 1. pending_trade_streams に stream が登録されている
        // 2. バッファに過去 trades が流入（TradesBatchReceived 相当）
        // 3. advance_cursor_to(pb.current_time) を呼ぶ
        // 4. pending_trade_streams から削除 → 次 Tick から drain 対象に復帰
        let stream = mock_trade_stream();
        let mut buffers = HashMap::new();
        buffers.insert(
            stream,
            TradeBuffer {
                trades: vec![test_trade(10), test_trade(50), test_trade(90), test_trade(150)],
                cursor: 0,
            },
        );
        let mut pending = std::collections::HashSet::new();
        pending.insert(stream);

        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1_000_000,
            current_time: 100,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: buffers,
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: pending,
        };

        // バックフィル中なので drain されない
        assert!(pb.pending_trade_streams.contains(&stream));

        // TradesFetchCompleted ハンドラの挙動を模擬:
        let buffer = pb.trade_buffers.get_mut(&stream).unwrap();
        let skipped = buffer.advance_cursor_to(pb.current_time);
        pb.pending_trade_streams.remove(&stream);

        assert_eq!(skipped, 3, "time <= 100 の 3 件が skip される");
        assert_eq!(pb.trade_buffers[&stream].cursor, 3);
        assert!(!pb.pending_trade_streams.contains(&stream));

        // 以降の Tick では drain 対象に復帰し、残りの 1 件（time=150）は
        // current_time=150 になるまで流れない（通常の drain_until 経路）
        let buffer = pb.trade_buffers.get_mut(&stream).unwrap();
        let rem = buffer.drain_until(100);
        assert!(rem.is_empty(), "100 以前はもう drain しない（既に skip 済み）");
        let rem = buffer.drain_until(200);
        assert_eq!(rem.len(), 1, "time=150 の 1 件が drain される");
    }


    fn empty_pb(current: u64) -> PlaybackState {
        PlaybackState {
            start_time: 0,
            end_time: 1_000_000_000_000,
            current_time: current,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: HashMap::new(),
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
        }
    }

    #[test]
    fn process_tick_m1_jumps_after_60s_virtual_elapsed() {
        // delta_to_next = 60_000 (= 1 分) なので COARSE_CUTOFF_MS 未満 → threshold=delta
        // speed=1.0 で elapsed 60_000ms 一発で閾値到達 → ジャンプ
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 60_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            60_000.0,
            FireStatus::Ready(next_t),
        );

        assert_eq!(result.current_time, Some(next_t), "jump to next bar");
        assert_eq!(pb.current_time, next_t);
        assert!(virtual_elapsed.abs() < 1.0, "余剰 virtual_elapsed は 0 付近");
    }

    #[test]
    fn process_tick_m1_throttles_until_accumulated_threshold() {
        // speed=1.0, elapsed=10_000ms を 5 回: 累積 50_000 < 60_000 なのでジャンプ無し
        // 6 回目で 60_000 到達 → ジャンプ
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 60_000;

        for _ in 0..5 {
            let result = process_tick(
                &mut pb,
                &mut virtual_elapsed,
                10_000.0,
                FireStatus::Ready(next_t),
            );
            assert!(result.current_time.is_none());
            assert_eq!(pb.current_time, 1_000_000);
        }

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            10_000.0,
            FireStatus::Ready(next_t),
        );
        assert_eq!(result.current_time, Some(next_t));
        assert_eq!(pb.current_time, next_t);
    }

    #[test]
    fn process_tick_d1_jumps_after_1s_virtual_elapsed() {
        // delta_to_next = 86_400_000 (= 1 日) なので COARSE_CUTOFF_MS 以上 → threshold=COARSE_BAR_MS
        // speed=1.0 で elapsed 1000ms 一発で 1000ms 到達 → ジャンプ
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 86_400_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(next_t),
        );

        assert_eq!(result.current_time, Some(next_t), "jump full delta to next bar");
        assert_eq!(pb.current_time, next_t, "current_time jumps full D1 delta");
    }

    #[test]
    fn process_tick_h1_boundary_uses_coarse_threshold() {
        // delta_to_next = 3_600_000 (= 1h) は境界: `>= COARSE_CUTOFF_MS` なので粗補正側
        // speed=1.0 で 1000ms 一発でジャンプ
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 3_600_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(next_t),
        );

        assert_eq!(result.current_time, Some(next_t));
    }

    #[test]
    fn process_tick_m30_uses_fine_threshold() {
        // delta_to_next = 1_800_000 (= 30 分、M30) は `< COARSE_CUTOFF_MS` なので fine 側。
        // speed=1.0, elapsed=1000ms では threshold(1_800_000) に全く及ばずジャンプしない。
        // これで tooltip 「M30 以下: 実時間連動」の契約が担保される（review 🟡 #3）。
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 1_800_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(next_t),
        );

        assert!(
            result.current_time.is_none(),
            "M30 では 1 秒では threshold(1_800_000) に達せずジャンプしない"
        );
    }

    #[test]
    fn coarse_cutoff_boundary_matches_h1_in_ms() {
        // tooltip の「M30 以下 / H1 以上」の境界が H1 (3_600_000ms) と一致することを固定化。
        // この値が動いたら tooltip 文言の見直しが必要（review 🟡 #3）。
        assert_eq!(COARSE_CUTOFF_MS, 3_600_000);
    }

    #[test]
    fn process_tick_h4_uses_coarse_threshold() {
        // delta_to_next = 14_400_000 (= 4h), speed=1.0, elapsed=1000ms でジャンプ
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 14_400_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(next_t),
        );
        assert_eq!(result.current_time, Some(next_t));
        assert_eq!(pb.current_time, next_t);
    }

    #[test]
    fn process_tick_d1_speed_scales_jump_rate() {
        // speed=10.0: elapsed 100ms で 100*10=1000ms 蓄積 → D1 でジャンプ
        let mut pb = empty_pb(1_000_000);
        pb.speed = 10.0;
        let mut virtual_elapsed = 0.0_f64;
        let next_t = 1_000_000 + 86_400_000;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            100.0,
            FireStatus::Ready(next_t),
        );
        assert_eq!(result.current_time, Some(next_t));
    }

    #[test]
    fn process_tick_terminal_pauses_and_clears_virtual_elapsed() {
        let mut pb = empty_pb(100_000_000);
        let mut virtual_elapsed = 12_345.0_f64;

        let result = process_tick(&mut pb, &mut virtual_elapsed, 1000.0, FireStatus::Terminal);

        assert!(result.current_time.is_none());
        assert_eq!(pb.status, PlaybackStatus::Paused);
        assert_eq!(pb.current_time, 100_000_000, "current_time は変化しない");
        assert_eq!(virtual_elapsed, 0.0, "terminal で virtual_elapsed はクリア");
    }

    #[test]
    fn process_tick_pending_holds_virtual_elapsed_and_status() {
        // Pending 時は virtual_elapsed を据え置き、status を変更しない
        let mut pb = empty_pb(50_000);
        let mut virtual_elapsed = 30_000.0_f64;

        let result = process_tick(&mut pb, &mut virtual_elapsed, 1000.0, FireStatus::Pending);

        assert!(result.current_time.is_none());
        assert_eq!(pb.status, PlaybackStatus::Playing);
        assert_eq!(pb.current_time, 50_000);
        assert_eq!(
            virtual_elapsed, 30_000.0,
            "Pending では virtual_elapsed を加算しない（据え置き）"
        );
    }

    #[test]
    fn process_tick_drains_trades_when_jumped() {
        // Ready(t) でジャンプが発生した Tick では、current_time 以下の trades を drain する
        let stream = mock_trade_stream();
        let mut buffers = HashMap::new();
        buffers.insert(
            stream,
            TradeBuffer {
                trades: vec![test_trade(10), test_trade(50), test_trade(80)],
                cursor: 0,
            },
        );
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1_000_000,
            current_time: 0,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: buffers,
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
        };
        let mut virtual_elapsed = 0.0_f64;

        let result = process_tick(&mut pb, &mut virtual_elapsed, 100.0, FireStatus::Ready(100));

        assert_eq!(result.current_time, Some(100), "jump to 100");
        let collected = result.trades_collected.expect("drain が実行される");
        assert_eq!(collected.len(), 1);
        let (_, trades, _) = &collected[0];
        assert_eq!(trades.len(), 3, "100 以下の全 trades が drain される");
    }

    #[test]
    fn process_tick_drains_every_tick_even_without_jump() {
        // 穴 A: M1 シナリオで virtual_elapsed < threshold でも drain_until が呼ばれる
        // current_time は変わらないが、蓄積 trades は drain される
        let stream = mock_trade_stream();
        let mut buffers = HashMap::new();
        buffers.insert(
            stream,
            TradeBuffer {
                trades: vec![test_trade(10), test_trade(20), test_trade(200)],
                cursor: 0,
            },
        );
        let mut pb = PlaybackState {
            start_time: 0,
            end_time: 1_000_000,
            current_time: 100,
            status: PlaybackStatus::Playing,
            speed: 1.0,
            trade_buffers: buffers,
            resume_status: PlaybackStatus::Playing,
            pending_trade_streams: HashSet::new(),
        };
        let mut virtual_elapsed = 0.0_f64;

        // delta_to_next = 60_000, elapsed=1000 → 未達だが drain は回る
        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(60_100),
        );

        assert!(result.current_time.is_none(), "ジャンプ無し");
        let collected = result.trades_collected.expect("未達 Tick でも drain される");
        assert_eq!(collected.len(), 1);
        let (_, trades, _) = &collected[0];
        assert_eq!(trades.len(), 2, "current_time(100) 以下の 2 件のみ drain");
    }

    #[test]
    fn process_tick_skips_drain_when_all_buffers_empty() {
        // Tachibana ケース: trade_buffers 全空 → trades_collected=None
        let mut pb = empty_pb(1_000_000);
        let mut virtual_elapsed = 0.0_f64;

        let result = process_tick(
            &mut pb,
            &mut virtual_elapsed,
            1000.0,
            FireStatus::Ready(86_401_000_u64),
        );
        assert!(result.trades_collected.is_none());
    }

    // ── diff_trade_streams: mid-replay での trade stream 差分計算（review 🟡 #4） ──

    fn make_ticker(symbol: &str) -> StreamKind {
        use exchange::adapter::Exchange;
        let ti = exchange::TickerInfo::new(
            exchange::Ticker::new(symbol, Exchange::BinanceSpot),
            1.0,
            1.0,
            None,
        );
        StreamKind::Trades { ticker_info: ti }
    }

    #[test]
    fn diff_trade_streams_detects_new_streams() {
        let mut pb = empty_pb(0);
        let existing = make_ticker("BTCUSDT");
        pb.trade_buffers.insert(
            existing,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );

        let new_stream = make_ticker("ETHUSDT");
        let diff = pb.diff_trade_streams(&[existing, new_stream]);

        assert_eq!(diff.new_streams, vec![new_stream], "ETHUSDT は new");
        assert!(diff.orphan_streams.is_empty(), "orphan なし");
    }

    #[test]
    fn diff_trade_streams_detects_orphan_streams() {
        let mut pb = empty_pb(0);
        let removed = make_ticker("BTCUSDT");
        let kept = make_ticker("ETHUSDT");
        pb.trade_buffers.insert(
            removed,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );
        pb.trade_buffers.insert(
            kept,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );

        // current に BTCUSDT を含めない → orphan
        let diff = pb.diff_trade_streams(&[kept]);

        assert!(diff.new_streams.is_empty(), "new なし");
        assert_eq!(diff.orphan_streams, vec![removed], "BTCUSDT は orphan");
    }

    #[test]
    fn diff_trade_streams_empty_when_no_change() {
        let mut pb = empty_pb(0);
        let a = make_ticker("BTCUSDT");
        let b = make_ticker("ETHUSDT");
        pb.trade_buffers.insert(
            a,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );
        pb.trade_buffers.insert(
            b,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );

        let diff = pb.diff_trade_streams(&[a, b]);

        assert!(diff.new_streams.is_empty());
        assert!(diff.orphan_streams.is_empty());
    }

    #[test]
    fn diff_trade_streams_both_new_and_orphan() {
        let mut pb = empty_pb(0);
        let removed = make_ticker("BTCUSDT");
        pb.trade_buffers.insert(
            removed,
            TradeBuffer { trades: Vec::new(), cursor: 0 },
        );

        let added = make_ticker("ETHUSDT");
        let diff = pb.diff_trade_streams(&[added]);

        assert_eq!(diff.new_streams, vec![added]);
        assert_eq!(diff.orphan_streams, vec![removed]);
    }

    // ── ingest_trades_batch: orphan 自己復活ループ防止（review 🔴 #2） ──

    #[test]
    fn ingest_trades_batch_accepts_for_registered_stream() {
        // 通常経路: 事前に trade_buffers に登録された stream のバッチは追記される
        let stream = mock_trade_stream();
        let mut pb = empty_pb(0);
        pb.trade_buffers.insert(
            stream,
            TradeBuffer {
                trades: Vec::new(),
                cursor: 0,
            },
        );

        let accepted = pb.ingest_trades_batch(stream, vec![test_trade(10), test_trade(20)]);

        assert!(accepted, "登録済み stream は受け入れる");
        assert_eq!(pb.trade_buffers[&stream].trades.len(), 2);
    }

    #[test]
    fn ingest_trades_batch_drops_batch_for_orphan_stream() {
        // orphan 経路: trade_buffers に存在しない stream のバッチは捨てる
        // （SyncReplayBuffers で削除した後に残存 fetch タスクから届くケース）
        let stream = mock_trade_stream();
        let mut pb = empty_pb(0);
        assert!(!pb.trade_buffers.contains_key(&stream));

        let accepted = pb.ingest_trades_batch(stream, vec![test_trade(10), test_trade(20)]);

        assert!(!accepted, "未登録 stream はバッチを拒否する");
        assert!(
            !pb.trade_buffers.contains_key(&stream),
            "拒否された stream は trade_buffers に再出現しない（or_insert_with 経路を通さない）"
        );
    }

    #[test]
    fn ingest_trades_batch_preserves_cursor_on_accepted_append() {
        // cursor が途中にあっても、append は既存 cursor に影響しない
        let stream = mock_trade_stream();
        let mut pb = empty_pb(0);
        pb.trade_buffers.insert(
            stream,
            TradeBuffer {
                trades: vec![test_trade(1), test_trade(2)],
                cursor: 1,
            },
        );

        let accepted = pb.ingest_trades_batch(stream, vec![test_trade(3), test_trade(4)]);

        assert!(accepted);
        assert_eq!(pb.trade_buffers[&stream].trades.len(), 4);
        assert_eq!(pb.trade_buffers[&stream].cursor, 1, "cursor は変化しない");
    }
}
