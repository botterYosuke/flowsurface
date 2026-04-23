/// 仮想約定エンジン — REPLAYモードでの仮想注文・ポジション管理
///
/// main.rs の App 構造体が `Option<Arc<Mutex<VirtualExchangeEngine>>>` として保持する。
/// HTTP API スレッドとの Arc 共有も main.rs から行う。
pub mod order_book;
pub mod portfolio;

pub use order_book::{
    FillEvent, VirtualOrder, VirtualOrderBook, VirtualOrderStatus, VirtualOrderType,
};
pub use portfolio::{PortfolioSnapshot, PositionSide};

use exchange::Trade;

/// セッションのライフサイクルイベント。agent API の state
/// （`client_order_id` UNIQUE map 等）がこれを購読してリセットする。
/// ADR-0001 の不変条件: UI リモコン API ハンドラから agent API の state を
/// 直接触らず、本イベント経由でのみ伝播させる。
///
/// Phase 4b-1 では値自体は使わず、`session_generation()` の増分を
/// 「イベント発火」とみなして購読側がリセットを判断する
/// （シングルプロセス・シングルスレッド想定の最小実装）。
/// 将来 Phase 4c の multi-session では broadcast channel に置き換える想定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLifecycleEvent {
    /// 新セッション開始（UI `/play` / 将来の agent `start` 両方からトリガ）。
    Started,
    /// seek / step-backward / reset で内部状態が巻き戻った。
    Reset,
    /// toggle → live、アプリ終了。
    Terminated,
}

/// main.rs が保持する仮想約定エンジン全体。
/// `Arc<Mutex<VirtualExchangeEngine>>` で HTTP API スレッドと共有する。
pub struct VirtualExchangeEngine {
    order_book: VirtualOrderBook,
    /// セッション状態遷移の世代カウンタ。`Started` / `Reset` / `Terminated`
    /// のたびに +1 され、agent API 側は「前回観測値と異なる」ことを
    /// 購読イベントのトリガとする。
    session_generation: u64,
}

impl VirtualExchangeEngine {
    pub fn new(initial_cash: f64) -> Self {
        Self {
            order_book: VirtualOrderBook::new(initial_cash),
            session_generation: 0,
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

    /// 内部状態（pending 注文・ポートフォリオ）をクリアする。
    /// **SessionLifecycleEvent は発火しない**。呼び出し側が `mark_session_*`
    /// のいずれかを明示的に呼ぶこと（意図を曖昧にしないため）。
    pub fn reset(&mut self) {
        self.order_book.reset();
    }

    pub fn portfolio_snapshot(&self, current_price: f64) -> PortfolioSnapshot {
        self.order_book.portfolio_snapshot(current_price)
    }

    /// 現在 pending な注文の一覧を返す（HTTP API / UI 表示用）。
    pub fn get_orders(&self) -> &[VirtualOrder] {
        self.order_book.pending_orders()
    }

    /// セッション世代カウンタ。agent API 側がこの値の変化を検知して
    /// SessionLifecycleEvent を購読する。
    pub fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// 明示的にセッション開始を通知する（`/play` から呼ばれる）。
    /// `reset()` は内部状態もクリアするが、こちらは世代のみ進める。
    pub fn mark_session_started(&mut self) {
        self.bump_session_generation(SessionLifecycleEvent::Started);
    }

    /// 明示的に「seek / rewind による巻き戻し」を通知する。
    pub fn mark_session_reset(&mut self) {
        self.bump_session_generation(SessionLifecycleEvent::Reset);
    }

    /// 明示的にセッション終了を通知する（`/toggle` → Live 等）。
    pub fn mark_session_terminated(&mut self) {
        self.bump_session_generation(SessionLifecycleEvent::Terminated);
    }

    fn bump_session_generation(&mut self, event: SessionLifecycleEvent) {
        self.session_generation = self.session_generation.wrapping_add(1);
        log::debug!(
            "VirtualExchange: SessionLifecycleEvent::{event:?} (generation={gen})",
            gen = self.session_generation,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order(ticker: &str) -> VirtualOrder {
        VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: 0,
            status: VirtualOrderStatus::Pending,
        }
    }

    #[test]
    fn get_orders_returns_empty_initially() {
        let engine = VirtualExchangeEngine::new(1_000_000.0);
        assert_eq!(engine.get_orders().len(), 0);
    }

    #[test]
    fn get_orders_returns_pending_after_place() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        assert_eq!(engine.get_orders().len(), 1);
    }

    #[test]
    fn get_orders_returns_multiple_orders() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        engine.place_order(make_order("BTCUSDT"));
        assert_eq!(engine.get_orders().len(), 2);
    }

    #[test]
    fn get_orders_empty_after_reset() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        engine.reset();
        assert_eq!(engine.get_orders().len(), 0);
    }

    // ── SessionLifecycleEvent / generation counter (Phase 4b-1 サブフェーズ E) ──

    #[test]
    fn session_generation_starts_at_zero() {
        let engine = VirtualExchangeEngine::new(1_000_000.0);
        assert_eq!(engine.session_generation(), 0);
    }

    #[test]
    fn mark_session_started_increments_generation() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        let before = engine.session_generation();
        engine.mark_session_started();
        assert_eq!(engine.session_generation(), before + 1);
    }

    #[test]
    fn reset_does_not_change_generation_by_itself() {
        // reset() は内部状態クリアのみ。Lifecycle 発火は呼び出し側が明示する。
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        let before = engine.session_generation();
        engine.reset();
        assert_eq!(engine.session_generation(), before);
    }

    #[test]
    fn mark_session_reset_increments_generation() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        let before = engine.session_generation();
        engine.mark_session_reset();
        assert_eq!(engine.session_generation(), before + 1);
    }

    #[test]
    fn mark_session_terminated_increments_generation() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        let before = engine.session_generation();
        engine.mark_session_terminated();
        assert_eq!(engine.session_generation(), before + 1);
    }

    #[test]
    fn multiple_lifecycle_events_each_increment() {
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.mark_session_started();
        engine.mark_session_reset();
        engine.mark_session_reset();
        engine.mark_session_terminated();
        assert_eq!(engine.session_generation(), 4);
    }

    #[test]
    fn place_order_does_not_change_generation() {
        // 通常 tick 進行・注文発注では agent map のクリアは走らない。
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        let before = engine.session_generation();
        engine.place_order(make_order("BTCUSDT"));
        assert_eq!(engine.session_generation(), before);
    }

    // ── ADR-0001 §4 Reset 不変条件 (サブフェーズ Q) ──────────────────────────

    #[test]
    fn reset_restores_cash_to_initial() {
        // 発注 → 約定で cash が変動した後 reset すると initial_cash に戻る。
        use exchange::Trade;
        use exchange::unit::price::Price;
        use exchange::unit::qty::Qty;
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        let trades = [Trade {
            time: 1,
            is_sell: false,
            price: Price::from_f32(50_000.0),
            qty: Qty::from_f32(0.1),
        }];
        let fills = engine.on_tick("BTCUSDT", &trades, 1);
        assert!(!fills.is_empty(), "約定が発生すること");
        let snap_before = engine.portfolio_snapshot(50_000.0);
        assert!(
            (snap_before.cash - 1_000_000.0).abs() > f64::EPSILON,
            "約定後は cash が変動していること"
        );

        engine.reset();
        let snap_after = engine.portfolio_snapshot(50_000.0);
        assert!(
            (snap_after.cash - 1_000_000.0).abs() < f64::EPSILON,
            "reset 後 cash は initial_cash に戻る: got {}",
            snap_after.cash
        );
    }

    #[test]
    fn reset_clears_all_positions() {
        use exchange::Trade;
        use exchange::unit::price::Price;
        use exchange::unit::qty::Qty;
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        let trades = [Trade {
            time: 1,
            is_sell: false,
            price: Price::from_f32(50_000.0),
            qty: Qty::from_f32(0.1),
        }];
        engine.on_tick("BTCUSDT", &trades, 1);

        let snap_before = engine.portfolio_snapshot(50_000.0);
        assert!(
            !snap_before.open_positions.is_empty() || !snap_before.closed_positions.is_empty(),
            "約定により position が作られていること"
        );

        engine.reset();
        let snap_after = engine.portfolio_snapshot(50_000.0);
        assert!(
            snap_after.open_positions.is_empty(),
            "reset 後 open_positions は空"
        );
        assert!(
            snap_after.closed_positions.is_empty(),
            "reset 後 closed_positions は空"
        );
    }

    #[test]
    fn reset_then_mark_session_reset_clears_orders_and_bumps_generation() {
        // ADR-0001 §4: rewind-to-start 完了時の不変条件。
        // reset() で orders / fills / balance クリア + mark_session_reset() で世代 bump。
        let mut engine = VirtualExchangeEngine::new(1_000_000.0);
        engine.place_order(make_order("BTCUSDT"));
        let gen_before = engine.session_generation();

        engine.reset();
        engine.mark_session_reset();

        assert_eq!(engine.get_orders().len(), 0, "reset 後 pending orders は空");
        assert_eq!(
            engine.session_generation(),
            gen_before + 1,
            "mark_session_reset で generation が進む → AgentSessionState::observe_generation が client_order_id map をクリアできる"
        );
    }
}
