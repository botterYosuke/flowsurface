/// リプレイモジュール共通テストヘルパー。
/// `dispatcher.rs` / `store.rs` / `loader.rs` の各テストモジュールで使用する。
use exchange::Volume;
use exchange::adapter::{Exchange, StreamKind};
use exchange::unit::MinTicksize;
use exchange::unit::price::Price;
use exchange::unit::qty::Qty;
use exchange::{Kline, Ticker, TickerInfo, Timeframe, Trade};

pub fn dummy_trade(time: u64) -> Trade {
    Trade {
        time,
        is_sell: false,
        price: Price::from_f32_lossy(100.0),
        qty: Qty::from_f32_lossy(1.0),
    }
}

pub fn dummy_kline(time: u64) -> Kline {
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

pub fn trade_stream() -> StreamKind {
    StreamKind::Trades {
        ticker_info: TickerInfo::new(
            Ticker::new("BTCUSDT", Exchange::BinanceLinear),
            0.01,
            0.001,
            Some(1.0),
        ),
    }
}

pub fn kline_stream() -> StreamKind {
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
