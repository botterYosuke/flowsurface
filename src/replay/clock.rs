use std::ops::Range;
use std::time::{Duration, Instant};

/// リプレイ再生中の単方向仮想時刻。
/// wall elapsed × speed を積算するだけ。pause 時は `anchor_wall` を update せず holds。
pub struct VirtualClock {
    /// 仮想時刻 (Unix ms)。Play 開始時 = replay.start_ms、Pause 時は固定。
    now_ms: u64,
    /// 最後に now_ms を更新した実時刻。Pause 中は None。
    anchor_wall: Option<Instant>,
    /// 再生速度 (1.0 = 等倍)。
    speed: f32,
    /// 再生状態。
    status: ClockStatus,
    /// リプレイ範囲 (Unix ms)
    range: Range<u64>,
    /// D1 バーステップモード設定 (bar_interval_ms, wall_delay_ms)
    bar_step_mode: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockStatus {
    Paused,
    Playing,
    /// EventStore が range loading 中。now_ms を進めない。
    Waiting,
}

impl VirtualClock {
    /// 新しい VirtualClock を作成する。初期状態は Paused。
    pub fn new(start_ms: u64, end_ms: u64) -> Self {
        Self {
            now_ms: start_ms,
            anchor_wall: None,
            speed: 1.0,
            status: ClockStatus::Paused,
            range: start_ms..end_ms,
            bar_step_mode: None,
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn status(&self) -> ClockStatus {
        self.status
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn full_range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// Playing に遷移する。anchor_wall を wall_now に設定。
    pub fn play(&mut self, wall_now: Instant) {
        self.status = ClockStatus::Playing;
        self.anchor_wall = Some(wall_now);
    }

    /// Paused に遷移する。
    pub fn pause(&mut self) {
        self.status = ClockStatus::Paused;
        self.anchor_wall = None;
    }

    /// now_ms を指定値に設定する（step forward/back で使用）。
    /// 範囲を超えた値はクランプされる。
    pub fn seek(&mut self, target_ms: u64) {
        self.now_ms = target_ms.clamp(self.range.start, self.range.end);
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
    }

    /// Waiting 状態に落とす（Store 未 load が検出されたとき）。
    /// anchor_wall は None にリセット。冪等: 既に Waiting なら何もしない。
    pub fn set_waiting(&mut self) {
        if self.status != ClockStatus::Waiting {
            self.status = ClockStatus::Waiting;
            self.anchor_wall = None;
        }
    }

    /// Waiting → Playing へ復帰する。EventStore::ingest_loaded 完了時に呼ぶ。
    /// 新しい wall_now を anchor にするため、待機中の実時間経過ぶんは仮想時間に反映されない。
    pub fn resume_from_waiting(&mut self, wall_now: Instant) {
        if self.status == ClockStatus::Waiting {
            self.status = ClockStatus::Playing;
            self.anchor_wall = Some(wall_now);
        }
    }

    /// D1 など大粒度 timeframe で使用する。
    /// 1 バー進めたら次のバー時刻まで wall 1 秒待機。
    #[cfg(test)]
    pub fn enable_bar_step_mode(&mut self, bar_interval_ms: u64, wall_delay_ms: u64) {
        self.bar_step_mode = Some((bar_interval_ms, wall_delay_ms));
    }

    /// 各フレームで呼ぶ。wall elapsed を仮想時刻に変換して now_ms を進め、
    /// (prev_now, current_now) の範囲を返す。Playing 以外は空 range を返す。
    pub fn advance(&mut self, wall_now: Instant) -> Range<u64> {
        if self.status != ClockStatus::Playing {
            return self.now_ms..self.now_ms;
        }
        let anchor = self.anchor_wall.expect("Playing without anchor");

        if let Some((bar_interval_ms, wall_delay_ms)) = self.bar_step_mode {
            return self.advance_bar_step(wall_now, anchor, bar_interval_ms, wall_delay_ms);
        }

        let wall_elapsed = wall_now.duration_since(anchor);
        let virtual_delta =
            (wall_elapsed.as_secs_f64() * self.speed as f64 * 1000.0) as u64;
        let prev = self.now_ms;
        let next = prev.saturating_add(virtual_delta).min(self.range.end);
        self.now_ms = next;
        self.anchor_wall = Some(wall_now);
        if next >= self.range.end {
            self.status = ClockStatus::Paused;
            self.anchor_wall = None;
        }
        prev..next
    }

    fn advance_bar_step(
        &mut self,
        wall_now: Instant,
        anchor: Instant,
        bar_interval_ms: u64,
        wall_delay_ms: u64,
    ) -> Range<u64> {
        let prev = self.now_ms;
        let wall_elapsed_ms = wall_now.duration_since(anchor).as_millis() as u64;
        let bars_to_advance = wall_elapsed_ms / wall_delay_ms;
        if bars_to_advance == 0 {
            return prev..prev;
        }
        let current_bar = self.now_ms / bar_interval_ms;
        let next_bar_time = (current_bar + bars_to_advance) * bar_interval_ms;
        self.now_ms = next_bar_time.min(self.range.end);
        // 消費した wall 時間ぶんだけ anchor を進める（余剰は次フレームに繰り越し）
        self.anchor_wall = Some(anchor + Duration::from_millis(bars_to_advance * wall_delay_ms));
        if self.now_ms >= self.range.end {
            self.status = ClockStatus::Paused;
            self.anchor_wall = None;
        }
        prev..self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instant_plus(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn new_clock_starts_paused_at_start_time() {
        let clock = VirtualClock::new(1_000_000, 2_000_000);
        assert_eq!(clock.now_ms(), 1_000_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn advance_while_paused_returns_empty_range() {
        let mut clock = VirtualClock::new(1_000, 2_000);
        let now = Instant::now();
        let range = clock.advance(now);
        assert!(range.is_empty(), "paused clock should return empty range");
        assert_eq!(clock.now_ms(), 1_000);
    }

    #[test]
    fn play_transitions_to_playing() {
        let mut clock = VirtualClock::new(1_000, 2_000);
        let now = Instant::now();
        clock.play(now);
        assert_eq!(clock.status(), ClockStatus::Playing);
    }

    #[test]
    fn advance_while_playing_advances_now_ms() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);

        // Simulate 1 second elapsed at 1x speed
        let range = clock.advance(make_instant_plus(base, 1_000));
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 1_000);
        assert_eq!(clock.now_ms(), 1_000);
    }

    #[test]
    fn advance_respects_speed_multiplier() {
        let mut clock = VirtualClock::new(0, 100_000);
        clock.set_speed(2.0);
        let base = Instant::now();
        clock.play(base);

        // 1 second wall time at 2x speed → 2000ms virtual
        let range = clock.advance(make_instant_plus(base, 1_000));
        assert_eq!(range.end, 2_000);
    }

    #[test]
    fn advance_clamps_to_range_end() {
        let mut clock = VirtualClock::new(0, 500);
        let base = Instant::now();
        clock.play(base);

        // 2 seconds wall time would normally give 2000ms, clamped to 500
        let range = clock.advance(make_instant_plus(base, 2_000));
        assert_eq!(clock.now_ms(), 500);
        assert_eq!(range.end, 500);
    }

    #[test]
    fn advance_past_end_sets_paused() {
        let mut clock = VirtualClock::new(0, 500);
        let base = Instant::now();
        clock.play(base);

        clock.advance(make_instant_plus(base, 2_000));
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn pause_stops_advance() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);
        clock.advance(make_instant_plus(base, 500));
        let after_500 = clock.now_ms();
        clock.pause();

        // After pause, another advance should return empty range
        let range = clock.advance(make_instant_plus(base, 1_500));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), after_500);
    }

    #[test]
    fn seek_changes_now_ms() {
        let mut clock = VirtualClock::new(0, 100_000);
        clock.seek(50_000);
        assert_eq!(clock.now_ms(), 50_000);
    }

    #[test]
    fn seek_clamps_above_range_end() {
        let mut clock = VirtualClock::new(0, 100_000);
        clock.seek(999_999);
        assert_eq!(clock.now_ms(), 100_000);
    }

    #[test]
    fn seek_clamps_below_range_start() {
        let mut clock = VirtualClock::new(1_000, 100_000);
        clock.seek(0);
        assert_eq!(clock.now_ms(), 1_000);
    }

    #[test]
    fn set_waiting_transitions_to_waiting() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();
        assert_eq!(clock.status(), ClockStatus::Waiting);
    }

    #[test]
    fn advance_while_waiting_returns_empty_range() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();

        let range = clock.advance(make_instant_plus(base, 1_000));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn set_waiting_is_idempotent() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();
        clock.set_waiting(); // second call should be no-op
        assert_eq!(clock.status(), ClockStatus::Waiting);
    }

    #[test]
    fn resume_from_waiting_transitions_to_playing() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();

        clock.resume_from_waiting(make_instant_plus(base, 2_000));
        assert_eq!(clock.status(), ClockStatus::Playing);
    }

    #[test]
    fn resume_from_waiting_resets_anchor_so_wait_time_not_counted() {
        let mut clock = VirtualClock::new(0, 100_000);
        let base = Instant::now();
        clock.play(base);

        // Advance 500ms virtual
        clock.advance(make_instant_plus(base, 500));
        assert_eq!(clock.now_ms(), 500);

        // Wait 5 seconds (Waiting state)
        clock.set_waiting();

        // Resume at base+5500ms — the 5000ms wait should NOT be counted
        let resume_wall = make_instant_plus(base, 5_500);
        clock.resume_from_waiting(resume_wall);

        // Only 100ms wall time passes after resume → 100ms virtual
        let range = clock.advance(make_instant_plus(base, 5_600));
        assert_eq!(range.start, 500);
        assert_eq!(range.end, 600); // Only 100ms added, not 5100ms
    }

    #[test]
    fn bar_step_mode_advances_by_bar_intervals() {
        let mut clock = VirtualClock::new(0, 10 * 86_400_000);
        let base = Instant::now();
        clock.enable_bar_step_mode(86_400_000, 1_000); // D1 bars, 1 second delay
        clock.play(base);

        // 1 second wall elapsed → 1 D1 bar
        let range = clock.advance(make_instant_plus(base, 1_000));
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 86_400_000);
        assert_eq!(clock.now_ms(), 86_400_000);
    }

    #[test]
    fn bar_step_mode_no_advance_before_wall_delay() {
        let mut clock = VirtualClock::new(0, 10 * 86_400_000);
        let base = Instant::now();
        clock.enable_bar_step_mode(86_400_000, 1_000);
        clock.play(base);

        // Only 500ms elapsed → not enough for 1 bar
        let range = clock.advance(make_instant_plus(base, 500));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn bar_step_mode_catchup_multiple_bars() {
        let mut clock = VirtualClock::new(0, 100 * 86_400_000);
        let base = Instant::now();
        clock.enable_bar_step_mode(86_400_000, 1_000);
        clock.play(base);

        // 3 seconds wall elapsed → 3 D1 bars catch-up
        let range = clock.advance(make_instant_plus(base, 3_000));
        assert_eq!(range.end, 3 * 86_400_000);
    }
}
