use std::collections::HashSet;
use std::time::Instant;

use exchange::adapter::StreamKind;
use exchange::{Kline, Trade};

use super::clock::{ClockStatus, StepClock};
use super::store::EventStore;

/// `dispatch_tick` の戻り値。
pub struct DispatchResult {
    /// 現在の仮想時刻
    pub current_time: u64,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    /// true なら replay 終端に到達、clock は Paused へ。
    pub reached_end: bool,
}

impl DispatchResult {
    pub fn empty(current_time: u64) -> Self {
        Self {
            current_time,
            trade_events: vec![],
            kline_events: vec![],
            reached_end: false,
        }
    }
}

/// 各 Tick で呼ばれ、clock を進めて emit すべきイベントを集めて返す。
/// StepClock と EventStore の橋渡しをするステートレスなロジック。
pub fn dispatch_tick(
    clock: &mut StepClock,
    store: &EventStore,
    active_streams: &HashSet<StreamKind>,
    wall_now: Instant,
) -> DispatchResult {
    // 1. 全 active_streams の full replay range が loaded か確認
    // Paused 状態では Waiting に遷移させない — mid-replay で銘柄/timeframe を変更した際に
    // clock が Paused のまま保たれ、ロード完了後の自動再生 (try_resume_from_waiting) を防ぐ。
    let full_range = clock.full_range();
    for stream in active_streams {
        if !store.is_loaded(stream, full_range.clone()) {
            if clock.status() == ClockStatus::Playing {
                clock.set_waiting();
            }
            return DispatchResult::empty(clock.now_ms());
        }
    }

    // clock が Waiting の場合は Waiting のまま空を返す
    // (全stream loaded になったら resume_from_waiting を呼ぶのは Dashboard の責任)
    if clock.status() == ClockStatus::Waiting {
        return DispatchResult::empty(clock.now_ms());
    }

    // 2. clock を 1 ステップ進める
    let range = clock.tick(wall_now);
    if range.is_empty() {
        // 通常の空レンジ (start == end): Paused clock や未発火ステップ — reached_end = false
        // 逆転レンジ (start > end): seek_to_start_on_end 発火時 — clock は Paused になっており
        //   最終ステップの emit はスキップされるが reached_end = true を伝播して Toast を発行する。
        let reached_end = range.start > range.end && clock.status() == ClockStatus::Paused;
        return DispatchResult {
            current_time: clock.now_ms(),
            trade_events: vec![],
            kline_events: vec![],
            reached_end,
        };
    }

    // 3. イベント抽出
    let mut trade_events = vec![];
    let mut kline_events = vec![];
    for stream in active_streams {
        let trades = store.trades_in(stream, range.clone()).to_vec();
        let klines = store.klines_in(stream, range.clone()).to_vec();
        trade_events.push((*stream, trades));
        kline_events.push((*stream, klines));
    }

    DispatchResult {
        current_time: clock.now_ms(),
        trade_events,
        kline_events,
        reached_end: clock.status() == ClockStatus::Paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::Volume;
    use exchange::unit::MinTicksize;
    use exchange::unit::price::Price;
    use exchange::unit::qty::Qty;
    use std::time::Duration;

    fn t(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn dummy_trade(time: u64) -> Trade {
        Trade {
            time,
            is_sell: false,
            price: Price::from_f32_lossy(100.0),
            qty: Qty::from_f32_lossy(1.0),
        }
    }

    fn dummy_kline(time: u64) -> Kline {
        Kline::new(
            time,
            100.0,
            101.0,
            99.0,
            100.5,
            Volume::empty_total(),
            MinTicksize::from(0.01),
        )
    }

    fn trade_stream() -> StreamKind {
        use exchange::adapter::Exchange;
        use exchange::{Ticker, TickerInfo};
        StreamKind::Trades {
            ticker_info: TickerInfo::new(
                Ticker::new("BTCUSDT", Exchange::BinanceLinear),
                0.01,
                0.001,
                Some(1.0),
            ),
        }
    }

    fn make_store_with_data(stream: StreamKind, range: std::ops::Range<u64>) -> EventStore {
        use super::super::store::LoadedData;
        let mut store = EventStore::new();
        let trades = vec![dummy_trade(500), dummy_trade(1_000), dummy_trade(2_000)];
        let klines = vec![dummy_kline(0), dummy_kline(1_000)];
        store.ingest_loaded(stream, range, LoadedData { klines, trades });
        store
    }

    #[test]
    fn dispatch_returns_empty_when_clock_is_paused() {
        let mut clock = StepClock::new(0, 10_000, 1_000);
        // clock は Paused (デフォルト)
        let store = EventStore::new();
        let streams = HashSet::new();
        let base = Instant::now();

        let result = dispatch_tick(&mut clock, &store, &streams, t(base, 1_000));
        assert!(result.trade_events.is_empty());
        assert!(result.kline_events.is_empty());
        assert!(!result.reached_end);
    }

    #[test]
    fn dispatch_sets_waiting_when_store_not_loaded() {
        let mut clock = StepClock::new(0, 10_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        let store = EventStore::new(); // empty store
        let stream = trade_stream();
        let mut streams = HashSet::new();
        streams.insert(stream);

        dispatch_tick(&mut clock, &store, &streams, t(base, 1_000));
        assert_eq!(clock.status(), ClockStatus::Waiting);
    }

    #[test]
    fn dispatch_returns_one_trade_in_half_open_range_when_store_loaded() {
        // step_size=1000, step_delay=BASE_STEP_DELAY_MS(100ms):
        // wall=100ms → exactly 1 step fires → range [0, 1000)
        // trade at 500 → ✓ (included)
        // trade at 1000 → ✗ (end 境界は半開区間で除外)
        let stream = trade_stream();
        let store = make_store_with_data(stream, 0..10_000);
        let mut clock = StepClock::new(0, 10_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        let mut streams = HashSet::new();
        streams.insert(stream);

        let result = dispatch_tick(&mut clock, &store, &streams, t(base, 100));
        assert_eq!(result.current_time, 1_000);

        let (_, trades) = result
            .trade_events
            .iter()
            .find(|(s, _)| *s == stream)
            .unwrap();
        assert_eq!(
            trades.len(),
            1,
            "trade at 500 included; trade at 1000 excluded (half-open)"
        );
        assert_eq!(trades[0].time, 500);
    }

    #[test]
    fn dispatch_catchup_two_steps_returns_events_from_entire_range() {
        // step_size=1000, step_delay=100ms:
        // wall=200ms → exactly 2 steps fire: range [0, 2000)
        // trades at 500, 1000 → both included (2000 is exclusive end)
        let stream = trade_stream();
        let store = make_store_with_data(stream, 0..10_000);
        let mut clock = StepClock::new(0, 10_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        let mut streams = HashSet::new();
        streams.insert(stream);

        let result = dispatch_tick(&mut clock, &store, &streams, t(base, 200));
        assert_eq!(result.current_time, 2_000);

        let (_, trades) = result
            .trade_events
            .iter()
            .find(|(s, _)| *s == stream)
            .unwrap();
        assert_eq!(trades.len(), 2, "trades at 500 and 1000 both in [0, 2000)");
    }

    #[test]
    fn dispatch_sets_reached_end_when_range_end_reached() {
        let stream = trade_stream();
        let store = make_store_with_data(stream, 0..3_000);
        let mut clock = StepClock::new(0, 3_000, 1_000);
        let base = Instant::now();
        clock.play(base);

        let mut streams = HashSet::new();
        streams.insert(stream);

        // 5s wall → 3 steps → 0→1000→2000→3000 = range.end → Paused
        let result = dispatch_tick(&mut clock, &store, &streams, t(base, 5_000));
        assert!(result.reached_end);
        assert_eq!(clock.status(), ClockStatus::Paused);
    }

    #[test]
    fn dispatch_empty_streams_just_advances_clock() {
        let mut clock = StepClock::new(0, 10_000, 1_000);
        let base = Instant::now();
        clock.play(base);
        let store = EventStore::new();
        let streams = HashSet::new(); // no streams

        // step_delay=100ms → 1 step fires at wall=100ms: now_ms = 1000
        let result = dispatch_tick(&mut clock, &store, &streams, t(base, 100));
        assert_eq!(result.current_time, 1_000);
        assert!(result.trade_events.is_empty());
        assert!(!result.reached_end);
    }
}
