use std::collections::HashMap;
use std::ops::Range;

use exchange::{Kline, Trade};
use exchange::adapter::StreamKind;

/// (stream, time) で引ける read-only 履歴データストア。
/// Range 単位で bulk load される。
pub struct EventStore {
    klines: HashMap<StreamKind, SortedVec<Kline>>,
    trades: HashMap<StreamKind, SortedVec<Trade>>,
    /// 各 stream で既に load 済みの時刻範囲集合。
    loaded_ranges: HashMap<StreamKind, Vec<Range<u64>>>,
}

/// 時刻順にソートされた Vec。挿入時に維持、クエリは binary search。
pub struct SortedVec<T> {
    data: Vec<T>,
}

pub struct LoadedData {
    pub klines: Vec<Kline>,
    pub trades: Vec<Trade>,
}

impl<T> SortedVec<T> {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl SortedVec<Trade> {
    pub fn insert_sorted(&mut self, items: Vec<Trade>) {
        self.data.extend(items);
        self.data.sort_by_key(|t| t.time);
        self.data.dedup_by_key(|t| t.time);
    }

    pub fn range_slice(&self, range: Range<u64>) -> &[Trade] {
        let start = self.data.partition_point(|t| t.time < range.start);
        let end = self.data.partition_point(|t| t.time < range.end);
        &self.data[start..end]
    }
}

impl SortedVec<Kline> {
    pub fn insert_sorted(&mut self, items: Vec<Kline>) {
        self.data.extend(items);
        self.data.sort_by_key(|k| k.time);
        self.data.dedup_by_key(|k| k.time);
    }

    pub fn range_slice(&self, range: Range<u64>) -> &[Kline] {
        let start = self.data.partition_point(|k| k.time < range.start);
        let end = self.data.partition_point(|k| k.time < range.end);
        &self.data[start..end]
    }
}

impl EventStore {
    pub fn new() -> Self {
        Self {
            klines: HashMap::new(),
            trades: HashMap::new(),
            loaded_ranges: HashMap::new(),
        }
    }

    /// 指定範囲の trades を返す（binary search）。cursor なし。
    pub fn trades_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Trade] {
        match self.trades.get(stream) {
            Some(sv) => sv.range_slice(range),
            None => &[],
        }
    }

    /// 指定範囲の klines を返す（binary search）。cursor なし。
    pub fn klines_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Kline] {
        match self.klines.get(stream) {
            Some(sv) => sv.range_slice(range),
            None => &[],
        }
    }

    /// range が loaded かどうかを返す。
    pub fn is_loaded(&self, stream: &StreamKind, range: Range<u64>) -> bool {
        let Some(loaded) = self.loaded_ranges.get(stream) else {
            return false;
        };
        // 初期実装: loaded_ranges のいずれかが range を包含するか確認
        loaded
            .iter()
            .any(|lr| lr.start <= range.start && lr.end >= range.end)
    }

    /// データを Store に注入し、loaded_ranges を更新する。
    pub fn ingest_loaded(&mut self, stream: StreamKind, range: Range<u64>, data: LoadedData) {
        self.trades
            .entry(stream)
            .or_insert_with(SortedVec::new)
            .insert_sorted(data.trades);
        self.klines
            .entry(stream)
            .or_insert_with(SortedVec::new)
            .insert_sorted(data.klines);
        self.loaded_ranges
            .entry(stream)
            .or_default()
            .push(range);
    }

    /// stream がどのペインからも参照されなくなったときに呼ぶ。
    #[cfg(test)]
    pub fn drop_stream(&mut self, stream: &StreamKind) {
        self.klines.remove(stream);
        self.trades.remove(stream);
        self.loaded_ranges.remove(stream);
    }

    /// stream の trade データ件数（テスト用）
    #[cfg(test)]
    pub fn trade_count(&self, stream: &StreamKind) -> usize {
        self.trades.get(stream).map_or(0, |sv| sv.len())
    }

    #[cfg(test)]
    pub fn kline_count(&self, stream: &StreamKind) -> usize {
        self.klines.get(stream).map_or(0, |sv| sv.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::unit::price::Price;
    use exchange::unit::qty::Qty;
    use exchange::unit::MinTicksize;
    use exchange::Volume;

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

    fn kline_stream() -> StreamKind {
        use exchange::adapter::Exchange;
        use exchange::{Ticker, TickerInfo, Timeframe};
        StreamKind::Kline {
            ticker_info: TickerInfo::new(
                Ticker::new("BTCUSDT", Exchange::BinanceLinear),
                0.01,
                0.001,
                Some(1.0),
            ),
            timeframe: Timeframe::M1,
        }
    }

    #[test]
    fn empty_store_returns_empty_slice_for_trades() {
        let store = EventStore::new();
        let stream = trade_stream();
        let result = store.trades_in(&stream, 0..1_000);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_store_is_not_loaded() {
        let store = EventStore::new();
        let stream = trade_stream();
        assert!(!store.is_loaded(&stream, 0..1_000));
    }

    #[test]
    fn is_loaded_after_ingest() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        store.ingest_loaded(
            stream,
            0..5_000,
            LoadedData {
                klines: vec![],
                trades: vec![dummy_trade(100), dummy_trade(200)],
            },
        );
        assert!(store.is_loaded(&stream, 0..5_000));
    }

    #[test]
    fn is_loaded_requires_range_to_be_covered() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        store.ingest_loaded(
            stream,
            0..5_000,
            LoadedData {
                klines: vec![],
                trades: vec![],
            },
        );
        // Querying a range outside what's loaded returns false
        assert!(!store.is_loaded(&stream, 0..10_000));
    }

    #[test]
    fn trades_in_returns_subset_by_time_range() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        let trades = vec![
            dummy_trade(100),
            dummy_trade(200),
            dummy_trade(300),
            dummy_trade(400),
        ];
        store.ingest_loaded(
            stream,
            0..1_000,
            LoadedData {
                klines: vec![],
                trades,
            },
        );
        let result = store.trades_in(&stream, 150..350);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].time, 200);
        assert_eq!(result[1].time, 300);
    }

    #[test]
    fn klines_in_returns_subset_by_time_range() {
        let mut store = EventStore::new();
        let stream = kline_stream();
        let klines = vec![
            dummy_kline(1_000),
            dummy_kline(2_000),
            dummy_kline(3_000),
        ];
        store.ingest_loaded(
            stream,
            0..5_000,
            LoadedData {
                klines,
                trades: vec![],
            },
        );
        let result = store.klines_in(&stream, 1_500..3_000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].time, 2_000);
    }

    #[test]
    fn drop_stream_removes_data() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        store.ingest_loaded(
            stream,
            0..1_000,
            LoadedData {
                klines: vec![],
                trades: vec![dummy_trade(100)],
            },
        );
        assert_eq!(store.trade_count(&stream), 1);

        store.drop_stream(&stream);
        assert_eq!(store.trade_count(&stream), 0);
        assert!(!store.is_loaded(&stream, 0..1_000));
    }

    #[test]
    fn ingest_twice_deduplicates_by_time() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        let first_batch = vec![dummy_trade(100), dummy_trade(200)];
        let second_batch = vec![dummy_trade(200), dummy_trade(300)]; // 200 is duplicate

        store.ingest_loaded(
            stream,
            0..1_000,
            LoadedData {
                klines: vec![],
                trades: first_batch,
            },
        );
        store.ingest_loaded(
            stream,
            0..1_000,
            LoadedData {
                klines: vec![],
                trades: second_batch,
            },
        );
        // dedup: 100, 200, 300 = 3 unique
        assert_eq!(store.trade_count(&stream), 3);
    }

    #[test]
    fn klines_in_with_exclusive_start_skips_current_time_kline() {
        let mut store = EventStore::new();
        let stream = kline_stream();
        let klines = vec![
            dummy_kline(60_000),
            dummy_kline(120_000),
            dummy_kline(180_000),
        ];
        store.ingest_loaded(
            stream,
            0..3_000_000,
            LoadedData {
                klines,
                trades: vec![],
            },
        );

        // New pattern: current_time + 1 as range start — excludes the kline AT current_time.
        let current_time: u64 = 120_000;
        let next = store.klines_in(&stream, current_time + 1..3_000_000);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].time, 180_000);

        // Old pattern: current_time as range start — INCLUDES the kline AT current_time,
        // meaning .find(|k| k.time > current_time) has to skip it explicitly.
        let old_first = store
            .klines_in(&stream, current_time..3_000_000)
            .first()
            .map(|k| k.time);
        assert_eq!(old_first, Some(120_000)); // proves old pattern starts at current_time's kline
    }
}
