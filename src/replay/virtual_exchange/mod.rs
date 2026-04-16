/// 仮想約定エンジン — REPLAYモードでの仮想注文・ポジション管理
///
/// main.rs の App 構造体が `Option<Arc<Mutex<VirtualExchangeEngine>>>` として保持する。
/// HTTP API スレッドとの Arc 共有も main.rs から行う。
pub mod order_book;
pub mod portfolio;

pub use order_book::{FillEvent, VirtualOrder, VirtualOrderBook, VirtualOrderStatus, VirtualOrderType};
pub use portfolio::{PortfolioSnapshot, PositionSide};

use exchange::Trade;

/// main.rs が保持する仮想約定エンジン全体。
/// `Arc<Mutex<VirtualExchangeEngine>>` で HTTP API スレッドと共有する。
pub struct VirtualExchangeEngine {
    order_book: VirtualOrderBook,
}

impl VirtualExchangeEngine {
    pub fn new(initial_cash: f64) -> Self {
        Self {
            order_book: VirtualOrderBook::new(initial_cash),
        }
    }

    /// 注文を受け付ける（UI / HTTP API 共通エントリーポイント）。
    /// 戻り値: order_id（UUID）。約定は次の on_tick() で行う。
    pub fn place_order(&mut self, order: VirtualOrder) -> String {
        self.order_book.place(order)
    }

    /// replay dispatcher から毎 tick 呼ばれる。
    ///
    /// # 引数
    /// - `ticker`: その tick のトレードが属する銘柄（呼び出し側が StreamKind から特定する）
    /// - `trades`: EventStore から取り出した Trade のスライス
    /// - `now_ms`: StepClock::now_ms()
    ///
    /// 戻り値: 約定した注文の一覧（UI 通知用）
    pub fn on_tick(&mut self, ticker: &str, trades: &[Trade], now_ms: u64) -> Vec<FillEvent> {
        self.order_book.on_tick(ticker, trades, now_ms)
    }

    /// seek / replay 再開時にリセット
    pub fn reset(&mut self) {
        self.order_book.reset();
    }

    pub fn portfolio_snapshot(&self, current_price: f64) -> PortfolioSnapshot {
        self.order_book.portfolio_snapshot(current_price)
    }
}
