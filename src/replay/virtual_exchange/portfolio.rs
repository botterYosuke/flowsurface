/// 仮想ポートフォリオ — ポジション管理と PnL 計算
///
/// Phase 1 制約: 単一銘柄を前提とする。`unrealized_pnl(current_price)` は
/// 1 値のみ受け取る。複数銘柄の同時ポジションは Phase 2 で対応する。

// ── 型定義 ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PositionSide {
    Long,
    Short,
}

impl std::fmt::Display for PositionSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionSide::Long => write!(f, "Long"),
            PositionSide::Short => write!(f, "Short"),
        }
    }
}

/// 1 注文のポジション（open / closed 両方）
#[derive(Debug, Clone)]
pub struct Position {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub exit_price: Option<f64>,
    /// exit 時刻（Phase 2 の PnL 履歴表示で使用予定）
    #[allow(dead_code)]
    pub exit_time_ms: Option<u64>,
    pub realized_pnl: Option<f64>,
}

/// ポートフォリオ全体（open + closed ポジション）
#[derive(Debug, Clone)]
pub struct VirtualPortfolio {
    pub initial_cash: f64,
    pub cash: f64,
    positions: Vec<Position>,
}

impl Default for VirtualPortfolio {
    fn default() -> Self {
        Self::new(1_000_000.0)
    }
}

/// HTTP API レスポンス形式（そのまま JSON シリアライズ）
#[derive(Debug, Clone, serde::Serialize)]
pub struct PortfolioSnapshot {
    pub cash: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
    /// cash + unrealized_pnl
    pub total_equity: f64,
    pub open_positions: Vec<PositionSnapshot>,
    pub closed_positions: Vec<PositionSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PositionSnapshot {
    pub order_id: String,
    pub ticker: String,
    pub side: String,
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub exit_price: Option<f64>,
    pub realized_pnl: Option<f64>,
}

// ── 実装 ──────────────────────────────────────────────────────────────────────

impl VirtualPortfolio {
    pub fn new(initial_cash: f64) -> Self {
        Self {
            initial_cash,
            cash: initial_cash,
            positions: Vec::new(),
        }
    }

    /// 約定時に呼ぶ（open ポジションを追加し、現金を変動させる）。
    ///
    /// - Long: 購入コスト（entry_price × qty）を cash から差し引く
    /// - Short: 売り代金（entry_price × qty）を cash に加算する（裸ショート）
    pub fn record_open(&mut self, pos: Position) {
        match pos.side {
            PositionSide::Long => self.cash -= pos.entry_price * pos.qty,
            PositionSide::Short => self.cash += pos.entry_price * pos.qty,
        }
        self.positions.push(pos);
    }

    /// クローズ時に呼ぶ。実現 PnL を確定し、売却代金（または買い戻しコスト）を cash に反映する。
    ///
    /// - Long クローズ: exit_price × qty を cash に返還（売却代金）
    /// - Short クローズ: exit_price × qty を cash から差し引く（買い戻しコスト）
    pub fn record_close(&mut self, order_id: &str, exit_price: f64, exit_time_ms: u64) {
        if let Some(pos) = self.positions.iter_mut().find(|p| p.order_id == order_id) {
            let pnl = match pos.side {
                PositionSide::Long => (exit_price - pos.entry_price) * pos.qty,
                PositionSide::Short => (pos.entry_price - exit_price) * pos.qty,
            };
            pos.exit_price = Some(exit_price);
            pos.exit_time_ms = Some(exit_time_ms);
            pos.realized_pnl = Some(pnl);
            match pos.side {
                PositionSide::Long => self.cash += exit_price * pos.qty,
                PositionSide::Short => self.cash -= exit_price * pos.qty,
            }
        }
    }

    /// 指定 ticker の open Long ポジションの order_id を返す（最古優先 = FIFO）
    pub fn oldest_open_long_order_id(&self, ticker: &str) -> Option<&str> {
        self.positions
            .iter()
            .filter(|p| {
                p.ticker == ticker && p.side == PositionSide::Long && p.exit_price.is_none()
            })
            .min_by_key(|p| p.entry_time_ms)
            .map(|p| p.order_id.as_str())
    }

    /// 未実現 PnL（現在価格で評価）。単一銘柄を前提とする。
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 {
        self.positions
            .iter()
            .filter(|p| p.exit_price.is_none())
            .map(|p| match p.side {
                PositionSide::Long => (current_price - p.entry_price) * p.qty,
                PositionSide::Short => (p.entry_price - current_price) * p.qty,
            })
            .sum()
    }

    /// 実現 PnL 合計
    pub fn realized_pnl(&self) -> f64 {
        self.positions.iter().filter_map(|p| p.realized_pnl).sum()
    }

    /// 公開スナップショット（HTTP API の GET /api/replay/portfolio レスポンスに使う）
    pub fn snapshot(&self, current_price: f64) -> PortfolioSnapshot {
        let unrealized = self.unrealized_pnl(current_price);
        let realized = self.realized_pnl();

        let (open, closed) =
            self.positions
                .iter()
                .fold((Vec::new(), Vec::new()), |(mut open, mut closed), p| {
                    let snap = PositionSnapshot {
                        order_id: p.order_id.clone(),
                        ticker: p.ticker.clone(),
                        side: p.side.to_string(),
                        qty: p.qty,
                        entry_price: p.entry_price,
                        entry_time_ms: p.entry_time_ms,
                        exit_price: p.exit_price,
                        realized_pnl: p.realized_pnl,
                    };
                    if p.exit_price.is_none() {
                        open.push(snap);
                    } else {
                        closed.push(snap);
                    }
                    (open, closed)
                });

        PortfolioSnapshot {
            cash: self.cash,
            unrealized_pnl: unrealized,
            realized_pnl: realized,
            total_equity: self.cash + unrealized,
            open_positions: open,
            closed_positions: closed,
        }
    }

    /// seek / replay 再開時にリセット
    pub fn reset(&mut self) {
        self.cash = self.initial_cash;
        self.positions.clear();
    }
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_long_pos(order_id: &str, qty: f64, entry_price: f64) -> Position {
        Position {
            order_id: order_id.to_string(),
            ticker: "BTCUSDT".to_string(),
            side: PositionSide::Long,
            qty,
            entry_price,
            entry_time_ms: 1_000,
            exit_price: None,
            exit_time_ms: None,
            realized_pnl: None,
        }
    }

    fn make_long_pos_at(
        order_id: &str,
        qty: f64,
        entry_price: f64,
        entry_time_ms: u64,
    ) -> Position {
        Position {
            order_id: order_id.to_string(),
            ticker: "BTCUSDT".to_string(),
            side: PositionSide::Long,
            qty,
            entry_price,
            entry_time_ms,
            exit_price: None,
            exit_time_ms: None,
            realized_pnl: None,
        }
    }

    fn make_short_pos(order_id: &str, qty: f64, entry_price: f64) -> Position {
        Position {
            order_id: order_id.to_string(),
            ticker: "BTCUSDT".to_string(),
            side: PositionSide::Short,
            qty,
            entry_price,
            entry_time_ms: 1_000,
            exit_price: None,
            exit_time_ms: None,
            realized_pnl: None,
        }
    }

    // ── A-1: record_open の cash deduction ───────────────────────────────────

    #[test]
    fn buy_fill_deducts_cash() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_long_pos("o1", 1.0, 90_000.0));

        let expected = 1_000_000.0 - 90_000.0;
        assert!(
            (portfolio.cash - expected).abs() < 1e-9,
            "Long open 後 cash = {} のはず、実際: {}",
            expected,
            portfolio.cash
        );
    }

    #[test]
    fn short_open_credits_cash() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_short_pos("o1", 1.0, 90_000.0));

        let expected = 1_000_000.0 + 90_000.0;
        assert!(
            (portfolio.cash - expected).abs() < 1e-9,
            "Short open 後 cash = {} のはず（裸ショート）、実際: {}",
            expected,
            portfolio.cash
        );
    }

    // ── A-0: record_close の cash 返還 ───────────────────────────────────────

    #[test]
    fn close_long_returns_sell_proceeds() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_long_pos("o1", 1.0, 90_000.0));
        // cash = 910_000
        portfolio.record_close("o1", 92_000.0, 2_000);
        // cash += 92_000 → 1_002_000

        let expected = 1_000_000.0 - 90_000.0 + 92_000.0;
        assert!(
            (portfolio.cash - expected).abs() < 1e-9,
            "Long close 後 cash = {} のはず、実際: {}",
            expected,
            portfolio.cash
        );
        assert!(
            (portfolio.realized_pnl() - 2_000.0).abs() < 1e-9,
            "realized_pnl = +2000 のはず"
        );
    }

    #[test]
    fn close_short_deducts_buyback_cost() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_short_pos("o1", 1.0, 90_000.0));
        // cash = 1_090_000（売り代金受取）
        portfolio.record_close("o1", 88_000.0, 2_000);
        // cash -= 88_000 → 1_002_000

        let expected = 1_000_000.0 + 90_000.0 - 88_000.0;
        assert!(
            (portfolio.cash - expected).abs() < 1e-9,
            "Short close 後 cash = {} のはず、実際: {}",
            expected,
            portfolio.cash
        );
        assert!(
            (portfolio.realized_pnl() - 2_000.0).abs() < 1e-9,
            "Short realized_pnl = +2000 のはず"
        );
    }

    // ── 既存テスト（A-0+A-1 後も成立することを確認） ─────────────────────────

    #[test]
    fn unrealized_pnl_long_position() {
        let mut portfolio = VirtualPortfolio::new(100_000.0);
        portfolio.record_open(make_long_pos("o1", 1.0, 90_000.0));

        let pnl = portfolio.unrealized_pnl(92_000.0);
        assert!(
            (pnl - 2_000.0).abs() < 1e-9,
            "ロングポジション未実現PnLは +2000 のはず、実際: {pnl}"
        );
    }

    #[test]
    fn unrealized_pnl_short_position() {
        let mut portfolio = VirtualPortfolio::new(100_000.0);
        portfolio.record_open(make_short_pos("o1", 1.0, 90_000.0));

        let pnl = portfolio.unrealized_pnl(88_000.0);
        assert!(
            (pnl - 2_000.0).abs() < 1e-9,
            "ショートポジション未実現PnLは +2000 のはず、実際: {pnl}"
        );
    }

    #[test]
    fn realized_pnl_closes_position() {
        // initial=100_000 → open Long @90_000 → cash=10_000
        // close @92_000 → cash += 92_000 = 102_000
        let mut portfolio = VirtualPortfolio::new(100_000.0);
        portfolio.record_open(make_long_pos("o1", 1.0, 90_000.0));
        portfolio.record_close("o1", 92_000.0, 2_000);

        assert!(
            (portfolio.realized_pnl() - 2_000.0).abs() < 1e-9,
            "実現PnLは +2000 のはず"
        );
        assert!(
            (portfolio.cash - 102_000.0).abs() < 1e-9,
            "cash は 102000 のはず、実際: {}",
            portfolio.cash
        );
    }

    #[test]
    fn snapshot_sums_correctly() {
        let mut portfolio = VirtualPortfolio::new(100_000.0);
        portfolio.record_open(make_long_pos("o1", 1.0, 90_000.0));

        let snap = portfolio.snapshot(92_000.0);
        assert!(
            (snap.total_equity - (snap.cash + snap.unrealized_pnl)).abs() < 1e-9,
            "total_equity = cash + unrealized_pnl でなければならない"
        );
        assert_eq!(snap.open_positions.len(), 1);
        assert_eq!(snap.closed_positions.len(), 0);
    }

    // ── A-1.5: oldest_open_long_order_id ─────────────────────────────────────

    #[test]
    fn oldest_open_long_returns_none_when_empty() {
        let portfolio = VirtualPortfolio::new(1_000_000.0);
        assert_eq!(portfolio.oldest_open_long_order_id("BTCUSDT"), None);
    }

    #[test]
    fn oldest_open_long_returns_oldest() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        // 新しいほうを先に追加（entry_time_ms=2_000）
        portfolio.record_open(make_long_pos_at("newer", 1.0, 90_000.0, 2_000));
        // 古いほうを後に追加（entry_time_ms=1_000）
        portfolio.record_open(make_long_pos_at("older", 1.0, 89_000.0, 1_000));

        assert_eq!(
            portfolio.oldest_open_long_order_id("BTCUSDT"),
            Some("older"),
            "entry_time_ms が最小のポジションを返すはず"
        );
    }

    #[test]
    fn oldest_open_long_ignores_closed() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_long_pos_at("o1", 1.0, 90_000.0, 1_000));
        portfolio.record_close("o1", 92_000.0, 2_000);

        assert_eq!(
            portfolio.oldest_open_long_order_id("BTCUSDT"),
            None,
            "クローズ済みポジションは返さないはず"
        );
    }

    #[test]
    fn oldest_open_long_ignores_short() {
        let mut portfolio = VirtualPortfolio::new(1_000_000.0);
        portfolio.record_open(make_short_pos("o1", 1.0, 90_000.0));

        assert_eq!(
            portfolio.oldest_open_long_order_id("BTCUSDT"),
            None,
            "Short ポジションは Long として返さないはず"
        );
    }
}
