use exchange::adapter::StreamKind;
use exchange::{Kline, Trade};

use super::super::ReplaySession;
use super::ReplayController;

/// GetState API レスポンスウィンドウ: trades の取得範囲（現在時刻から遡る ms 数）。
pub(crate) const TRADE_WINDOW_MS: u64 = 300_000; // 5 分

/// GET /api/replay/state レスポンス用データ。
/// `ReplaySession::Active` のときのみ生成される。
#[must_use]
pub struct ApiStateData {
    pub current_time_ms: u64,
    /// `(stream_label, klines)` のリスト。例: `"BinanceLinear:BTCUSDT:1m"`
    pub klines: Vec<(String, Vec<Kline>)>,
    /// `(stream_label, trades)` のリスト。例: `"BinanceLinear:BTCUSDT:Trades"`
    pub trades: Vec<(String, Vec<Trade>)>,
}

impl ReplayController {
    /// アクティブセッションの最新 kline close 価格を返す。
    pub fn last_close_price(&self) -> Option<f64> {
        let (clock, store, active_streams) = match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => (clock, store, active_streams),
            _ => return None,
        };
        let now_ms = clock.now_ms();
        for stream in active_streams {
            if matches!(stream, StreamKind::Kline { .. }) {
                let klines = store.klines_in(stream, 0..now_ms + 1);
                if let Some(last) = klines.last() {
                    return Some(last.close.to_f64());
                }
            }
        }
        None
    }

    /// 現在の仮想時刻（ms）を返す。セッションがアクティブでない場合は `None`。
    pub fn current_time_ms(&self) -> Option<u64> {
        match &self.state.session {
            ReplaySession::Active { clock, .. } | ReplaySession::Loading { clock, .. } => {
                Some(clock.now_ms())
            }
            ReplaySession::Idle => None,
        }
    }

    /// リプレイ範囲の終端（ms）を返す。`Active` / `Loading` のみ値を返す。
    pub fn range_end_ms(&self) -> Option<u64> {
        match &self.state.session {
            ReplaySession::Active { clock, .. } | ReplaySession::Loading { clock, .. } => {
                Some(clock.full_range().end)
            }
            ReplaySession::Idle => None,
        }
    }

    /// 1 step で進む仮想時刻幅（ms）を返す。`Active` / `Loading` のみ値を返す。
    pub fn step_size_ms(&self) -> Option<u64> {
        match &self.state.session {
            ReplaySession::Active { clock, .. } | ReplaySession::Loading { clock, .. } => {
                Some(clock.step_size_ms())
            }
            ReplaySession::Idle => None,
        }
    }

    /// アクティブな kline ストリームを収集する（mid-replay 銘柄変更用）。
    /// `Kline` 種別のみを返す。
    pub fn active_kline_streams(&self) -> Vec<StreamKind> {
        let active_streams = match &self.state.session {
            ReplaySession::Loading { active_streams, .. }
            | ReplaySession::Active { active_streams, .. } => active_streams,
            ReplaySession::Idle => return vec![],
        };
        active_streams
            .iter()
            .filter(|s| matches!(s, StreamKind::Kline { .. }))
            .copied()
            .collect()
    }

    /// 全 active_streams をデバッグ文字列リストで返す（API 診断用）。
    pub fn active_stream_debug_labels(&self) -> Vec<String> {
        let active_streams = match &self.state.session {
            ReplaySession::Loading { active_streams, .. }
            | ReplaySession::Active { active_streams, .. } => active_streams,
            ReplaySession::Idle => return vec![],
        };
        active_streams.iter().map(|s| format!("{s:?}")).collect()
    }

    /// GET /api/replay/state 用: アクティブセッションの現在時刻と直近 N 件のデータを返す。
    /// `ReplaySession::Active` 以外のときは `None` を返す。
    pub fn get_api_state(&self, limit: usize) -> Option<ApiStateData> {
        let (clock, store, active_streams) = match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => (clock, store, active_streams),
            _ => return None,
        };

        let now_ms = clock.now_ms();
        let mut klines = Vec::new();
        let mut trades = Vec::new();

        let mut sorted_streams: Vec<_> = active_streams.iter().collect();
        sorted_streams.sort_by_cached_key(|s| format!("{s:?}"));

        for stream in sorted_streams {
            if let StreamKind::Kline {
                ticker_info,
                timeframe,
            } = stream
            {
                let now_end = now_ms.saturating_add(1);
                let all_klines = store.klines_in(stream, 0..now_end);
                let slice = if all_klines.len() > limit {
                    &all_klines[all_klines.len() - limit..]
                } else {
                    all_klines
                };
                let exchange_str = format!("{:?}", ticker_info.ticker.exchange).replace(' ', "");
                let ticker_str = ticker_info.ticker.to_string();

                if !slice.is_empty() {
                    let label = format!("{exchange_str}:{ticker_str}:{timeframe}");
                    klines.push((label, slice.to_vec()));
                }

                let trade_stream = StreamKind::Trades {
                    ticker_info: *ticker_info,
                };
                let trade_start = now_ms.saturating_sub(TRADE_WINDOW_MS);
                let all_trades = store.trades_in(&trade_stream, trade_start..now_end);
                let trade_slice = if all_trades.len() > limit {
                    &all_trades[all_trades.len() - limit..]
                } else {
                    all_trades
                };
                if !trade_slice.is_empty() {
                    let label = format!("{exchange_str}:{ticker_str}:Trades");
                    trades.push((label, trade_slice.to_vec()));
                }
            }
        }

        Some(ApiStateData {
            current_time_ms: now_ms,
            klines,
            trades,
        })
    }
}
