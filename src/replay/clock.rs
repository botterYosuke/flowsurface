use std::ops::Range;
use std::time::{Duration, Instant};

/// 1x speed での wall delay (ms/bar)
pub const BASE_STEP_DELAY_MS: u64 = 100;

/// 瞬間再生速度（for ループそのまま、1 tick で range.end まで完走）。
pub const SPEED_INSTANT: f32 = f32::INFINITY;

/// バーステップ離散クロック。
/// `tick()` を各フレームで呼び、発火タイミングなら 1 ステップ進めて emit range を返す。
pub struct StepClock {
    /// 現在の仮想時刻 (Unix ms)。常にバー境界値。
    now_ms: u64,
    /// 次のステップを発火する wall 時刻。Playing 以外は None。
    next_step_at: Option<Instant>,
    /// 1 ステップで進める仮想時刻幅（min active timeframe ms）。
    step_size_ms: u64,
    /// 1 ステップあたりの wall delay 基準値 (1x speed, ms)。
    base_step_delay_ms: u64,
    /// 再生速度倍率。
    speed: f32,
    /// 再生状態。
    status: ClockStatus,
    /// リプレイ範囲 (Unix ms)
    range: Range<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockStatus {
    Paused,
    Playing,
    /// EventStore が range loading 中。now_ms を進めない。
    Waiting,
}

impl StepClock {
    /// 新しい StepClock を作成する。初期状態は Paused。
    pub fn new(start_ms: u64, end_ms: u64, step_size_ms: u64) -> Self {
        Self {
            now_ms: start_ms,
            next_step_at: None,
            step_size_ms,
            base_step_delay_ms: BASE_STEP_DELAY_MS,
            speed: 1.0,
            status: ClockStatus::Paused,
            range: start_ms..end_ms,
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

    /// Playing に遷移する。次のステップは wall_now + step_delay 後に発火する。
    pub fn play(&mut self, wall_now: Instant) {
        self.status = ClockStatus::Playing;
        let delay_ms = self.step_delay_ms();
        self.next_step_at = Some(wall_now + Duration::from_millis(delay_ms));
    }

    /// Paused に遷移する。
    pub fn pause(&mut self) {
        self.status = ClockStatus::Paused;
        self.next_step_at = None;
    }

    /// now_ms を指定値に設定する（step forward/back で使用）。
    /// 範囲をクランプし、step_size_ms の倍数（range.start 基準）へ floor スナップする。
    /// range.end 丁度へのクランプはスナップしない（終端状態として有効）。
    pub fn seek(&mut self, target_ms: u64) {
        let clamped = target_ms.clamp(self.range.start, self.range.end);
        // range.end へのクランプはそのまま（終端境界はスナップ不要）
        if clamped == self.range.end {
            self.now_ms = clamped;
            return;
        }
        let offset = clamped.saturating_sub(self.range.start);
        let snapped_offset = if self.step_size_ms > 0 {
            (offset / self.step_size_ms) * self.step_size_ms
        } else {
            offset
        };
        self.now_ms = self.range.start + snapped_offset;
    }

    /// speed を更新する。0 以下は Pause と同義（speed は変更しない）。
    pub fn set_speed(&mut self, speed: f32) {
        if speed <= 0.0 {
            self.pause();
            return;
        }
        self.speed = speed;
    }

    /// active streams が変わったとき呼ぶ（最小 timeframe が変わる可能性）。
    /// now_ms を新 step_size の倍数（range.start 基準）へ floor 再整列する。
    pub fn set_step_size(&mut self, step_size_ms: u64) {
        self.step_size_ms = step_size_ms;
        let offset = self.now_ms.saturating_sub(self.range.start);
        let aligned_offset = if step_size_ms > 0 {
            (offset / step_size_ms) * step_size_ms
        } else {
            offset
        };
        self.now_ms = self.range.start + aligned_offset;
    }

    /// Waiting 状態に落とす（Store 未 load が検出されたとき）。
    /// 冪等: 既に Waiting なら何もしない。
    pub fn set_waiting(&mut self) {
        if self.status != ClockStatus::Waiting {
            self.status = ClockStatus::Waiting;
            self.next_step_at = None;
        }
    }

    /// Waiting → Playing へ復帰する。EventStore::ingest_loaded 完了時に呼ぶ。
    /// 待機中の実時間経過分は仮想時間に反映されない。
    pub fn resume_from_waiting(&mut self, wall_now: Instant) {
        if self.status == ClockStatus::Waiting {
            self.status = ClockStatus::Playing;
            let delay_ms = self.step_delay_ms();
            self.next_step_at = Some(wall_now + Duration::from_millis(delay_ms));
        }
    }

    /// 各フレームで呼ぶ。
    /// 発火タイミングなら 1 ステップ（または catch-up で複数ステップ）進めて emit range を返す。
    /// Playing 以外、または発火タイミング未到達の場合は空 range を返す。
    /// `SPEED_INSTANT` 時は 1 tick で `range.end` まで一気に完走し Paused に遷移する。
    pub fn tick(&mut self, wall_now: Instant) -> Range<u64> {
        if self.status != ClockStatus::Playing {
            return self.now_ms..self.now_ms;
        }

        let step_delay_ms = self.step_delay_ms();

        // Instant mode (SPEED_INSTANT): wall 時刻によらず 1 tick で range.end へ
        if step_delay_ms == 0 {
            let prev = self.now_ms;
            self.now_ms = self.range.end;
            self.status = ClockStatus::Paused;
            self.next_step_at = None;
            return prev..self.now_ms;
        }

        let mut next_step = self.next_step_at.expect("Playing without next_step_at");

        if wall_now < next_step {
            return self.now_ms..self.now_ms;
        }

        let prev = self.now_ms;

        // Catch-up ループ: 溜まったステップをまとめて emit
        while self.status == ClockStatus::Playing && wall_now >= next_step {
            let new_now = self.now_ms.saturating_add(self.step_size_ms).min(self.range.end);
            self.now_ms = new_now;
            next_step = next_step + Duration::from_millis(step_delay_ms);

            if self.now_ms >= self.range.end {
                self.status = ClockStatus::Paused;
                self.next_step_at = None;
                break;
            }
        }

        if self.status == ClockStatus::Playing {
            self.next_step_at = Some(next_step);
        }

        prev..self.now_ms
    }

    fn step_delay_ms(&self) -> u64 {
        // speed > 0 保証は set_speed で担保
        (self.base_step_delay_ms as f64 / self.speed as f64) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    // ── 基本状態 ──────────────────────────────────────────────────────────────

    #[test]
    fn new_clock_starts_paused_at_start_time() {
        let clock = StepClock::new(1_000_000, 2_000_000, 60_000);
        assert_eq!(clock.now_ms(), 1_000_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn tick_while_paused_returns_empty_range() {
        let mut clock = StepClock::new(1_000, 2_000, 1_000);
        let now = Instant::now();
        let range = clock.tick(now);
        assert!(range.is_empty(), "paused clock should return empty range");
        assert_eq!(clock.now_ms(), 1_000);
    }

    #[test]
    fn play_transitions_to_playing() {
        let mut clock = StepClock::new(1_000, 2_000, 1_000);
        let now = Instant::now();
        clock.play(now);
        assert_eq!(clock.status(), ClockStatus::Playing);
    }

    // ── tick 発火タイミング ────────────────────────────────────────────────────

    #[test]
    fn tick_before_step_delay_returns_empty_range() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        // 99ms 後はまだ発火しない（delay = 100ms）
        let range = clock.tick(t(base, 99));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn tick_emits_one_step_at_step_delay() {
        let mut clock = StepClock::new(0, 100_000, 60_000);
        let base = Instant::now();
        clock.play(base);

        // 100ms 後に発火（1x speed = 100ms/step）
        let range = clock.tick(t(base, 100));
        assert!(!range.is_empty());
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 60_000);
        assert_eq!(clock.now_ms(), 60_000);
    }

    #[test]
    fn tick_advances_step_size_per_fire() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        let base = Instant::now();
        clock.play(base);

        // 2 ステップ発火: 0 → 60_000 → 120_000
        clock.tick(t(base, 100)); // 1st step
        clock.tick(t(base, 200)); // 2nd step
        assert_eq!(clock.now_ms(), 120_000);
    }

    // ── speed ────────────────────────────────────────────────────────────────

    #[test]
    fn set_speed_2x_halves_step_delay() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.set_speed(2.0);
        let base = Instant::now();
        clock.play(base);

        // 2x speed → delay = 50ms/step
        let range = clock.tick(t(base, 50));
        assert!(!range.is_empty(), "2x speed should fire at 50ms");
        assert_eq!(clock.now_ms(), 60_000);
    }

    #[test]
    fn set_speed_zero_pauses_clock() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_speed(0.0);
        assert_eq!(clock.status(), ClockStatus::Paused);

        // 以後 tick しても空 range
        let range = clock.tick(t(base, 5_000));
        assert!(range.is_empty());
    }

    // ── catch-up ─────────────────────────────────────────────────────────────

    #[test]
    fn multiple_ticks_catchup_in_one_frame() {
        let mut clock = StepClock::new(0, 1_000_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        // 300ms wall → 3 ステップ catch-up: 0 → 1000 → 2000 → 3000
        let range = clock.tick(t(base, 300));
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 3_000);
        assert_eq!(clock.now_ms(), 3_000);
    }

    #[test]
    fn catchup_clamps_at_range_end_and_pauses() {
        let mut clock = StepClock::new(0, 2_500, 1_000);
        let base = Instant::now();
        clock.play(base);

        // 10s wall → range.end=2500 でクランプ
        let range = clock.tick(t(base, 10_000));
        // 0→1000→2000→2500 (min clamp)
        assert_eq!(clock.now_ms(), 2_500);
        assert_eq!(range.end, 2_500);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    // ── pause ─────────────────────────────────────────────────────────────────

    #[test]
    fn pause_stops_tick() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 1_000)); // 1st step
        let after_step = clock.now_ms();
        clock.pause();

        let range = clock.tick(t(base, 5_000));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), after_step);
    }

    // ── seek ──────────────────────────────────────────────────────────────────

    #[test]
    fn seek_snaps_to_bar_boundary_floor() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.seek(90_000); // between 60_000 and 120_000 → snaps to 60_000
        assert_eq!(clock.now_ms(), 60_000);
    }

    #[test]
    fn seek_snaps_exactly_on_boundary() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.seek(120_000); // exactly on boundary → stays 120_000
        assert_eq!(clock.now_ms(), 120_000);
    }

    #[test]
    fn seek_clamps_above_range_end() {
        let mut clock = StepClock::new(0, 100_000, 60_000);
        clock.seek(999_999);
        assert_eq!(clock.now_ms(), 100_000);
    }

    #[test]
    fn seek_clamps_below_range_start() {
        let mut clock = StepClock::new(60_000, 200_000, 60_000);
        clock.seek(0);
        assert_eq!(clock.now_ms(), 60_000);
    }

    // ── set_step_size ─────────────────────────────────────────────────────────

    #[test]
    fn set_step_size_floor_realigns_now_ms_on_expansion() {
        // now_ms = 3m (180_000), step_size 拡大 1m→5m
        // offset = 180_000, aligned = (180_000 / 300_000) * 300_000 = 0
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 300)); // 3 steps: 0→60_000→120_000→180_000
        assert_eq!(clock.now_ms(), 180_000);

        clock.set_step_size(300_000); // 5m
        // 180_000 は 300_000 の倍数でない → floor → 0
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn set_step_size_shrink_direction_stays_aligned() {
        // now_ms = 5m (300_000), step_size 縮小 5m→1m
        let mut clock = StepClock::new(0, 1_000_000, 300_000);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 100)); // 1 step: 0→300_000
        assert_eq!(clock.now_ms(), 300_000);

        clock.set_step_size(60_000); // 1m
        // 300_000 は 60_000 の倍数 → 変わらず
        assert_eq!(clock.now_ms(), 300_000);
    }

    // ── instant speed ────────────────────────────────────────────────────────

    #[test]
    fn instant_speed_completes_in_one_tick() {
        let mut clock = StepClock::new(0, 300_000, 60_000);
        clock.set_speed(SPEED_INSTANT);
        let base = Instant::now();
        clock.play(base);

        // wall 時刻によらず 1 tick で完走
        let range = clock.tick(base);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 300_000);
        assert_eq!(clock.now_ms(), 300_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn instant_speed_from_mid_range_jumps_to_end() {
        let mut clock = StepClock::new(0, 300_000, 60_000);
        clock.seek(120_000);
        clock.set_speed(SPEED_INSTANT);
        let base = Instant::now();
        clock.play(base);

        let range = clock.tick(base);
        assert_eq!(range.start, 120_000);
        assert_eq!(range.end, 300_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn after_instant_tick_further_ticks_return_empty() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        clock.set_speed(SPEED_INSTANT);
        let base = Instant::now();
        clock.play(base);
        clock.tick(base); // completes instantly

        let range = clock.tick(t(base, 1_000));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 100_000);
    }

    // ── waiting ───────────────────────────────────────────────────────────────

    #[test]
    fn set_waiting_transitions_to_waiting() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();
        assert_eq!(clock.status(), ClockStatus::Waiting);
    }

    #[test]
    fn tick_while_waiting_returns_empty_range() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();

        let range = clock.tick(t(base, 5_000));
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn set_waiting_is_idempotent() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();
        clock.set_waiting(); // 2回目は no-op
        assert_eq!(clock.status(), ClockStatus::Waiting);
    }

    #[test]
    fn resume_from_waiting_transitions_to_playing() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_waiting();
        clock.resume_from_waiting(t(base, 2_000));
        assert_eq!(clock.status(), ClockStatus::Playing);
    }

    #[test]
    fn resume_from_waiting_resets_step_timer_so_wait_not_counted() {
        let mut clock = StepClock::new(0, 100_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 100)); // 1st step: now_ms = 1000
        assert_eq!(clock.now_ms(), 1_000);

        clock.set_waiting();

        // 5000ms 待機後に resume
        let resume_wall = t(base, 6_000);
        clock.resume_from_waiting(resume_wall);

        // resume から 100ms 後に次のステップが発火するはず
        let range = clock.tick(t(base, 6_099)); // 99ms 後 → まだ発火しない
        assert!(range.is_empty());

        let range = clock.tick(t(base, 6_100)); // 100ms 後 → 発火
        assert!(!range.is_empty());
        assert_eq!(clock.now_ms(), 2_000);
    }
}
