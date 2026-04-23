use std::ops::Range;

/// バーステップ離散クロック。
///
/// ADR-0001 §2 自動再生機構の全廃に伴い、`StepClock` は純粋な
/// 「bar 境界を管理する now_ms カウンタ」に縮退した。旧 `speed` /
/// `ClockStatus::{Playing, Paused, Waiting}` / `pause` / `play` /
/// `resume_from_waiting` / `set_speed` は全て削除。
/// 進行は agent session API から明示的に `tick_until` / `seek` を呼び出す。
///
/// Loading 中か否かは呼び出し側の `ReplaySession::{Loading, Active}` enum で判断する
/// （`StepClock` 自身は状態を持たない）。
#[derive(Debug)]
pub struct StepClock {
    /// 現在の仮想時刻 (Unix ms)。常に bar 境界値。
    now_ms: u64,
    /// 1 ステップで進める仮想時刻幅（min active timeframe ms）。
    step_size_ms: u64,
    /// リプレイ範囲 (Unix ms)
    range: Range<u64>,
}

impl StepClock {
    /// 新しい StepClock を作成する。`now_ms` は `start_ms`。
    pub fn new(start_ms: u64, end_ms: u64, step_size_ms: u64) -> Self {
        Self {
            now_ms: start_ms,
            step_size_ms,
            range: start_ms..end_ms,
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    pub fn step_size_ms(&self) -> u64 {
        self.step_size_ms
    }

    pub fn full_range(&self) -> Range<u64> {
        self.range.clone()
    }

    /// `range.end` に到達済みか。
    pub fn reached_end(&self) -> bool {
        self.now_ms >= self.range.end
    }

    /// now_ms を指定値に設定する（step forward/back で使用）。
    /// 範囲をクランプし、step_size_ms の倍数（range.start 基準）へ floor スナップする。
    /// range.end 丁度へのクランプはスナップしない（終端状態として有効）。
    pub fn seek(&mut self, target_ms: u64) {
        let clamped = target_ms.clamp(self.range.start, self.range.end);
        if clamped == self.range.end {
            self.now_ms = clamped;
            return;
        }
        let offset = clamped.saturating_sub(self.range.start);
        let snapped_offset = offset
            .checked_div(self.step_size_ms)
            .map(|q| q * self.step_size_ms)
            .unwrap_or(offset);
        self.now_ms = self.range.start + snapped_offset;
    }

    /// active streams が変わったとき呼ぶ（最小 timeframe が変わる可能性）。
    /// now_ms を新 step_size の倍数（range.start 基準）へ floor 再整列する。
    pub fn set_step_size(&mut self, step_size_ms: u64) {
        self.step_size_ms = step_size_ms;
        let offset = self.now_ms.saturating_sub(self.range.start);
        let aligned_offset = offset
            .checked_div(step_size_ms)
            .map(|q| q * step_size_ms)
            .unwrap_or(offset);
        self.now_ms = self.range.start + aligned_offset;
    }

    /// `target_ms` まで now_ms を前進させ、進行区間を `prev..new` の Range で返す。
    ///
    /// - `target_ms` は `step_size_ms` の倍数（range.start 基準）へ floor スナップされる
    /// - `range.end` を越える target は clamp される
    /// - 既に `target_ms` 以上なら空 range を返し、進行しない
    ///
    /// agent session `advance` ハンドラから呼ばれる。`step` 1 bar 前進は
    /// `tick_until(now_ms + step_size_ms)` と等価。
    pub fn tick_until(&mut self, target_ms: u64) -> Range<u64> {
        let clamped = target_ms.min(self.range.end);
        if clamped <= self.now_ms {
            return self.now_ms..self.now_ms;
        }
        let offset = clamped.saturating_sub(self.range.start);
        // range.end 丁度は step_size_ms の倍数でない可能性があるため snap しない
        let snapped = if clamped == self.range.end {
            clamped
        } else {
            self.range.start
                + offset
                    .checked_div(self.step_size_ms)
                    .map(|q| q * self.step_size_ms)
                    .unwrap_or(offset)
        };
        let prev = self.now_ms;
        self.now_ms = snapped.max(prev);
        prev..self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 基本状態 ──────────────────────────────────────────────────────────────

    #[test]
    fn new_clock_starts_at_start_time() {
        let clock = StepClock::new(1_000_000, 2_000_000, 60_000);
        assert_eq!(clock.now_ms(), 1_000_000);
        assert!(!clock.reached_end());
    }

    #[test]
    fn reached_end_true_at_range_end() {
        let mut clock = StepClock::new(0, 100_000, 60_000);
        clock.seek(100_000);
        assert!(clock.reached_end());
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
        clock.seek(120_000);
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
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.seek(180_000);
        assert_eq!(clock.now_ms(), 180_000);

        clock.set_step_size(300_000);
        // 180_000 は 300_000 の倍数でない → floor → 0
        assert_eq!(clock.now_ms(), 0);
    }

    #[test]
    fn set_step_size_shrink_direction_stays_aligned() {
        let mut clock = StepClock::new(0, 1_000_000, 300_000);
        clock.seek(300_000);
        assert_eq!(clock.now_ms(), 300_000);

        clock.set_step_size(60_000);
        assert_eq!(clock.now_ms(), 300_000);
    }

    // ── tick_until ────────────────────────────────────────────────────────────

    #[test]
    fn tick_until_advances_single_step() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        let range = clock.tick_until(60_000);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 60_000);
        assert_eq!(clock.now_ms(), 60_000);
    }

    #[test]
    fn tick_until_snaps_target_to_bar_boundary() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        let range = clock.tick_until(90_000); // between bars → snaps down to 60_000
        assert_eq!(range.end, 60_000);
        assert_eq!(clock.now_ms(), 60_000);
    }

    #[test]
    fn tick_until_clamps_at_range_end() {
        let mut clock = StepClock::new(0, 100_000, 60_000);
        let range = clock.tick_until(999_999);
        assert_eq!(range.end, 100_000);
        assert_eq!(clock.now_ms(), 100_000);
        assert!(clock.reached_end());
    }

    #[test]
    fn tick_until_past_is_noop() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.seek(120_000);
        let range = clock.tick_until(60_000); // target past
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 120_000);
    }

    #[test]
    fn tick_until_current_is_noop() {
        let mut clock = StepClock::new(0, 1_000_000, 60_000);
        clock.seek(120_000);
        let range = clock.tick_until(120_000);
        assert!(range.is_empty());
        assert_eq!(clock.now_ms(), 120_000);
    }

    #[test]
    fn tick_until_multiple_steps_in_one_call() {
        let mut clock = StepClock::new(0, 1_000_000, 1_000);
        let range = clock.tick_until(3_000);
        assert_eq!(range.start, 0);
        assert_eq!(range.end, 3_000);
        assert_eq!(clock.now_ms(), 3_000);
    }
}
