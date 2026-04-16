/// 仮想注文帳（注文受付・約定判定）
///
/// # 約定ルール
/// - 成行注文 → `on_tick()` 内でその tick の最初の Trade 価格で即時約定
/// - 指値買い  → trade.price <= limit_price のトレードが来た tick で約定
/// - 指値売り  → trade.price >= limit_price のトレードが来た tick で約定
///
/// `place()` は `Pending` 状態で登録するのみ。約定は必ず次の `on_tick()` で行う。
use exchange::Trade;

use super::portfolio::{Position, PositionSide, VirtualPortfolio};

// ── 型定義 ────────────────────────────────────────────────────────────────────

/// UI や HTTP API から受け付ける仮想注文
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VirtualOrder {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub order_type: VirtualOrderType,
    /// StepClock::now_ms() で記録
    pub placed_time_ms: u64,
    pub status: VirtualOrderStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum VirtualOrderType {
    Market,
    Limit { price: f64 },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VirtualOrderStatus {
    Pending,
    Filled { fill_price: f64, fill_time_ms: u64 },
    Cancelled,
}

/// 約定イベント（UI 通知・ナラティブ記録に使う）
#[derive(Debug, Clone)]
pub struct FillEvent {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub fill_price: f64,
    pub fill_time_ms: u64,
}

/// 注文受付・約定判定エンジン
#[derive(Debug)]
pub struct VirtualOrderBook {
    pending: Vec<VirtualOrder>,
    portfolio: VirtualPortfolio,
}

impl Default for VirtualOrderBook {
    fn default() -> Self {
        Self::new(1_000_000.0)
    }
}

// ── 実装 ──────────────────────────────────────────────────────────────────────

impl VirtualOrderBook {
    pub fn new(initial_cash: f64) -> Self {
        Self {
            pending: Vec::new(),
            portfolio: VirtualPortfolio::new(initial_cash),
        }
    }

    /// 注文を受け付ける（UI / HTTP API 共通エントリーポイント）。
    /// 戻り値: order_id。約定は次の on_tick() まで保留。
    pub fn place(&mut self, order: VirtualOrder) -> String {
        let id = order.order_id.clone();
        self.pending.push(order);
        id
    }

    /// StepClock が 1 tick 進んだときに呼ぶ。
    ///
    /// # 引数
    /// - `ticker`: このトレード一覧が属する銘柄（呼び出し側が StreamKind から特定して渡す）
    /// - `trades`: EventStore から取り出した Trade のスライス
    /// - `now_ms`: StepClock::now_ms()
    ///
    /// 戻り値: 約定した注文の一覧（UI 通知用）
    pub fn on_tick(&mut self, ticker: &str, trades: &[Trade], now_ms: u64) -> Vec<FillEvent> {
        if trades.is_empty() {
            return Vec::new();
        }

        // 最初の trade 価格（成行約定に使う）
        let first_price = trades[0].price.to_f64();

        let mut fills = Vec::new();

        for order in self.pending.iter_mut() {
            // ticker が一致しない注文はスキップ
            if order.ticker != ticker {
                continue;
            }
            if order.status != VirtualOrderStatus::Pending {
                continue;
            }

            let fill_price = match &order.order_type {
                VirtualOrderType::Market => Some(first_price),
                VirtualOrderType::Limit { price: limit } => {
                    let limit = *limit;
                    // 約定条件を満たす最初の trade を探す
                    trades.iter().find_map(|t| {
                        let tp = t.price.to_f64();
                        let triggered = match order.side {
                            PositionSide::Long => tp <= limit,
                            PositionSide::Short => tp >= limit,
                        };
                        if triggered { Some(tp) } else { None }
                    })
                }
            };

            if let Some(fp) = fill_price {
                order.status = VirtualOrderStatus::Filled {
                    fill_price: fp,
                    fill_time_ms: now_ms,
                };
                fills.push(FillEvent {
                    order_id: order.order_id.clone(),
                    ticker: order.ticker.clone(),
                    side: order.side.clone(),
                    qty: order.qty,
                    fill_price: fp,
                    fill_time_ms: now_ms,
                });
                // ポートフォリオに open ポジションを記録
                self.portfolio.record_open(Position {
                    order_id: order.order_id.clone(),
                    ticker: order.ticker.clone(),
                    side: order.side.clone(),
                    qty: order.qty,
                    entry_price: fp,
                    entry_time_ms: now_ms,
                    exit_price: None,
                    exit_time_ms: None,
                    realized_pnl: None,
                });
            }
        }

        // 約定済みを pending から除去
        self.pending
            .retain(|o| o.status == VirtualOrderStatus::Pending);

        fills
    }

    /// seek / replay 再開時にリセット
    pub fn reset(&mut self) {
        self.pending.clear();
        self.portfolio.reset();
    }

    /// 現在のポートフォリオスナップショット（HTTP API 用）
    pub fn portfolio_snapshot(&self, current_price: f64) -> super::portfolio::PortfolioSnapshot {
        self.portfolio.snapshot(current_price)
    }

    /// 現在 pending な注文の一覧（HTTP API / UI 表示用）。
    /// 約定後は on_tick() 内で削除されるため、pending 状態のものだけが返る。
    pub fn pending_orders(&self) -> &[VirtualOrder] {
        &self.pending
    }

    /// ポートフォリオへの参照（テスト用）
    #[cfg(test)]
    pub fn portfolio(&self) -> &VirtualPortfolio {
        &self.portfolio
    }
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use exchange::{Trade, unit::price::Price, unit::qty::Qty};

    fn make_trade(price_raw: f32) -> Trade {
        Trade {
            time: 1_000,
            is_sell: false,
            price: Price::from_f32(price_raw),
            qty: Qty::from_f32(1.0),
        }
    }

    fn market_buy(ticker: &str, qty: f64, now_ms: u64) -> VirtualOrder {
        VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: PositionSide::Long,
            qty,
            order_type: VirtualOrderType::Market,
            placed_time_ms: now_ms,
            status: VirtualOrderStatus::Pending,
        }
    }

    fn limit_buy(ticker: &str, qty: f64, limit_price: f64, now_ms: u64) -> VirtualOrder {
        VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: PositionSide::Long,
            qty,
            order_type: VirtualOrderType::Limit { price: limit_price },
            placed_time_ms: now_ms,
            status: VirtualOrderStatus::Pending,
        }
    }

    fn limit_sell(ticker: &str, qty: f64, limit_price: f64, now_ms: u64) -> VirtualOrder {
        VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: PositionSide::Short,
            qty,
            order_type: VirtualOrderType::Limit { price: limit_price },
            placed_time_ms: now_ms,
            status: VirtualOrderStatus::Pending,
        }
    }

    #[test]
    fn market_order_fills_on_next_tick() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));

        let trades = vec![make_trade(92_000.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "1件約定するはず");
        assert!((fills[0].fill_price - 92_000.0).abs() < 1.0);
        assert_eq!(book.pending_orders().len(), 0, "約定後は pending から消えるはず");
    }

    #[test]
    fn limit_buy_fills_when_trade_below_price() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        // 指値 91_000 で買い注文
        book.place(limit_buy("BTCUSDT", 0.1, 91_000.0, 1_000));

        // 90_500 のトレードが来た → 91_000 <= 90_500? No → 逆: 90_500 <= 91_000? Yes → 約定
        let trades = vec![make_trade(90_500.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "指値以下のトレードで約定するはず");
    }

    #[test]
    fn limit_sell_fills_when_trade_above_price() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        // 指値 93_000 で売り注文
        book.place(limit_sell("BTCUSDT", 0.1, 93_000.0, 1_000));

        // 93_500 のトレードが来た → 93_500 >= 93_000 → 約定
        let trades = vec![make_trade(93_500.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "指値以上のトレードで約定するはず");
    }

    #[test]
    fn limit_not_filled_when_price_not_met() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        // 指値 91_000 で買い → 91_500 のトレードは条件未達
        book.place(limit_buy("BTCUSDT", 0.1, 91_000.0, 1_000));

        let trades = vec![make_trade(91_500.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 0, "指値未達では約定しないはず");
        assert_eq!(book.pending_orders().len(), 1, "pending に残るはず");
    }

    #[test]
    fn portfolio_pnl_after_round_trip() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 1.0, 1_000));

        // 買い約定
        let fills = book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);
        assert_eq!(fills.len(), 1);

        // 未実現 PnL 確認
        let snap = book.portfolio_snapshot(92_000.0);
        assert!((snap.unrealized_pnl - 2_000.0).abs() < 1.0);

        // 売り注文（ロングを手動でクローズするためにショート注文を入れる代わりに
        // portfolio.record_close を直接呼ぶ）
        book.portfolio.record_close(&fills[0].order_id, 92_000.0, 3_000);
        assert!((book.portfolio.realized_pnl() - 2_000.0).abs() < 1.0);
    }

    #[test]
    fn reset_clears_pending_and_portfolio() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        book.reset();

        assert_eq!(book.pending_orders().len(), 0, "reset 後は pending が空のはず");
        let snap = book.portfolio_snapshot(90_000.0);
        assert_eq!(snap.open_positions.len(), 0, "reset 後は open ポジションが空のはず");
        assert!((snap.cash - 1_000_000.0).abs() < 1.0, "reset 後は cash が初期値に戻るはず");
    }

    #[test]
    fn on_tick_ignores_wrong_ticker() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));

        // ETHUSDT のトレードが来ても BTCUSDT 注文は約定しない
        let trades = vec![make_trade(90_000.0)];
        let fills = book.on_tick("ETHUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 0, "ticker 不一致では約定しないはず");
        assert_eq!(book.pending_orders().len(), 1, "pending に残るはず");
    }
}
