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

    /// 約定時に呼ぶ（open ポジションを追加）。現金はまだ動かさない。
    pub fn record_open(&mut self, pos: Position) {
        self.positions.push(pos);
    }

    /// クローズ時に呼ぶ。実現 PnL を確定し cash に反映する。（Phase 2 で使用予定）
    #[allow(dead_code)]
    pub fn record_close(&mut self, order_id: &str, exit_price: f64, exit_time_ms: u64) {
        if let Some(pos) = self.positions.iter_mut().find(|p| p.order_id == order_id) {
            let pnl = match pos.side {
                PositionSide::Long => (exit_price - pos.entry_price) * pos.qty,
                PositionSide::Short => (pos.entry_price - exit_price) * pos.qty,
            };
            pos.exit_price = Some(exit_price);
            pos.exit_time_ms = Some(exit_time_ms);
            pos.realized_pnl = Some(pnl);
            self.cash += pnl;
        }
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
        self.positions
            .iter()
            .filter_map(|p| p.realized_pnl)
            .sum()
    }

    /// 公開スナップショット（HTTP API の GET /api/replay/portfolio レスポンスに使う）
    pub fn snapshot(&self, current_price: f64) -> PortfolioSnapshot {
        let unrealized = self.unrealized_pnl(current_price);
        let realized = self.realized_pnl();

        let (open, closed) = self.positions.iter().fold(
            (Vec::new(), Vec::new()),
            |(mut open, mut closed), p| {
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
            },
        );

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
}
