use std::collections::HashSet;

use exchange::adapter::StreamKind;
use exchange::{Kline, Trade};

use super::clock::StepClock;
use super::store::EventStore;

/// `dispatch_tick` の戻り値。
pub struct DispatchResult {
    /// 現在の仮想時刻
    pub current_time: u64,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    /// true なら replay 終端に到達。
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
    target_ms: u64,
) -> DispatchResult {
    // 1. 全 active_streams の full replay range が loaded か確認
    let full_range = clock.full_range();
    for stream in active_streams {
        if !store.is_loaded(stream, full_range.clone()) {
            return DispatchResult::empty(clock.now_ms());
        }
    }

    // 2. clock を指定時刻まで進める
    let range = clock.tick_until(target_ms);
    if range.is_empty() {
        return DispatchResult::empty(clock.now_ms());
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
        reached_end: clock.reached_end(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::testutil::{dummy_kline, dummy_trade, trade_stream};

    fn make_store_with_data(stream: StreamKind, range: std::ops::Range<u64>) -> EventStore {
        use super::super::store::LoadedData;
        let mut store = EventStore::new();
        let trades = vec![dummy_trade(500), dummy_trade(1_000), dummy_trade(2_000)];
        let klines = vec![dummy_kline(0), dummy_kline(1_000)];
        store.ingest_loaded(stream, range, LoadedData { klines, trades });
        store
    }

    #[test]
    fn dispatch_returns_empty_when_target_past() {
        let mut clock = StepClock::new(0, 10_000, 1_000);
        clock.seek(1_000);
        let store = EventStore::new();
        let streams = HashSet::new();

        let result = dispatch_tick(&mut clock, &store, &streams, 500);
        assert!(result.trade_events.is_empty());
        assert!(result.kline_events.is_empty());
        assert!(!result.reached_end);
    }

    #[test]
    fn dispatch_returns_empty_when_store_not_loaded() {
        let mut clock = StepClock::new(0, 10_000, 1_000);
        let store = EventStore::new(); // empty store
        let stream = trade_stream();
        let mut streams = HashSet::new();
        streams.insert(stream);

        let result = dispatch_tick(&mut clock, &store, &streams, 1_000);
        assert!(result.trade_events.is_empty());
    }

    #[test]
    fn dispatch_returns_one_trade_in_half_open_range_when_store_loaded() {
        let stream = trade_stream();
        let store = make_store_with_data(stream, 0..10_000);
        let mut clock = StepClock::new(0, 10_000, 1_000);

        let mut streams = HashSet::new();
        streams.insert(stream);

        let result = dispatch_tick(&mut clock, &store, &streams, 1_000);
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
    fn dispatch_sets_reached_end_when_range_end_reached() {
        let stream = trade_stream();
        let store = make_store_with_data(stream, 0..3_000);
        let mut clock = StepClock::new(0, 3_000, 1_000);

        let mut streams = HashSet::new();
        streams.insert(stream);

        let result = dispatch_tick(&mut clock, &store, &streams, 5_000);
        assert!(result.reached_end);
        assert_eq!(clock.now_ms(), 3_000);
    }
}

