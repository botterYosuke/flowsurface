/// 履歴データを fetch して EventStore に bulk load するためのローダー。
///
/// 設計:
/// - klines は `Task::perform(fetch_klines)` で一括 fetch → `LoadedData` を生成
/// - trades は既存の `build_trades_backfill_task` パターンを踏襲（R4 で統合）
/// - 完了時に呼び出し側が `EventStore::ingest_loaded` を呼ぶ
use std::ops::Range;

use exchange::adapter::{AdapterError, StreamKind};
use exchange::{Kline, TickerInfo, Timeframe};

/// klines の bulk load が完了したときに返すデータ。
pub struct KlineLoadResult {
    pub stream: StreamKind,
    pub range: Range<u64>,
    pub klines: Vec<Kline>,
}

/// klines を全量フェッチして `KlineLoadResult` を返す。
/// `Task::perform(load_klines(...), |result| Message::ReplayKlinesLoaded(result))` で使う。
pub async fn load_klines(stream: StreamKind, range: Range<u64>) -> Result<KlineLoadResult, String> {
    let (ticker_info, timeframe) = match stream {
        StreamKind::Kline {
            ticker_info,
            timeframe,
        } => (ticker_info, timeframe),
        other => {
            return Err(format!(
                "load_klines called on non-Kline stream: {:?}",
                std::mem::discriminant(&other)
            ));
        }
    };

    let klines = fetch_all_klines(ticker_info, timeframe, range.clone()).await?;
    Ok(KlineLoadResult {
        stream,
        range,
        klines,
    })
}

/// klines を全量フェッチして Vec<Kline> にまとめる。
async fn fetch_all_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Range<u64>,
) -> Result<Vec<Kline>, String> {
    use exchange::adapter::{self, Venue};

    if ticker_info.ticker.exchange.venue() == Venue::Tachibana {
        let (issue_code, _) = ticker_info.ticker.to_full_symbol_and_type();
        return crate::connector::fetcher::fetch_tachibana_daily_klines(
            &issue_code,
            Some((range.start, range.end)),
        )
        .await;
    }

    adapter::fetch_klines(ticker_info, timeframe, Some((range.start, range.end)))
        .await
        .map_err(|e: AdapterError| format!("klines fetch error: {e}"))
}

// ── テスト ────────────────────────────────────────────────────────────────────

/// ローダーの単体テスト。
///
/// ネットワーク呼び出しを避けるため、`LoadedData` の ingest → `is_loaded` / クエリ の
/// 振る舞いを EventStore 直接操作で検証する。
/// 実際の HTTP fetch は E2E テストで検証する。
#[cfg(test)]
mod tests {
    use super::super::store::{EventStore, LoadedData};
    use crate::replay::testutil::{kline_stream, trade_stream};
    use exchange::unit::MinTicksize;
    use exchange::unit::price::Price;
    use exchange::unit::qty::Qty;
    use exchange::{Timeframe, Trade, Volume};

    fn dummy_trades(n: usize, start_ms: u64, step_ms: u64) -> Vec<Trade> {
        (0..n)
            .map(|i| Trade {
                time: start_ms + i as u64 * step_ms,
                is_sell: false,
                price: Price::from_f32_lossy(100.0),
                qty: Qty::from_f32_lossy(1.0),
            })
            .collect()
    }

    fn dummy_klines(n: usize, start_ms: u64, step_ms: u64) -> Vec<exchange::Kline> {
        (0..n)
            .map(|i| {
                exchange::Kline::new(
                    start_ms + i as u64 * step_ms,
                    100.0,
                    101.0,
                    99.0,
                    100.5,
                    Volume::empty_total(),
                    MinTicksize::from(0.01),
                )
            })
            .collect()
    }

    /// bulk load が完了すると EventStore::is_loaded が true を返すことを検証する。
    /// これが R2 の核心テスト。
    #[test]
    fn bulk_load_makes_store_report_is_loaded_for_trades() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        let range = 0u64..60_000u64; // 1 minute
        let trades = dummy_trades(10, 0, 6_000);

        store.ingest_loaded(
            stream,
            range.clone(),
            LoadedData {
                klines: vec![],
                trades,
            },
        );

        assert!(
            store.is_loaded(&stream, range),
            "after ingest_loaded, is_loaded should return true"
        );
    }

    #[test]
    fn bulk_load_makes_store_report_is_loaded_for_klines() {
        let mut store = EventStore::new();
        let stream = kline_stream();
        let range = 0u64..3_600_000u64; // 1 hour
        let klines = dummy_klines(60, 0, 60_000);

        store.ingest_loaded(
            stream,
            range.clone(),
            LoadedData {
                klines,
                trades: vec![],
            },
        );

        assert!(store.is_loaded(&stream, range));
    }

    #[test]
    fn trades_are_queryable_after_bulk_load() {
        let mut store = EventStore::new();
        let stream = trade_stream();
        let trades = dummy_trades(5, 1_000, 2_000); // times: 1000, 3000, 5000, 7000, 9000

        store.ingest_loaded(
            stream,
            0..10_000,
            LoadedData {
                klines: vec![],
                trades,
            },
        );

        // Query sub-range
        let result = store.trades_in(&stream, 2_000..6_000);
        assert_eq!(result.len(), 2); // 3000 and 5000
        assert_eq!(result[0].time, 3_000);
        assert_eq!(result[1].time, 5_000);
    }

    #[test]
    fn klines_are_queryable_after_bulk_load() {
        let mut store = EventStore::new();
        let stream = kline_stream();
        let klines = dummy_klines(5, 0, 60_000); // times: 0, 60000, 120000, 180000, 240000

        store.ingest_loaded(
            stream,
            0..300_000,
            LoadedData {
                klines,
                trades: vec![],
            },
        );

        let result = store.klines_in(&stream, 50_000..200_000);
        assert_eq!(result.len(), 3); // 60000, 120000, 180000 (all < 200000)
    }

    #[test]
    fn loading_multiple_streams_independently() {
        let mut store = EventStore::new();
        let trades_s = trade_stream();
        let klines_s = kline_stream();

        store.ingest_loaded(
            trades_s,
            0..10_000,
            LoadedData {
                klines: vec![],
                trades: dummy_trades(3, 0, 3_000),
            },
        );
        store.ingest_loaded(
            klines_s,
            0..10_000,
            LoadedData {
                klines: dummy_klines(3, 0, 3_000),
                trades: vec![],
            },
        );

        assert!(store.is_loaded(&trades_s, 0..10_000));
        assert!(store.is_loaded(&klines_s, 0..10_000));
    }
}
