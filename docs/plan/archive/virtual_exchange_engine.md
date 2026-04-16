# 仮想約定エンジン（Virtual Exchange Engine）

## 概要

REPLAYモードで注文パネルを使えるようにし、かつ将来の AI エージェント強化学習基盤（Phase 2）でそのまま流用できる仮想約定エンジンを実装する。

---

## 問題の現状

| 問題 | 詳細 |
|---|---|
| 🚨 **REPLAY中に実際の発注が走る** | `dashboard.rs:466` の `SubmitNewOrder` ハンドラがモード判別せず `order_connector::submit_new_order()` を呼ぶ |
| ⚠️ 注文パネルにReplayガードなし | `is_replay` フラグが `OrderEntryPanel` に届いていない |
| ⛔ 仮想約定エンジン未存在 | Phase 2 で必要な `VirtualExchangeEngine` が未実装 |

---

## 設計方針

### Phase 2 との接続点を意識したアーキテクチャ

```
【現在のLIVEフロー】
OrderEntryPanel → Effect::SubmitNewOrder → order_connector::submit_new_order() → 立花証券API

【実装後のREPLAYフロー】
OrderEntryPanel → Effect::SubmitVirtualOrder → VirtualExchangeEngine::place_order()
                                                      ↓
                                           replay の次 tick で約定判定（on_tick）
                                                      ↓
                                           VirtualPortfolio（PnL管理）

【Phase 2: HTTP API 経由（Pythonエージェント）】
POST /api/replay/order → VirtualExchangeEngine::place_order()  ← 同じエンジンを使う
GET  /api/replay/portfolio → VirtualPortfolio のスナップショット
```

### モジュール配置

```
src/
├── replay/
│   ├── mod.rs              （既存: ReplayMode, ReplaySession, ...）
│   ├── controller.rs       （既存）
│   ├── store.rs            （既存: EventStore）
│   ├── clock.rs            （既存: StepClock）
│   ├── dispatcher.rs       （既存）
│   ├── loader.rs           （既存）
│   └── virtual_exchange/   ← 新規ディレクトリ
│       ├── mod.rs          ← VirtualExchangeEngine（公開エントリーポイント）
│       ├── order_book.rs   ← VirtualOrderBook（注文受付・約定判定）
│       └── portfolio.rs    ← VirtualPortfolio（ポジション・PnL）
└── replay_api.rs           （既存: HTTP API ── エンドポイント追加）
```

### VirtualExchangeEngine のオーナーシップ

`VirtualExchangeEngine` は **`main.rs` の `App` 構造体** に `Option<Arc<Mutex<VirtualExchangeEngine>>>` として配置する。

```
App（main.rs）
  ├── replay: ReplayController
  ├── virtual_engine: Option<Arc<Mutex<VirtualExchangeEngine>>>   ← ここ
  └── replay_api: ReplayApiHandle（既存）
```

**理由**:
- tick 処理が `main.rs` 内（`self.replay.is_replay()` ブロック）で動いており、tick 後に `on_tick()` を呼ぶ自然な場所がここ
- HTTP API スレッドへの `Arc<Mutex<>>` 共有も `main.rs` からが最もシンプル
- `Dashboard` の `update()` は `is_replay` を受け取らないため、Effect ハンドラを `dashboard.rs` 内に置くと判定不可（後述）

---

## フェーズ構成

```
フェーズ A（安全ガード）→ フェーズ B（仮想エンジン・コア）
  → フェーズ C（UI統合）→ フェーズ D（HTTP API）→ フェーズ E（テスト）
```

フェーズ A は **今すぐ必須**（REPLAY中の誤発注防止）。  
フェーズ B〜C が今回の主実装。フェーズ D は Phase 2 の Python SDK が使うエンドポイント。

---

## フェーズ A: 安全ガード（即時対応）

**目的**: REPLAY中に実際の立花証券APIへ発注されないようにする。

### A-0. 前提確認: `is_replay` の受け渡し経路

**重要**: `is_replay` は現在 `dashboard.view()` の引数としてのみ存在し（`dashboard.rs:912`）、  
`dashboard.update()` には届かない。したがって Effect ハンドラを `dashboard.rs` 内でガードするには  
**`Dashboard` 構造体に `is_replay: bool` フィールドを追加**し、`main.rs` から更新する必要がある。

```rust
// src/screen/dashboard.rs
pub struct Dashboard {
    // 既存フィールド ...
    pub is_replay: bool,   // ← 追加
}
```

`main.rs` の replay 状態が切り替わるタイミング（`ReplayUserMessage::ToggleMode` 処理後）で  
`self.active_dashboard_mut().is_replay = self.replay.is_replay();` を呼んでシンクする。

### A-1. `dashboard.rs` の Effect ハンドラに `is_replay` ガード追加

**ファイル**: `src/screen/dashboard.rs`

```rust
pane::Effect::SubmitNewOrder(req) => {
    if self.is_replay {
        // REPLAYモードでは実際の発注をブロック
        // （フェーズ C 完了後に SubmitVirtualOrder に差し替える）
        log::warn!("REPLAY中の発注はブロックされました: {:?}", req);
        Task::none()
    } else {
        let pane_id = state.unique_id();
        Task::perform(
            order_connector::submit_new_order(req),
            move |result| Message::OrderNewResult { pane_id, result },
        )
    }
}
// 訂正・取消も同様にガードする
pane::Effect::SubmitCorrectOrder(req) => {
    if self.is_replay { Task::none() } else { /* 既存 */ }
}
pane::Effect::SubmitCancelOrder(req) => {
    if self.is_replay { Task::none() } else { /* 既存 */ }
}
```

### A-2. REPLAY中の注文パネルに無効化バナーを表示

フェーズ C の `is_virtual` バナー実装まで、REPLAY中は注文パネル上部に  
「⏪ REPLAYモード中 — 注文は無効です」バナーを表示する（テキストのみ、青系）。  
`is_replay` を `view()` 経由で `OrderEntryPanel::view()` に渡す（既に `view()` には届いている）。

**テスト**: フェーズ E で実施。

---

## フェーズ B: 仮想約定エンジン・コア

### B-0. 型変換の前提知識

`exchange::Trade` の `price: Price` / `qty: Qty` はニュータイプ（固定小数点）。  
`f64` との比較・計算には以下の変換を使う：

```rust
// Price → f64（精度損失あり、約定判定には十分）
let price_f64 = trade.price.to_f32_lossy() as f64;

// Qty → f64
let qty_f64 = f64::from(trade.qty.to_f32_lossy());
```

`VirtualOrder` / `VirtualPortfolio` 内の価格・数量はすべて `f64` で保持する。

### B-1. `VirtualPortfolio`（ポジション・PnL 管理）

**ファイル**: `src/replay/virtual_exchange/portfolio.rs`

> **初期実装の制約**: 単一銘柄を前提とする。`unrealized_pnl(current_price: f64)` は  
> 現在価格を1値のみ受け取る。複数銘柄の同時ポジションは Phase 2 で対応する。

```rust
/// 1 注文のポジション
#[derive(Debug, Clone)]
pub struct Position {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,   // Long / Short
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub exit_price: Option<f64>,
    pub exit_time_ms: Option<u64>,
    pub realized_pnl: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PositionSide { Long, Short }

/// ポートフォリオ全体
#[derive(Debug, Default, Clone)]
pub struct VirtualPortfolio {
    pub initial_cash: f64,
    pub cash: f64,
    positions: Vec<Position>,       // open + closed
}

impl VirtualPortfolio {
    pub fn new(initial_cash: f64) -> Self { ... }

    /// 約定時に呼ぶ（open ポジションを追加）
    pub fn record_open(&mut self, pos: Position) { ... }

    /// クローズ時に呼ぶ（実現PnL を確定し cash に反映）
    pub fn record_close(&mut self, order_id: &str, exit_price: f64, exit_time_ms: u64) { ... }

    /// 未実現PnL（現在価格で評価）。単一銘柄を前提とする。
    pub fn unrealized_pnl(&self, current_price: f64) -> f64 { ... }

    /// 実現PnL 合計
    pub fn realized_pnl(&self) -> f64 { ... }

    /// 公開スナップショット（HTTP API の GET /api/replay/portfolio レスポンスに使う）
    pub fn snapshot(&self, current_price: f64) -> PortfolioSnapshot { ... }

    /// リセット（seek / replay 再開時）
    pub fn reset(&mut self) {
        self.cash = self.initial_cash;
        self.positions.clear();
    }
}

/// Phase 2 HTTP API レスポンス形式（そのまま JSON シリアライズ）
#[derive(Debug, Clone, serde::Serialize)]
pub struct PortfolioSnapshot {
    pub cash: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
    pub total_equity: f64,       // cash + unrealized_pnl
    pub open_positions: Vec<PositionSnapshot>,
    pub closed_positions: Vec<PositionSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PositionSnapshot {
    pub order_id: String,
    pub ticker: String,
    pub side: String,            // "Long" / "Short"
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub exit_price: Option<f64>,
    pub realized_pnl: Option<f64>,
}
```

### B-2. `VirtualOrderBook`（注文受付・約定判定）

**ファイル**: `src/replay/virtual_exchange/order_book.rs`

#### 約定タイミングの設計方針

**全注文（成行・指値ともに）を `on_tick()` で約定させる**。`place_order()` 時は `Pending` 状態で登録のみ。

- **成行注文**: `on_tick()` 内でその tick の最初の `Trade` 価格で即時約定
- **指値買い**: `Trade.price <= limit_price` のトレードが来た tick で約定
- **指値売り**: `Trade.price >= limit_price` のトレードが来た tick で約定

この設計により `place_order()` と `on_tick()` の役割が明確に分離される。

#### `on_tick()` のシグネチャ（ticker 引数あり）

`exchange::Trade` には銘柄情報フィールドがないため、`on_tick()` の呼び出し側（`main.rs`）が  
`EventStore::trades_in(stream, range)` から銘柄ごとにトレードを取り出し、`ticker` を一緒に渡す。

```rust
/// UI や HTTP API から受け付ける仮想注文
#[derive(Debug, Clone)]
pub struct VirtualOrder {
    pub order_id: String,          // UUID
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub order_type: VirtualOrderType,
    pub placed_time_ms: u64,       // StepClock::now_ms() で記録
    pub status: VirtualOrderStatus,
}

#[derive(Debug, Clone)]
pub enum VirtualOrderType {
    Market,
    Limit { price: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum VirtualOrderStatus {
    Pending,
    Filled { fill_price: f64, fill_time_ms: u64 },
    Cancelled,
}

/// 注文受付・約定判定エンジン
#[derive(Debug, Default)]
pub struct VirtualOrderBook {
    pending: Vec<VirtualOrder>,
    portfolio: VirtualPortfolio,
}

impl VirtualOrderBook {
    pub fn new(initial_cash: f64) -> Self { ... }

    /// 注文を受け付ける（UI / HTTP API 共通エントリーポイント）
    /// 戻り値: order_id（UUID）。約定は次の on_tick() まで保留。
    pub fn place(&mut self, order: VirtualOrder) -> String { ... }

    /// StepClock が 1 tick 進んだときに呼ぶ。
    ///
    /// # 引数
    /// - `ticker`: このトレード一覧が属する銘柄（StreamKind から呼び出し側が特定して渡す）
    /// - `trades`: EventStore::trades_in(stream, range) で取り出した Trade のスライス
    /// - `now_ms`: StepClock::now_ms()
    ///
    /// # 約定ルール
    /// - 成行注文 → その tick の最初の Trade 価格で約定
    /// - 指値買い  → trade.price.to_f32_lossy() as f64 <= limit_price のとき約定
    /// - 指値売り  → trade.price.to_f32_lossy() as f64 >= limit_price のとき約定
    ///
    /// 戻り値: 約定した注文の一覧（UI 通知用）
    pub fn on_tick(&mut self, ticker: &str, trades: &[Trade], now_ms: u64) -> Vec<FillEvent> { ... }

    /// seek / replay 再開時にリセット
    pub fn reset(&mut self) {
        self.pending.clear();
        self.portfolio.reset();
    }

    /// 現在のポートフォリオスナップショット（HTTP API 用）
    pub fn portfolio_snapshot(&self, current_price: f64) -> PortfolioSnapshot { ... }

    /// UI 表示用の注文一覧
    pub fn orders(&self) -> &[VirtualOrder] { ... }
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
```

### B-3. `VirtualExchangeEngine`（公開エントリーポイント）

**ファイル**: `src/replay/virtual_exchange/mod.rs`

```rust
pub use order_book::{VirtualOrder, VirtualOrderType, VirtualOrderBook, FillEvent};
pub use portfolio::{VirtualPortfolio, PortfolioSnapshot, PositionSide};

/// main.rs が保持する仮想約定エンジン全体。
/// Arc<Mutex<VirtualExchangeEngine>> で HTTP API スレッドと共有する。
pub struct VirtualExchangeEngine {
    order_book: VirtualOrderBook,
}

impl VirtualExchangeEngine {
    pub fn new(initial_cash: f64) -> Self { ... }

    pub fn place_order(&mut self, order: VirtualOrder) -> String {
        self.order_book.place(order)
    }

    /// replay dispatcher から毎 tick 呼ばれる。
    /// ticker: その tick のトレードが属する銘柄（呼び出し側が StreamKind から特定する）
    pub fn on_tick(&mut self, ticker: &str, trades: &[Trade], now_ms: u64) -> Vec<FillEvent> {
        self.order_book.on_tick(ticker, trades, now_ms)
    }

    pub fn reset(&mut self) {
        self.order_book.reset();
    }

    pub fn portfolio_snapshot(&self, current_price: f64) -> PortfolioSnapshot {
        self.order_book.portfolio_snapshot(current_price)
    }
}
```

---

## フェーズ C: UI 統合

### C-1. `OrderEntryPanel` に仮想注文モードを追加

**ファイル**: `src/screen/dashboard/panel/order_entry.rs`

#### is_virtual フラグの追加

```rust
pub struct OrderEntryPanel {
    // 既存フィールド ...
    is_virtual: bool,   // true = REPLAYモード（仮想注文）
}
```

#### view の変更

- `is_virtual == true` のとき:
  - パネル上部に `「⏪ 仮想注文モード」` バナーを表示（青色）
  - `「発注パスワード」` 入力フィールドを非表示
  - 確認モーダルに「仮想発注（資金は消費されません）」と表示
- 注文成功後の `last_result` 表示に `[仮想] 注文番号 XXX 受付` と表示

#### `Message` の追加

```rust
pub enum Message {
    // 既存 ...
    VirtualFilled(FillEvent),   // 約定通知をパネルに届ける（将来のトースト表示用）
}
```

### C-2. `pane.rs` に `Effect::SubmitVirtualOrder` を追加

**ファイル**: `src/screen/dashboard/pane.rs`

```rust
pub enum Effect {
    // 既存 ...
    SubmitVirtualOrder(VirtualOrder),   // REPLAYモード専用
}
```

`update()` 内で `Content::OrderEntry(panel)` の処理を以下のように変更:

```rust
order_entry::Action::Submit(req) => {
    if self.is_virtual_mode {
        let vo = virtual_order_from_new_order_request(&req);
        Some(Effect::SubmitVirtualOrder(vo))
    } else {
        Some(Effect::SubmitNewOrder(req))
    }
}
```

> `Pane` 構造体に `is_virtual_mode: bool` フィールドを追加する必要がある。  
> `dashboard.rs` から `is_replay` に連動して設定する。

### C-3. `main.rs` で `SubmitVirtualOrder` Effect を処理

`SubmitVirtualOrder` Effect は `dashboard.rs` の `update()` から返され、`main.rs` の Effect 処理箇所で受け取る。

```rust
// main.rs の dashboard Effect 処理箇所
pane::Effect::SubmitVirtualOrder(vo) => {
    if let Some(engine) = &self.virtual_engine {
        let mut eng = engine.blocking_lock();
        let _order_id = eng.place_order(vo);
        // 約定は次の on_tick() で行われる
    }
    Task::none()
}
```

### C-4. `main.rs` の replay tick 処理に `on_tick` フック追加

**ファイル**: `src/main.rs`（`self.replay.is_replay()` ブロック内）

リプレイが 1 tick 進むたびに `VirtualExchangeEngine::on_tick()` を呼び、  
`FillEvent` が返ってきたら Dashboard 経由でトースト通知を発行する。

```rust
// main.rs の tick 処理末尾（dispatch_tick の後）
if let Some(engine) = &self.virtual_engine {
    let mut eng = engine.lock().await;  // または blocking_lock()
    for (ticker, trades) in tick_trades_by_ticker {
        // tick_trades_by_ticker: EventStore から銘柄別に集めたトレード
        let fills = eng.on_tick(&ticker, &trades, clock.now_ms());
        for fill in fills {
            // dashboard::Message::VirtualOrderFilled(fill) を発行
            // → Toast 通知「[仮想] 約定: BTC 0.1 @ 92,500 (+$230)」
        }
    }
}
```

`dashboard::Message` に `VirtualOrderFilled(FillEvent)` バリアントを追加する。

### C-5. seek / replay 再開時のリセット

`VirtualExchangeEngine::reset()` を呼ぶタイミングは以下の3箇所：

| トリガー | コードパス |
|---|---|
| リプレイ新規開始（Play） | `ReplayUserMessage::Play` 処理後 |
| 時間を巻き戻す（StepBackward） | `ReplayUserMessage::StepBackward` 処理後 |
| REPLAYモード終了（Live へ切替） | `ReplayUserMessage::ToggleMode` で `was_replay && !is_replay` の分岐 |

`main.rs` の `ReplayController::handle_user_message()` 呼び出し後に、上記ケースを判定して `reset()` を呼ぶ。  
**注意**: ポジション・PnL がリセットされることを UI で明示する（バナー表示で十分、確認ダイアログは不要）。

---

## フェーズ D: HTTP API 追加（Phase 2 互換）

**ファイル**: `src/replay_api.rs`

`VirtualExchangeEngine` を `Arc<tokio::sync::Mutex<VirtualExchangeEngine>>` で HTTP API スレッドと共有する。  
共有元は `main.rs`（`App` 構造体）。`ReplayApiHandle` 初期化時に `Arc` を渡す。

### D-1. `POST /api/replay/order`

```
リクエスト:
{
  "ticker": "BTCUSDT",
  "side": "buy",          // "buy" | "sell"
  "qty": 0.1,
  "order_type": "market"  // "market" | { "limit": 92500.0 }
}

レスポンス:
{ "order_id": "uuid-string", "status": "pending" }
```

Python SDK からの `env.step(action)` の実装が `POST /api/replay/order` を叩く。

### D-2. `GET /api/replay/portfolio`

```
レスポンス:
{
  "cash": 100000.0,
  "unrealized_pnl": 230.5,
  "realized_pnl": 1200.0,
  "total_equity": 101430.5,
  "open_positions": [...],
  "closed_positions": [...]
}
```

`PortfolioSnapshot` をそのまま JSON シリアライズして返す。

### D-3. `GET /api/replay/state`（Phase 1 対応、観測データ追加）

既存の `/api/replay/status` とは別に、エージェントが市場データを取得するエンドポイント。  
ロードマップ Phase 1 で追加予定。このフェーズでは骨格のみ追加し、実装は Phase 1 着手時に行う。

```
GET /api/replay/state
→ { "current_time_ms": 1704067200000, "ohlcv": [...], "recent_trades": [...] }
```

---

## フェーズ E: テスト

### E-1. `VirtualOrderBook` ユニットテスト

**ファイル**: `src/replay/virtual_exchange/order_book.rs` の `#[cfg(test)]`

| テストケース | 確認内容 |
|---|---|
| `market_order_fills_on_next_tick` | 成行注文が `on_tick()` 呼び出し時のトレード価格で約定する |
| `limit_buy_fills_when_trade_below_price` | 指値買いがトレード価格 <= 指値で約定する |
| `limit_sell_fills_when_trade_above_price` | 指値売りがトレード価格 >= 指値で約定する |
| `limit_not_filled_when_price_not_met` | 指値未達では約定しない |
| `portfolio_pnl_after_round_trip` | 買い→売りで PnL が正しく計算される |
| `reset_clears_pending_and_portfolio` | `reset()` で注文・ポジション・cash がクリアされる |
| `on_tick_ignores_wrong_ticker` | ticker が一致しない pending 注文には約定しない |

### E-2. `VirtualPortfolio` ユニットテスト

| テストケース | 確認内容 |
|---|---|
| `unrealized_pnl_long_position` | ロングポジションの未実現PnL（値上がり） |
| `unrealized_pnl_short_position` | ショートポジションの未実現PnL（値下がり） |
| `realized_pnl_closes_position` | クローズで cash が増える |
| `snapshot_sums_correctly` | `total_equity = cash + unrealized_pnl` が正確 |

### E-3. フェーズ A のガード統合テスト

`is_replay = true` のとき `SubmitNewOrder` Effect が `order_connector` を呼ばないことを確認。

---

## タスクチェックリスト

### フェーズ A: 安全ガード
- ✅ `Dashboard` 構造体に `is_replay: bool` フィールドを追加
- ✅ `main.rs` で `ReplayUserMessage::ToggleMode` 処理後に `dashboard.is_replay` をシンク
- ✅ `dashboard.rs` の `SubmitNewOrder` / `SubmitCorrectOrder` / `SubmitCancelOrder` ハンドラに `is_replay` ガード追加
- ✅ REPLAYモード中に注文パネルに「REPLAYモード中 — 注文は無効です」バナーを表示

### フェーズ B: 仮想エンジン・コア
- ✅ `src/replay/virtual_exchange/` ディレクトリ作成
- ✅ `portfolio.rs`: `VirtualPortfolio` / `PortfolioSnapshot` / `PositionSnapshot` 実装
- ✅ `order_book.rs`: `VirtualOrderBook` / `VirtualOrder` / `FillEvent` 実装（成行・指値の約定ルール、`on_tick(ticker, trades, now_ms)` シグネチャ）
- ✅ `mod.rs`: `VirtualExchangeEngine` 公開エントリーポイント実装
- ✅ `src/replay/mod.rs` に `pub mod virtual_exchange;` 追記
- ✅ `cargo test --workspace` で全テスト通過確認

### フェーズ C: UI 統合
- ✅ `OrderEntryPanel` に `is_virtual` フラグ追加・view に仮想モードバナー表示
- ✅ `Pane` 構造体に `is_virtual_mode: bool` フィールドを追加
- ✅ `pane.rs` に `Effect::SubmitVirtualOrder` 追加
- ✅ `dashboard::Message` に `VirtualOrderFilled(FillEvent)` バリアントを追加
- ✅ `main.rs` の `App` 構造体に `virtual_engine: Option<Arc<Mutex<VirtualExchangeEngine>>>` 追加
- ✅ `main.rs` の Effect 処理箇所で `SubmitVirtualOrder` を処理（`place_order()` 呼び出し）
- ✅ `main.rs` の replay tick 処理に `on_tick(ticker, trades, now_ms)` フック追加
- ✅ `FillEvent` → トースト通知「[仮想] 約定: XXX @ YYY」
- ✅ `main.rs` の Play / ToggleMode 処理後に `reset()` を呼ぶ（StepBackward は clock 巻き戻し時に自動リセット）
- ✅ `cargo clippy -- -D warnings` 通過確認

### フェーズ D: HTTP API
- ✅ `VirtualExchangeEngine` を `Arc<Mutex<...>>` でラップして main.rs に保持
- ✅ `POST /api/replay/order` エンドポイント実装
- ✅ `GET /api/replay/portfolio` エンドポイント実装
- ✅ `GET /api/replay/state` の骨格追加（レスポンス: `{ "current_time_ms": ..., "not_implemented": true }`）

### フェーズ E: テスト
- ✅ `VirtualOrderBook` ユニットテスト 7 件（ticker 不一致テスト含む）
- ✅ `VirtualPortfolio` ユニットテスト 4 件
- ✅ `cargo test --workspace` 全テスト通過（259 passed）
- ✅ `cargo clippy -- -D warnings` 通過

---

## ファイル変更サマリー

| ファイル | 変更種別 |
|---|---|
| `src/replay/virtual_exchange/mod.rs` | **新規** `VirtualExchangeEngine` |
| `src/replay/virtual_exchange/order_book.rs` | **新規** `VirtualOrderBook` / `VirtualOrder` / `FillEvent` |
| `src/replay/virtual_exchange/portfolio.rs` | **新規** `VirtualPortfolio` / `PortfolioSnapshot` |
| `src/replay/mod.rs` | `pub mod virtual_exchange;` 追記 |
| `src/replay_api.rs` | `POST /api/replay/order` / `GET /api/replay/portfolio` 追加、`Arc<Mutex<VirtualExchangeEngine>>` 受け取り |
| `src/main.rs` | `virtual_engine: Option<Arc<Mutex<VirtualExchangeEngine>>>` 追加、tick フック追加、reset 呼び出し追加 |
| `src/screen/dashboard.rs` | `is_replay: bool` フィールド追加、`is_replay` ガード追加、`VirtualOrderFilled` ハンドラ追加 |
| `src/screen/dashboard/pane.rs` | `Effect::SubmitVirtualOrder` 追加、`Pane` 構造体に `is_virtual_mode: bool` 追加 |
| `src/screen/dashboard/panel/order_entry.rs` | `is_virtual` フラグ追加・仮想モードバナー表示・パスワード非表示 |

---

## 設計上の注意点

### 約定判定における型変換

`Trade.price: Price` / `Trade.qty: Qty` は固定小数点ニュータイプ。  
約定判定（f64 比較）には `trade.price.to_f32_lossy() as f64` を使う。  
精度損失はトレード価格の比較には許容範囲内。

### REPLAYの seek と PnL リセット

seek（時間を巻き戻す操作）が行われたとき、ポジション・PnL・保留注文はすべてリセットする。  
**理由**: 仮想ポジションは「その時系列における行動の結果」であるため、時間を戻すと因果関係が崩れる。

Phase 2 の強化学習サイクルでも `env.reset()` が `VirtualExchangeEngine::reset()` に対応する。

### 初期 cash の設定

`VirtualPortfolio::new(initial_cash)` の値は初期実装では **ハードコード（例: `1_000_000.0`）** で問題ない。  
将来フェーズで設定ファイルまたは HTTP API のパラメータから渡せるようにする。

### 単一銘柄制約（Phase 1）

`VirtualPortfolio::unrealized_pnl(current_price: f64)` は単一価格のみ受け取る。  
複数銘柄の同時ポジションは Phase 2 以降で `HashMap<ticker, price>` に拡張する。  
Phase 1 では UI 上で複数銘柄に発注できても PnL 計算が正確でないことをログで警告する。

### Phase 2 との型の互換性

`VirtualOrder` / `FillEvent` / `PortfolioSnapshot` は `serde::Serialize` / `Deserialize` を derive し、  
HTTP API の JSON レスポンスとして直接使える設計にしておく。  
Python SDK 側は `PortfolioSnapshot` の JSON をそのまま `obs`（観測データ）として使う。
