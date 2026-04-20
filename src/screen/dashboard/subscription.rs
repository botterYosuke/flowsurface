use exchange::{
    PushFrequency,
    adapter::{StreamConfig, StreamSpecs, StreamTicksize},
    connect::{MAX_KLINE_STREAMS_PER_STREAM, MAX_TRADE_TICKERS_PER_STREAM},
};
use iced::Subscription;

pub(super) fn build_depth_subs(
    _exchange: exchange::adapter::Exchange,
    specs: &StreamSpecs,
) -> Option<Subscription<exchange::Event>> {
    if specs.depth.is_empty() {
        return None;
    }

    let subs = specs
        .depth
        .iter()
        .map(|(ticker, aggr, push_freq)| {
            let tick_mltp = match aggr {
                StreamTicksize::Client => None,
                StreamTicksize::ServerSide(tick_mltp) => Some(*tick_mltp),
            };
            let config = StreamConfig::new(*ticker, ticker.exchange(), tick_mltp, *push_freq);
            Subscription::run_with(config, exchange::connect::depth_stream)
        })
        .collect::<Vec<_>>();

    Some(Subscription::batch(subs))
}

pub(super) fn build_trade_subs(
    exchange: exchange::adapter::Exchange,
    specs: &StreamSpecs,
) -> Option<Subscription<exchange::Event>> {
    if specs.trade.is_empty() {
        return None;
    }

    let subs = specs
        .trade
        .chunks(MAX_TRADE_TICKERS_PER_STREAM)
        .map(|tickers| {
            let config = StreamConfig::new(
                tickers.to_vec(),
                exchange,
                None,
                PushFrequency::ServerDefault,
            );
            Subscription::run_with(config, exchange::connect::trade_stream)
        })
        .collect::<Vec<_>>();

    Some(Subscription::batch(subs))
}

pub(super) fn build_kline_subs(
    exchange: exchange::adapter::Exchange,
    specs: &StreamSpecs,
) -> Option<Subscription<exchange::Event>> {
    if specs.kline.is_empty() {
        return None;
    }

    let subs = specs
        .kline
        .chunks(MAX_KLINE_STREAMS_PER_STREAM)
        .map(|streams| {
            let config = StreamConfig::new(
                streams.to_vec(),
                exchange,
                None,
                PushFrequency::ServerDefault,
            );
            Subscription::run_with(config, exchange::connect::kline_stream)
        })
        .collect::<Vec<_>>();

    Some(Subscription::batch(subs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::adapter::{Exchange, StreamSpecs};

    fn make_ticker_info_binance() -> exchange::TickerInfo {
        exchange::TickerInfo::new(
            exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BinanceLinear),
            0.1,
            0.001,
            None,
        )
    }

    fn make_ticker_info_bybit() -> exchange::TickerInfo {
        exchange::TickerInfo::new(
            exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BybitLinear),
            0.1,
            0.001,
            None,
        )
    }

    #[test]
    fn build_depth_subs_returns_none_when_specs_depth_is_empty() {
        let specs = StreamSpecs::default();
        let result = build_depth_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_none(),
            "depth が空のとき build_depth_subs は None を返すこと"
        );
    }

    #[test]
    fn build_depth_subs_returns_some_when_specs_depth_is_non_empty() {
        use exchange::{PushFrequency, adapter::StreamTicksize};
        let ticker = make_ticker_info_binance();
        let specs = StreamSpecs {
            depth: vec![(ticker, StreamTicksize::Client, PushFrequency::ServerDefault)],
            ..Default::default()
        };
        let result = build_depth_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "depth が非空のとき build_depth_subs は Some を返すこと"
        );
    }

    #[test]
    fn build_trade_subs_returns_none_when_specs_trade_is_empty() {
        let specs = StreamSpecs::default();
        let result = build_trade_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_none(),
            "trade が空のとき build_trade_subs は None を返すこと"
        );
    }

    #[test]
    fn build_trade_subs_returns_some_when_specs_trade_is_non_empty() {
        let ticker = make_ticker_info_binance();
        let specs = StreamSpecs {
            trade: vec![ticker],
            ..Default::default()
        };
        let result = build_trade_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "trade が非空のとき build_trade_subs は Some を返すこと"
        );
    }

    #[test]
    fn build_trade_subs_returns_some_when_trade_exceeds_max_per_stream() {
        let trade: Vec<_> = (0..101)
            .map(|i| {
                exchange::TickerInfo::new(
                    exchange::Ticker::new(
                        &format!("TICKER{i:03}USDT"),
                        exchange::adapter::Exchange::BinanceLinear,
                    ),
                    0.1,
                    0.001,
                    None,
                )
            })
            .collect();
        let specs = StreamSpecs {
            trade,
            ..Default::default()
        };
        let result = build_trade_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "trade が MAX を超えても build_trade_subs は Some を返すこと"
        );
    }

    #[test]
    fn build_kline_subs_returns_none_when_specs_kline_is_empty() {
        let specs = StreamSpecs::default();
        let result = build_kline_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_none(),
            "kline が空のとき build_kline_subs は None を返すこと"
        );
    }

    #[test]
    fn build_kline_subs_returns_some_when_specs_kline_is_non_empty() {
        use exchange::Timeframe;
        let ticker = make_ticker_info_binance();
        let specs = StreamSpecs {
            kline: vec![(ticker, Timeframe::M1)],
            ..Default::default()
        };
        let result = build_kline_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "kline が非空のとき build_kline_subs は Some を返すこと"
        );
    }

    #[test]
    fn build_kline_subs_returns_some_when_kline_exceeds_max_per_stream() {
        use exchange::Timeframe;
        let kline: Vec<_> = (0..101)
            .map(|i| {
                let info = exchange::TickerInfo::new(
                    exchange::Ticker::new(
                        &format!("TICKER{i:03}USDT"),
                        exchange::adapter::Exchange::BinanceLinear,
                    ),
                    0.1,
                    0.001,
                    None,
                );
                (info, Timeframe::M1)
            })
            .collect();
        let specs = StreamSpecs {
            kline,
            ..Default::default()
        };
        let result = build_kline_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "kline が MAX を超えても build_kline_subs は Some を返すこと"
        );
    }

    #[test]
    fn depth_subs_use_ticker_exchange_not_argument_exchange() {
        use exchange::{PushFrequency, adapter::StreamTicksize};
        let ticker = make_ticker_info_bybit();
        let specs = StreamSpecs {
            depth: vec![(ticker, StreamTicksize::Client, PushFrequency::ServerDefault)],
            ..Default::default()
        };
        let result = build_depth_subs(Exchange::BinanceLinear, &specs);
        assert!(
            result.is_some(),
            "build_depth_subs は ticker.exchange() を使うため引数 exchange に依存しない"
        );
    }
}
