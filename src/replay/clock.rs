use std::ops::Range;
use std::time::{Duration, Instant};

/// 1x speed での wall delay (ms/bar)
pub const BASE_STEP_DELAY_MS: u64 = 100;

/// 瞬間再生速度（for ループそのまま、1 tick で range.end まで完走）。
#[allow(dead_code)]
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
    /// true のとき、終端到達時に range.start へシークしてから停止する。
    /// Playing 中に StepForward が押されたとき設定される。一度発火したら false に戻す。
    seek_to_start_on_end: bool,
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
            seek_to_start_on_end: false,
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

    /// range.end を step 分延長する。
    #[allow(dead_code)]
    pub fn extend_range_end(&mut self, step: u64) {
        self.range.end = self.range.end.saturating_add(step);
    }

    /// true のとき、終端到達時に range.start へシークしてから停止する。
    #[allow(dead_code)]
    pub fn set_seek_to_start_on_end(&mut self, v: bool) {
        self.seek_to_start_on_end = v;
    }

    /// Playing に遷移する。次のステップは wall_now + step_delay 後に発火する。
    pub fn play(&mut self, wall_now: Instant) {
        self.status = ClockStatus::Playing;
        let delay_ms = self.step_delay_ms();
        self.next_step_at = Some(wall_now + Duration::from_millis(delay_ms));
    }

    /// Paused に遷移する。
    /// `seek_to_start_on_end` フラグも同時にクリアする（StepBackward や明示的 Pause で
    /// 保留中の「終端到達 → start へ戻す」動作をキャンセルするため）。
    pub fn pause(&mut self) {
        self.status = ClockStatus::Paused;
        self.next_step_at = None;
        self.seek_to_start_on_end = false;
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
            let new_now = self
                .now_ms
                .saturating_add(self.step_size_ms)
                .min(self.range.end);
            self.now_ms = new_now;
            next_step += Duration::from_millis(step_delay_ms);

            if self.now_ms >= self.range.end {
                if self.seek_to_start_on_end {
                    self.now_ms = self.range.start;
                    self.seek_to_start_on_end = false;
                }
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

    // ── extend_range_end ──────────────────────────────────────────────────────

    #[test]
    fn extend_range_end_increases_range_end() {
        let mut clock = StepClock::new(0, 1_000, 1_000);
        clock.extend_range_end(1_000);
        assert_eq!(clock.full_range().end, 2_000);
    }

    #[test]
    fn extend_range_end_multiple_calls_accumulate() {
        let mut clock = StepClock::new(0, 1_000, 1_000);
        clock.extend_range_end(500);
        clock.extend_range_end(500);
        assert_eq!(clock.full_range().end, 2_000);
    }

    #[test]
    fn extend_range_end_does_not_affect_playing_status() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        let base = Instant::now();
        clock.play(base);
        clock.extend_range_end(60_000);
        assert_eq!(clock.status(), ClockStatus::Playing);
    }

    #[test]
    fn extend_range_end_play_continues_past_original_end() {
        // original end = 2_000, extended to 3_000
        // 2nd tick (now = 2_000) must still be Playing — old boundary no longer applies
        let mut clock = StepClock::new(0, 2_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.extend_range_end(1_000); // new end = 3_000

        clock.tick(t(base, 100)); // step 1: now = 1_000
        clock.tick(t(base, 200)); // step 2: now = 2_000 (old end, not new)
        assert_eq!(clock.status(), ClockStatus::Playing);
        assert_eq!(clock.now_ms(), 2_000);
    }

    #[test]
    fn extend_range_end_clock_pauses_at_new_end() {
        // extended end = 3_000 → clock must pause at 3_000, not 2_000
        let mut clock = StepClock::new(0, 2_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.extend_range_end(1_000); // new end = 3_000

        clock.tick(t(base, 10_000)); // catch-up: crosses 3_000
        assert_eq!(clock.now_ms(), 3_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    // ── seek_to_start_on_end ─────────────────────────────────────────────────

    #[test]
    fn seek_to_start_on_end_resets_now_ms_to_range_start() {
        let mut clock = StepClock::new(0, 2_000, 1_000);
        clock.set_seek_to_start_on_end(true);
        let base = Instant::now();
        clock.play(base);

        clock.tick(t(base, 10_000)); // catch-up → reaches 2_000 → reset to 0
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn seek_to_start_on_end_pauses_clock_after_reset() {
        let mut clock = StepClock::new(0, 2_000, 1_000);
        clock.set_seek_to_start_on_end(true);
        let base = Instant::now();
        clock.play(base);

        clock.tick(t(base, 10_000));
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn seek_to_start_on_end_with_nonzero_range_start() {
        // range.start が 0 以外でも range.start に戻ること
        let mut clock = StepClock::new(500_000, 502_000, 1_000);
        clock.set_seek_to_start_on_end(true);
        let base = Instant::now();
        clock.play(base);

        clock.tick(t(base, 10_000));
        assert_eq!(clock.now_ms(), 500_000); // range.start
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn seek_to_start_on_end_flag_cleared_after_firing() {
        // 発火後フラグはリセットされる → 再 play で終端到達時に range.start に戻らない
        let mut clock = StepClock::new(0, 2_000, 1_000);
        clock.set_seek_to_start_on_end(true);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 10_000)); // fires: now_ms = 0, Paused

        // 再 play → 終端到達
        clock.play(base);
        clock.tick(t(base, 10_000));
        // フラグが消えているので now_ms = range.end (2_000)、range.start(0) に戻らない
        assert_eq!(clock.now_ms(), 2_000);
    }

    #[test]
    fn seek_to_start_on_end_false_keeps_now_ms_at_range_end() {
        // デフォルト (false): 終端到達で range.end に留まって停止
        let mut clock = StepClock::new(0, 2_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        clock.tick(t(base, 10_000));
        assert_eq!(clock.now_ms(), 2_000);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn seek_to_start_on_end_subsequent_ticks_return_empty_after_reset() {
        // リセット後は Paused なので tick は空レンジを返す
        let mut clock = StepClock::new(0, 1_000, 1_000);
        clock.set_seek_to_start_on_end(true);
        let base = Instant::now();
        clock.play(base);
        clock.tick(t(base, 10_000)); // fires: reset to 0

        let range = clock.tick(t(base, 20_000)); // Paused → empty
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 0);
    }

    // ── extend + seek_to_start_on_end の組み合わせ（Playing StepForward シナリオ）──

    #[test]
    fn extend_then_seek_to_start_plays_through_extended_range_then_resets() {
        // range: 0..2_000. extend → 0..3_000. play through 3_000 → reset to 0.
        let mut clock = StepClock::new(0, 2_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        clock.extend_range_end(1_000); // new end = 3_000
        clock.set_seek_to_start_on_end(true);

        clock.tick(t(base, 200)); // 2 steps: now = 2_000 — still Playing
        assert_eq!(clock.status(), ClockStatus::Playing);
        assert_eq!(clock.now_ms(), 2_000);

        clock.tick(t(base, 10_000)); // reaches 3_000 → seek to start
        assert_eq!(clock.now_ms(), 0);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn extend_multiple_steps_then_seek_to_start_plays_all_extended_bars() {
        // 3 回 StepForward → extend by 3 steps → seek_to_start_on_end
        let mut clock = StepClock::new(0, 1_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        clock.extend_range_end(1_000); // 2_000
        clock.extend_range_end(1_000); // 3_000
        clock.extend_range_end(1_000); // 4_000
        clock.set_seek_to_start_on_end(true);

        clock.tick(t(base, 300)); // 3 steps: now = 3_000 — not yet at 4_000
        assert_eq!(clock.status(), ClockStatus::Playing);

        clock.tick(t(base, 10_000)); // reaches 4_000 → reset
        assert_eq!(clock.now_ms(), 0);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    // ── pause() が seek_to_start_on_end フラグをクリアする ────────────────────

    #[test]
    fn pause_clears_seek_to_start_on_end_flag() {
        // StepForward(Playing) → StepBackward(Playing=pause) → Resume のシナリオ
        // pause() でフラグがクリアされ、Resume 後の終端到達でリセットされない
        let mut clock = StepClock::new(0, 5_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.extend_range_end(1_000); // new end = 6_000
        clock.set_seek_to_start_on_end(true);

        // StepBackward が pause() を呼ぶ
        clock.pause();

        // Resume
        clock.play(base);
        clock.tick(t(base, 10_000)); // reaches 6_000

        // フラグがクリアされていれば now_ms = 6_000、残っていれば 0（誤動作）
        assert_eq!(
            clock.now_ms(),
            6_000,
            "pause() must clear seek_to_start_on_end to prevent stale reset"
        );
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn pause_after_set_flag_then_replay_stays_at_range_end() {
        // フラグ設定後 Pause → 再 Play → 終端で停止（range.start に戻らない）
        let mut clock = StepClock::new(0, 2_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        clock.set_seek_to_start_on_end(true);
        clock.pause(); // clears the flag

        clock.play(base);
        clock.tick(t(base, 10_000));
        assert_eq!(clock.now_ms(), 2_000); // stays at end
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
