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

                // Short 注文: 既存 Long があればクローズ（ネットアウト）、なければ新規 Short
                // Long 注文: 常に新規ポジションを open
                match order.side {
                    PositionSide::Short => {
                        // 先に order_id を取得（その後 portfolio を可変借用するため）
                        let existing_long_id = self
                            .portfolio
                            .oldest_open_long_order_id(ticker)
                            .map(str::to_string);
                        if let Some(long_id) = existing_long_id {
                            self.portfolio.record_close(&long_id, fp, now_ms);
                        } else {
                            self.portfolio.record_open(Position {
                                order_id: order.order_id.clone(),
                                ticker: order.ticker.clone(),
                                side: PositionSide::Short,
                                qty: order.qty,
                                entry_price: fp,
                                entry_time_ms: now_ms,
                                exit_price: None,
                                exit_time_ms: None,
                                realized_pnl: None,
                            });
                        }
                    }
                    PositionSide::Long => {
                        // Long 注文: 既存 Short があればクローズ（ネットアウト）、なければ新規 Long
                        let existing_short_id = self
                            .portfolio
                            .oldest_open_short_order_id(ticker)
                            .map(str::to_string);
                        if let Some(short_id) = existing_short_id {
                            self.portfolio.record_close(&short_id, fp, now_ms);
                        } else {
                            self.portfolio.record_open(Position {
                                order_id: order.order_id.clone(),
                                ticker: order.ticker.clone(),
                                side: PositionSide::Long,
                                qty: order.qty,
                                entry_price: fp,
                                entry_time_ms: now_ms,
                                exit_price: None,
                                exit_time_ms: None,
                                realized_pnl: None,
                            });
                        }
                    }
                }
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
    #[allow(dead_code)]
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

    fn market_sell(ticker: &str, qty: f64, now_ms: u64) -> VirtualOrder {
        VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: PositionSide::Short,
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

    // ── 既存テスト ───────────────────────────────────────────────────────────

    #[test]
    fn market_order_fills_on_next_tick() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));

        let trades = vec![make_trade(92_000.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "1件約定するはず");
        assert!((fills[0].fill_price - 92_000.0).abs() < 1.0);
        assert_eq!(
            book.pending_orders().len(),
            0,
            "約定後は pending から消えるはず"
        );
    }

    #[test]
    fn limit_buy_fills_when_trade_below_price() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(limit_buy("BTCUSDT", 0.1, 91_000.0, 1_000));

        let trades = vec![make_trade(90_500.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "指値以下のトレードで約定するはず");
    }

    #[test]
    fn limit_sell_fills_when_trade_above_price() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(limit_sell("BTCUSDT", 0.1, 93_000.0, 1_000));

        let trades = vec![make_trade(93_500.0)];
        let fills = book.on_tick("BTCUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 1, "指値以上のトレードで約定するはず");
    }

    #[test]
    fn limit_not_filled_when_price_not_met() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
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

        // 買い約定 → cash = 1_000_000 - 90_000 = 910_000
        let fills = book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);
        assert_eq!(fills.len(), 1);

        // 未実現 PnL 確認
        let snap = book.portfolio_snapshot(92_000.0);
        assert!((snap.unrealized_pnl - 2_000.0).abs() < 1.0);

        // record_close を直接呼んでクローズ（on_tick 経由は A-2 の別テストで検証）
        // cash += 92_000 → 1_002_000
        book.portfolio
            .record_close(&fills[0].order_id, 92_000.0, 3_000);
        assert!((book.portfolio.realized_pnl() - 2_000.0).abs() < 1.0);
    }

    #[test]
    fn reset_clears_pending_and_portfolio() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        book.reset();

        assert_eq!(
            book.pending_orders().len(),
            0,
            "reset 後は pending が空のはず"
        );
        let snap = book.portfolio_snapshot(90_000.0);
        assert_eq!(
            snap.open_positions.len(),
            0,
            "reset 後は open ポジションが空のはず"
        );
        assert!(
            (snap.cash - 1_000_000.0).abs() < 1.0,
            "reset 後は cash が初期値に戻るはず"
        );
    }

    #[test]
    fn on_tick_ignores_wrong_ticker() {
        let mut book = VirtualOrderBook::new(1_000_000.0);
        book.place(market_buy("BTCUSDT", 0.1, 1_000));

        let trades = vec![make_trade(90_000.0)];
        let fills = book.on_tick("ETHUSDT", &trades, 2_000);

        assert_eq!(fills.len(), 0, "ticker 不一致では約定しないはず");
        assert_eq!(book.pending_orders().len(), 1, "pending に残るはず");
    }

    // ── A-2: on_tick() 経由のクローズロジック ────────────────────────────────

    #[test]
    fn sell_fill_closes_existing_long() {
        let mut book = VirtualOrderBook::new(1_000_000.0);

        // 買い約定
        book.place(market_buy("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        // 売り注文 → 既存 Long をクローズ
        book.place(market_sell("BTCUSDT", 1.0, 2_000));
        book.on_tick("BTCUSDT", &[make_trade(92_000.0)], 3_000);

        let snap = book.portfolio_snapshot(92_000.0);
        assert_eq!(
            snap.open_positions.len(),
            0,
            "Long がクローズされて open_positions = 0 のはず"
        );
        assert_eq!(
            snap.closed_positions.len(),
            1,
            "closed_positions = 1 のはず"
        );
    }

    #[test]
    fn sell_fill_without_long_opens_short() {
        let mut book = VirtualOrderBook::new(1_000_000.0);

        // Long なしで売り注文 → 新規 Short
        book.place(market_sell("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        let snap = book.portfolio_snapshot(90_000.0);
        assert_eq!(
            snap.open_positions.len(),
            1,
            "新規 Short ポジションが open されるはず"
        );
        assert_eq!(snap.open_positions[0].side, "Short");
        assert_eq!(snap.closed_positions.len(), 0);
    }

    #[test]
    fn round_trip_realized_pnl_positive() {
        // buy @90_000 → sell @92_000 → realized_pnl = +2_000
        let mut book = VirtualOrderBook::new(1_000_000.0);

        book.place(market_buy("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        book.place(market_sell("BTCUSDT", 1.0, 2_000));
        book.on_tick("BTCUSDT", &[make_trade(92_000.0)], 3_000);

        let snap = book.portfolio_snapshot(92_000.0);
        assert!(
            (snap.realized_pnl - 2_000.0).abs() < 1.0,
            "realized_pnl = +2000 のはず、実際: {}",
            snap.realized_pnl
        );
    }

    #[test]
    fn round_trip_realized_pnl_negative() {
        // buy @90_000 → sell @88_000 → realized_pnl = -2_000
        let mut book = VirtualOrderBook::new(1_000_000.0);

        book.place(market_buy("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);

        book.place(market_sell("BTCUSDT", 1.0, 2_000));
        book.on_tick("BTCUSDT", &[make_trade(88_000.0)], 3_000);

        let snap = book.portfolio_snapshot(88_000.0);
        assert!(
            (snap.realized_pnl - (-2_000.0)).abs() < 1.0,
            "realized_pnl = -2000 のはず、実際: {}",
            snap.realized_pnl
        );
    }

    #[test]
    fn cash_round_trip_profit() {
        // buy @90_000 qty 1.0 → sell @92_000 → cash = 1_002_000
        let mut book = VirtualOrderBook::new(1_000_000.0);

        book.place(market_buy("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);
        // cash = 910_000

        book.place(market_sell("BTCUSDT", 1.0, 2_000));
        book.on_tick("BTCUSDT", &[make_trade(92_000.0)], 3_000);
        // cash += 92_000 = 1_002_000

        let snap = book.portfolio_snapshot(92_000.0);
        assert!(
            (snap.cash - 1_002_000.0).abs() < 1.0,
            "cash = 1_002_000 のはず（利益 +2000 込み）、実際: {}",
            snap.cash
        );
    }

    #[test]
    fn cash_round_trip_loss() {
        // buy @90_000 qty 1.0 → sell @88_000 → cash = 998_000
        let mut book = VirtualOrderBook::new(1_000_000.0);

        book.place(market_buy("BTCUSDT", 1.0, 1_000));
        book.on_tick("BTCUSDT", &[make_trade(90_000.0)], 2_000);
        // cash = 910_000

        book.place(market_sell("BTCUSDT", 1.0, 2_000));
        book.on_tick("BTCUSDT", &[make_trade(88_000.0)], 3_000);
        // cash += 88_000 = 998_000

        let snap = book.portfolio_snapshot(88_000.0);
        assert!(
            (snap.cash - 998_000.0).abs() < 1.0,
            "cash = 998_000 のはず（損失 -2000 込み）、実際: {}",
            snap.cash
        );
    }
}
