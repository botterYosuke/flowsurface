use crate::Flowsurface;
use crate::connector::ResolvedStream;
use data::stream::PersistStreamKind;
use exchange::TickerInfo;

#[derive(serde::Serialize)]
pub(crate) struct KlineStateItem {
    pub(crate) stream: String,
    pub(crate) time: u64,
    pub(crate) open: f64,
    pub(crate) high: f64,
    pub(crate) low: f64,
    pub(crate) close: f64,
    pub(crate) volume: f64,
}

#[derive(serde::Serialize)]
pub(crate) struct TradeStateItem {
    pub(crate) stream: String,
    pub(crate) time: u64,
    pub(crate) price: f64,
    pub(crate) qty: f64,
    pub(crate) is_sell: bool,
}

pub(crate) fn extract_pane_ticker_timeframe(
    streams: &ResolvedStream,
) -> (Option<String>, Option<String>) {
    let format_ticker = |ticker: &exchange::Ticker| -> String {
        let ex_str = format!("{:?}", ticker.exchange).replace(' ', "");
        format!("{ex_str}:{ticker}")
    };

    match streams {
        ResolvedStream::Ready(list) => {
            let mut ticker_str: Option<String> = None;
            let mut tf_str: Option<String> = None;
            for s in list {
                if ticker_str.is_none() {
                    ticker_str = Some(format_ticker(&s.ticker_info().ticker));
                }
                if let Some((_, tf)) = s.as_kline_stream() {
                    tf_str = Some(format!("{tf:?}"));
                    break;
                }
            }
            (ticker_str, tf_str)
        }
        ResolvedStream::Waiting {
            streams: persist, ..
        } => {
            let mut ticker_str: Option<String> = None;
            let mut tf_str: Option<String> = None;
            for ps in persist {
                match ps {
                    PersistStreamKind::Kline { ticker, timeframe } => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(ticker));
                        }
                        tf_str = Some(format!("{timeframe:?}"));
                        break;
                    }
                    PersistStreamKind::Depth(d) => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(&d.ticker));
                        }
                    }
                    PersistStreamKind::Trades { ticker } => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(ticker));
                        }
                    }
                    PersistStreamKind::DepthAndTrades(d) => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(&d.ticker));
                        }
                    }
                }
            }
            (ticker_str, tf_str)
        }
    }
}

impl Flowsurface {
    pub(crate) fn parse_ser_ticker(s: &str) -> Result<exchange::Ticker, String> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "invalid ticker format: expected 'Exchange:Ticker', got '{s}'"
            ));
        }
        let exchange_str = parts[0];
        let normalized = ["Linear", "Inverse", "Spot"]
            .into_iter()
            .find_map(|suffix| {
                exchange_str
                    .strip_suffix(suffix)
                    .map(|prefix| format!("{prefix} {suffix}"))
            })
            .unwrap_or_else(|| exchange_str.to_owned());
        let exchange: exchange::adapter::Exchange = normalized
            .parse()
            .map_err(|_| format!("unknown exchange: {exchange_str}"))?;
        let ticker = exchange::Ticker::new(parts[1], exchange);
        Ok(ticker)
    }

    pub(crate) fn parse_timeframe(s: &str) -> Option<exchange::Timeframe> {
        crate::headless::parse_timeframe_str(s).ok()
    }

    pub(crate) fn parse_content_kind(s: &str) -> Option<data::layout::pane::ContentKind> {
        use data::layout::pane::ContentKind;
        match s {
            "CandlestickChart" | "Candlestick Chart" | "KlineChart" => {
                Some(ContentKind::CandlestickChart)
            }
            "HeatmapChart" | "Heatmap Chart" => Some(ContentKind::HeatmapChart),
            "ShaderHeatmap" | "Shader Heatmap" => Some(ContentKind::ShaderHeatmap),
            "FootprintChart" | "Footprint Chart" => Some(ContentKind::FootprintChart),
            "ComparisonChart" | "Comparison Chart" => Some(ContentKind::ComparisonChart),
            "TimeAndSales" | "Time&Sales" => Some(ContentKind::TimeAndSales),
            "Ladder" => Some(ContentKind::Ladder),
            "Starter" | "Starter Pane" => Some(ContentKind::Starter),
            "OrderEntry" | "Order Entry" => Some(ContentKind::OrderEntry),
            "OrderList" | "Order List" => Some(ContentKind::OrderList),
            "BuyingPower" | "Buying Power" => Some(ContentKind::BuyingPower),
            _ => None,
        }
    }

    pub(crate) fn resolve_ticker_info(&self, ticker: &exchange::Ticker) -> Option<TickerInfo> {
        self.sidebar
            .tickers_info()
            .get(ticker)
            .and_then(|opt| *opt)
            .or_else(|| {
                if ticker.exchange == exchange::adapter::Exchange::Tachibana {
                    exchange::adapter::tachibana::get_ticker_info_sync(ticker)
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timeframe_accepts_uppercase_form() {
        assert_eq!(
            Flowsurface::parse_timeframe("M1"),
            Some(exchange::Timeframe::M1)
        );
        assert_eq!(
            Flowsurface::parse_timeframe("H1"),
            Some(exchange::Timeframe::H1)
        );
        assert_eq!(
            Flowsurface::parse_timeframe("D1"),
            Some(exchange::Timeframe::D1)
        );
    }

    #[test]
    fn parse_timeframe_accepts_lowercase_alias() {
        assert_eq!(
            Flowsurface::parse_timeframe("1m"),
            Some(exchange::Timeframe::M1)
        );
        assert_eq!(
            Flowsurface::parse_timeframe("5m"),
            Some(exchange::Timeframe::M5)
        );
        assert_eq!(
            Flowsurface::parse_timeframe("1h"),
            Some(exchange::Timeframe::H1)
        );
        assert_eq!(
            Flowsurface::parse_timeframe("1d"),
            Some(exchange::Timeframe::D1)
        );
    }

    #[test]
    fn parse_timeframe_returns_none_for_unknown() {
        assert_eq!(Flowsurface::parse_timeframe("X99"), None);
    }
}
