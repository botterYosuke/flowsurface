# 注文機能仕様書

## 概要

flowsurface に立花証券 e 支店 API を使った注文機能を追加した。
**注文発注がメイン**。照会・余力確認はそれを支える補助機能。

| ウィンドウ / 機能 | 役割 | 使用 API |
|---|---|---|
| **注文入力パネル** ★メイン | 買い・売り注文の入力と発注 | CLMKabuNewOrder |
| **注文訂正・取消** | 発注済み注文の変更・キャンセル | CLMKabuCorrectOrder / CLMKabuCancelOrder |
| **注文約定照会パネル** | 注文一覧と約定状況の確認 | CLMOrderList / CLMOrderListDetail |
| **余力情報パネル** | 買付可能額・保証金率の確認 | CLMZanKaiKanougaku / CLMZanShinkiKanoIjiritu |
| **保有株数取得** | 売り注文時の全数量ボタン | CLMGenbutuKabuList |
| **仮想注文モード (REPLAY)** | REPLAY 中の疑似発注・PnL トラッキング | 内部エンジン（証券 API 不使用） |
| **サイドバー注文ボタン** | 注文パネルを開くエントリーポイント | UI のみ（API なし） |

---

## サイドバー注文ボタン

### 概要

注文パネルを開くためのエントリーポイント。サイドバーに専用の「注文」ボタン（鉛筆アイコン）を追加した。

### ボタン配置

```
虫眼鏡 (Search)  ← ティッカーテーブル展開
注文   (Order)   ← 注文パネル選択リスト展開  ★新規
レイアウト
音量
───（スペーサー）───
設定
```

### 動作

1. 注文ボタンをクリックするとインラインパネルが展開する:

   ```
   [ Order Entry  ]
   [ Order List   ]
   [ Buying Power ]
   ```

2. いずれかを選択すると、フォーカスペインを **Horizontal Split** して新ペインに選択した種類を直接表示する。

3. **注文パネルとティッカーテーブルは相互排他**: 一方を開くと他方が閉じる。

### 実装ファイル

| ファイル | 変更内容 |
|---|---|
| `data/src/config/sidebar.rs` | `Menu::Order` バリアント追加（`#[serde(skip)]` のため永続化に影響なし） |
| `src/screen/dashboard/sidebar.rs` | `Message::OrderPaneSelected` / `Action::OpenOrderPane` 追加、注文ボタン・インラインパネル・相互排他ロジック |
| `src/screen/dashboard.rs` | `split_focused_and_init_order()` 追加、`auto_focus_single_pane()` 切り出し |
| `src/main.rs` | `Action::OpenOrderPane` ハンドラ・`Menu::Order` アーム追加 |

### `split_focused_and_init_order`

`TickerInfo` 不要（`SyncIssueToOrderEntry` で自動連動）のため、`set_content_and_streams` を使わず `Content::placeholder()` で直接初期化する。

```rust
/// フォーカスペインを Horizontal Split し、新ペインを指定の注文パネルで初期化する。
pub fn split_focused_and_init_order(
    &mut self,
    main_window: window::Id,
    content_kind: data::layout::pane::ContentKind,
) -> Task<Message>
```

- フォーカスなし・単一ペイン → 自動フォーカス後に Split
- フォーカスなし・複数ペイン → `Toast::warn("No focused pane found")`
- `panes.split()` 失敗 → `Toast::warn("Could not split pane")`

### `auto_focus_single_pane`（プライベートヘルパー）

`split_focused_and_init` / `split_focused_and_init_order` / `switch_tickers_in_group` で共通化された「フォーカスなし単一ペイン時の自動フォーカス」ロジック。

---

## API 仕様サマリー

### CLMKabuNewOrder（新規注文）

**リクエスト主要フィールド:**

| フィールド | 説明 | 値 |
|---|---|---|
| `sZyoutoekiKazeiC` | 口座区分 | 1=特定, 3=一般, 5=NISA, 6=N成長 |
| `sIssueCode` | 銘柄コード | 例: "8411" |
| `sSizyouC` | 市場 | "00"=東証 |
| `sBaibaiKubun` | 売買区分 | 1=売, 3=買, 5=現渡, 7=現引 |
| `sCondition` | 執行条件 | 0=指定なし, 2=寄付, 4=引け, 6=不成 |
| `sOrderPrice` | 注文値段 | "*"=指定なし, "0"=成行, 数値=指値 |
| `sOrderSuryou` | 注文株数 | 例: "100" |
| `sGenkinShinyouKubun` | 現物/信用区分 | 0=現物, 2=信用新規(制度6ヶ月), 4=信用返済(制度), 6=信用新規(一般), 8=信用返済(一般) |
| `sOrderExpireDay` | 注文期日 | "0"=当日, YYYYMMDD=期日指定(10営業日以内) |
| `sGyakusasiOrderType` | 逆指値種別 | 0=通常（現在は "0" 固定） |
| `sGyakusasiZyouken` | 逆指値条件（トリガー価格） | "0"=指定なし（現在は固定） |
| `sGyakusasiPrice` | 逆指値値段 | "*"=指定なし（現在は固定） |
| `sSecondPassword` | **第二パスワード（必須）** | 発注パスワード |

> **逆指値フィールド**は `serialize_order_request()` 内で常に通常注文の固定値を付与する。
> 逆指値 UI は将来フェーズで追加する。

**レスポンス主要フィールド:**

| フィールド | 説明 |
|---|---|
| `sResultCode` | 0=正常, その他=エラー |
| `sResultText` | エラーテキスト |
| `sWarningCode` | 警告コード（0=なし） |
| `sWarningText` | 警告テキスト |
| `sOrderNumber` | 採番された注文番号 |
| `sEigyouDay` | 営業日 (YYYYMMDD) |
| `sOrderUkewatasiKingaku` | 注文受渡金額 |
| `sOrderTesuryou` | 注文手数料 |
| `sOrderSyouhizei` | 注文消費税 |
| `sKinri` | 金利（現物時は "-"） |
| `sOrderDate` | 注文日時 (YYYYMMDDHHMMSS) |

### CLMKabuCorrectOrder（訂正注文）

| フィールド | 説明 |
|---|---|
| `sOrderNumber` | 訂正する注文番号 |
| `sEigyouDay` | 営業日 |
| `sCondition` | "*"=変更なし, その他=変更後の執行条件 |
| `sOrderPrice` | "*"=変更なし, "0"=成行変更, 数値=変更後の値段 |
| `sOrderSuryou` | "*"=変更なし, 数値=変更後の株数（増株不可） |
| `sOrderExpireDay` | "*"=変更なし, "0"=当日, YYYYMMDD=変更後の期日 |
| `sSecondPassword` | **第二パスワード（必須）** |

### CLMKabuCancelOrder（取消注文）

| フィールド | 説明 |
|---|---|
| `sOrderNumber` | 取消する注文番号 |
| `sEigyouDay` | 営業日 |
| `sSecondPassword` | **第二パスワード（必須）** |

### CLMOrderList（注文一覧）

**リクエストフィールド:**

| フィールド | 説明 |
|---|---|
| `sIssueCode` | 銘柄コード（空文字=全銘柄） |
| `sSikkouDay` | 執行予定日 (YYYYMMDD) |
| `sOrderSyoukaiStatus` | 照会状態（""=全件, 1〜5=状態指定） |

**レスポンス配列 `aOrderList` の主要フィールド:**

| フィールド | 説明 |
|---|---|
| `sOrderOrderNumber` | 注文番号 |
| `sOrderIssueCode` | 銘柄コード |
| `sOrderOrderSuryou` | 注文株数 |
| `sOrderCurrentSuryou` | 有効株数 |
| `sOrderOrderPrice` | 注文単価 |
| `sOrderOrderDateTime` | 注文日時 (YYYYMMDDHHMMSS) |
| `sOrderStatus` | 状態名称（テキスト） |
| `sOrderYakuzyouSuryo` | 約定株数 |
| `sOrderYakuzyouPrice` | 約定単価 |
| `sOrderEigyouDay` | 営業日（暫定フィールド名。実 API で確認要） |

> **⚠️ 要注意**: `sOrderEigyouDay` は暫定フィールド名。実際の CLMOrderList レスポンスで
> 別のフィールド名が使われている場合、`eig_day` が `""` になり訂正・取消 API 呼び出し時に
> エラーが発生する。実注文照会のログで確認が必要。

### CLMOrderListDetail（約定明細）

**リクエストフィールド:**

| フィールド | 説明 |
|---|---|
| `sOrderNumber` | 注文番号 |
| `sEigyouDay` | 営業日 (YYYYMMDD) |

**レスポンス `aYakuzyouSikkouList` の主要フィールド:**

| フィールド | 説明 |
|---|---|
| `sYakuzyouSuryou` | 約定株数 |
| `sYakuzyouPrice` | 約定単価 |
| `sYakuzyouDate` | 約定日時 |

### CLMZanKaiKanougaku（現物買付余力）

| フィールド | 説明 |
|---|---|
| `sSummaryGenkabuKaituke` | 株式現物買付可能額 |
| `sSummaryNseityouTousiKanougaku` | NISA成長投資可能額 |
| `sHusokukinHasseiFlg` | 不足金発生フラグ |

### CLMZanShinkiKanoIjiritu（信用新規可能委託保証金率）

| フィールド | 説明 |
|---|---|
| `sSummarySinyouSinkidate` | 信用新規建可能額 |
| `sItakuhosyoukin` | 委託保証金率(%) |
| `sOisyouKakuteiFlg` | 追証フラグ（0=なし, 1=確定） |

### CLMGenbutuKabuList（現物保有株数取得）

**リクエストフィールド:**

| フィールド | 説明 |
|---|---|
| `sIssueCode` | 銘柄コード（指定時=1銘柄, 空文字=全保有銘柄） |

**レスポンス配列 `aGenbutuKabuList` の主要フィールド:**

| フィールド | 説明 |
|---|---|
| `sUriOrderIssueCode` | 銘柄コード |
| `sUriOrderZanKabuSuryou` | 残高株数（保有数量） |
| `sUriOrderUritukeKanouSuryou` | 売付可能株数 |

---

## アーキテクチャ

### リクエスト送信の仕組み

全注文 API は `serialize_order_request()` ヘルパーを通じて送信する。

```rust
// exchange/src/adapter/tachibana.rs
// p_no / p_sd_date / sCLMID / sJsonOfmt を動的にマージして URL クエリに付加する
pub fn serialize_order_request<T: Serialize>(
    clm_id: &'static str,
    req: &T,
) -> Result<String, TachibanaError>
```

各リクエスト struct は業務フィールドのみを持ち、共通フィールド（`p_no` 等）は
`serialize_order_request()` が自動付与する。これは既存の `build_api_url_from()` とは
別の方式として注文 API 向けに実装された。

### レスポンスのエラー判定

全レスポンスは `ApiResponse<T>` でラップし、`check()` でエラー判定する。

```rust
let resp: ApiResponse<NewOrderResponse> = serde_json::from_str(&body)?;
let data = resp.check()?;  // sResultCode != "0" → TachibanaError
```

### 営業日の管理

`dashboard.rs` が `Option<String>` の `eig_day` フィールドを保持する。
初回 `submit_new_order()` 成功後に `NewOrderResponse.eig_day` をセットする。
発注前に注文照会を開いた場合は `eig_day_or_today()` でローカル時計から当日日付を生成する。

```rust
// dashboard.rs
fn eig_day_or_today(&self) -> String {
    self.eig_day.clone().unwrap_or_else(|| {
        chrono::Local::now().format("%Y%m%d").to_string()
    })
}
```

---

## API 型定義（`exchange` クレート）

**ファイル**: `exchange/src/adapter/tachibana.rs`

### 注文発注 API 型

```rust
// 新規注文リクエスト（業務フィールドのみ。共通フィールドは serialize_order_request() が付与）
// second_password を含むため #[derive(Debug)] 不可 → Debug を手動実装
#[derive(Clone, Serialize)]
pub struct NewOrderRequest {
    #[serde(rename = "sZyoutoekiKazeiC")]
    pub account_type: String,           // 1=特定, 3=一般, 5=NISA, 6=N成長
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,
    #[serde(rename = "sSizyouC")]
    pub market_code: String,            // "00"=東証
    #[serde(rename = "sBaibaiKubun")]
    pub side: String,                   // 1=売, 3=買
    #[serde(rename = "sCondition")]
    pub condition: String,              // "0"=指定なし, "2"=寄付, "4"=引け, "6"=不成
    #[serde(rename = "sOrderPrice")]
    pub price: String,                  // "0"=成行, 数値=指値
    #[serde(rename = "sOrderSuryou")]
    pub qty: String,
    #[serde(rename = "sGenkinShinyouKubun")]
    pub cash_margin: String,            // "0"=現物, "2"=信用新規(制度), ...
    #[serde(rename = "sOrderExpireDay")]
    pub expire_day: String,             // "0"=当日
    #[serde(rename = "sSecondPassword")]
    pub second_password: String,
}

// Debug を手動実装（second_password をマスク）
impl std::fmt::Debug for NewOrderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewOrderRequest")
            .field("issue_code", &self.issue_code)
            .field("side", &self.side)
            .field("price", &self.price)
            .field("qty", &self.qty)
            .field("second_password", &"[REDACTED]")
            .finish()
    }
}

// 新規注文レスポンス（ApiResponse<NewOrderResponse> でラップして使用）
#[derive(Debug, Clone, Deserialize)]
pub struct NewOrderResponse {
    #[serde(rename = "sOrderNumber", default)]
    pub order_number: String,
    #[serde(rename = "sEigyouDay", default)]
    pub eig_day: String,
    #[serde(rename = "sOrderUkewatasiKingaku", default)]
    pub delivery_amount: String,
    #[serde(rename = "sOrderTesuryou", default)]
    pub commission: String,
    #[serde(rename = "sOrderSyouhizei", default)]
    pub consumption_tax: String,
    #[serde(rename = "sKinri", default)]
    pub interest: String,               // 現物時は "-"
    #[serde(rename = "sOrderDate", default)]
    pub order_datetime: String,         // YYYYMMDDHHMMSS
    #[serde(rename = "sWarningCode", default)]
    pub warning_code: String,
    #[serde(rename = "sWarningText", default)]
    pub warning_text: String,
}

// 訂正注文リクエスト（second_password を含むため Debug 手動実装）
#[derive(Clone, Serialize)]
pub struct CorrectOrderRequest { ... }

// 取消注文リクエスト（second_password を含むため Debug 手動実装）
#[derive(Clone, Serialize)]
pub struct CancelOrderRequest { ... }

// 訂正・取消共通レスポンス（ApiResponse<ModifyOrderResponse> でラップして使用）
#[derive(Debug, Clone, Deserialize)]
pub struct ModifyOrderResponse {
    pub order_number: String,
    pub eig_day: String,
    pub order_datetime: String,
}
```

> **`Clone` の要求**: `Effect` enum が `Debug + Clone` を要求するため、
> 全リクエスト / レスポンス struct に `Clone` が必要。

> **`Box<NewOrderRequest>`**: `NewOrderRequest` が 240 bytes のため、
> `Action::Submit(Box<NewOrderRequest>)` として boxed にする（clippy 警告対応）。

### 注文照会 API 型

```rust
// 注文一覧リクエスト
#[derive(Debug, Clone, Serialize)]
pub struct OrderListRequest {
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,             // 空文字=全銘柄
    #[serde(rename = "sSikkouDay")]
    pub sikkou_day: String,             // YYYYMMDD（当日営業日）
    #[serde(rename = "sOrderSyoukaiStatus")]
    pub status_filter: String,          // ""=全件
}

// 注文一覧レスポンス
#[derive(Debug, Deserialize)]
pub struct OrderListResponse {
    #[serde(rename = "aOrderList", default)]
    pub orders: Vec<OrderRecord>,
}

// 注文レコード (1件)
#[derive(Debug, Clone, Deserialize)]
pub struct OrderRecord {
    #[serde(rename = "sOrderOrderNumber", default)]
    pub order_num: String,
    #[serde(rename = "sOrderIssueCode", default)]
    pub issue_code: String,
    #[serde(rename = "sOrderOrderSuryou", default)]
    pub order_qty: String,
    #[serde(rename = "sOrderCurrentSuryou", default)]
    pub current_qty: String,            // 有効株数
    #[serde(rename = "sOrderOrderPrice", default)]
    pub order_price: String,
    #[serde(rename = "sOrderOrderDateTime", default)]
    pub order_datetime: String,         // YYYYMMDDHHMMSS
    #[serde(rename = "sOrderStatus", default)]
    pub status_text: String,            // 状態名称（テキスト）
    #[serde(rename = "sOrderYakuzyouSuryo", default)]
    pub executed_qty: String,
    #[serde(rename = "sOrderYakuzyouPrice", default)]
    pub executed_price: String,
    // ⚠️ 暫定: 実際のフィールド名は実 API レスポンスで確認が必要
    #[serde(rename = "sOrderEigyouDay", default)]
    pub eig_day: String,
}

impl OrderRecord {
    // 取消可能判定（状態テキストで判定）
    pub fn is_cancelable(&self) -> bool {
        matches!(
            self.status_text.as_str(),
            "受付中" | "注文中" | "一部約定"
        )
    }
}
```

### 余力情報 API 型

```rust
// 現物買付余力レスポンス
#[derive(Debug, Clone, Deserialize)]
pub struct BuyingPowerResponse {
    #[serde(rename = "sSummaryGenkabuKaituke", default)]
    pub cash_buying_power: String,
    #[serde(rename = "sSummaryNseityouTousiKanougaku", default)]
    pub nisa_growth_buying_power: String,
    #[serde(rename = "sHusokukinHasseiFlg", default)]
    pub shortage_flag: String,
}

// 信用新規可能委託保証金率レスポンス
#[derive(Debug, Clone, Deserialize)]
pub struct MarginPowerResponse {
    #[serde(rename = "sSummarySinyouSinkidate", default)]
    pub margin_new_order_power: String,
    #[serde(rename = "sItakuhosyoukin", default)]
    pub maintenance_margin_rate: String,
    #[serde(rename = "sOisyouKakuteiFlg", default)]
    pub margin_call_flag: String,       // "0"=なし, "1"=確定
}
```

### 保有株数 API 型

```rust
// 現物保有レスポンス
#[derive(Debug, Deserialize)]
pub struct GenbutuKabuResponse {
    #[serde(rename = "aGenbutuKabuList", default)]
    pub holdings: Vec<HoldingRecord>,
}

// 保有レコード (1件)
#[derive(Debug, Deserialize)]
pub struct HoldingRecord {
    #[serde(rename = "sUriOrderIssueCode", default)]
    pub issue_code: String,
    #[serde(rename = "sUriOrderZanKabuSuryou", default)]
    pub holding_qty: String,            // 残高株数
    #[serde(rename = "sUriOrderUritukeKanouSuryou", default)]
    pub sellable_qty: String,           // 売付可能株数
}
```

---

## data クレート（`data/src/layout/pane.rs`）

### ContentKind 拡張

```rust
pub enum ContentKind {
    // チャート・パネル系 8 種類
    Starter,
    HeatmapChart,
    ShaderHeatmap,
    FootprintChart,
    CandlestickChart,
    ComparisonChart,
    TimeAndSales,
    Ladder,
    // 注文系（ALL には含まない — サイドバー注文ボタン経由のみで開く）
    OrderEntry,
    OrderList,
    BuyingPower,
}

// ALL 定数: 8 要素（注文系 3 種は除外 — Starter ペインの pick_list に表示しない）
pub const ALL: [ContentKind; 8] = [
    ContentKind::Starter,
    ContentKind::HeatmapChart,
    ContentKind::ShaderHeatmap,
    ContentKind::FootprintChart,
    ContentKind::CandlestickChart,
    ContentKind::ComparisonChart,
    ContentKind::TimeAndSales,
    ContentKind::Ladder,
];
```

> **注文系バリアントを `ALL` から除外した理由**: 注文パネルはサイドバー注文ボタンが唯一の開き方であるため、Starter ペインの pick_list に表示する必要がなくなった。`Pane` enum のバリアントとしては残るため、保存済みレイアウト JSON のデシリアライズには影響しない。

### Pane enum 拡張

注文パネルは設定（stream / indicators）を持たないため **ユニット variant** として追加。
ユニット variant の追加は既存の保存済みレイアウト JSON のデシリアライズに影響しない。

```rust
pub enum Pane {
    // 既存...
    OrderEntry,
    OrderList,
    BuyingPower,
}
```

---

## 注文入力パネル（`src/screen/dashboard/panel/order_entry.rs`）

`Panel` trait（`canvas::Program` ベース）は実装しない。
`view() -> Element<Message>` と `update(msg: Message) -> Option<Action>` を直接定義する。

### UI レイアウト

```
┌─────────────────────────────────┐
│  [買い]  [売り]                  │  売買区分タブ
├─────────────────────────────────┤
│  銘柄: [7203 トヨタ自動車]       │  銘柄表示（チャートペインと連動）
│─────────────────────────────────│
│  口座: [特定 ▼]                  │  口座区分
│  数量: [____100____] [全数量]    │  注文株数 / 売り時のみ「全数量」ボタン
│         (保有: 200株)            │  売り時のみ保有株数を表示
│  価格: [成行 ▼] [▼][________][▲]│  成行/指値 + 呼値単位ステップボタン
│  期日: [当日 ▼]                  │  注文期日
│─────────────────────────────────│
│  受渡金額: ¥XXX,XXX (概算)       │  確認情報
│  手数料:   ¥YYY                  │
│─────────────────────────────────│
│  発注パスワード: [__________]    │  第二パスワード
│─────────────────────────────────│
│       [  注文確認  ]             │  確認ボタン
└─────────────────────────────────┘
```

> **BBO（最良気配）**: 立花証券の株価気配は既存の crypto 向け depth ストリームとは
> 別系統のため、初期実装では **BBO 表示を省く**。銘柄名・コードのみ表示する。

### 注文確認モーダル

```
┌─────────────────────────────────┐
│  注文確認                        │
│─────────────────────────────────│
│  銘柄:    7203 トヨタ自動車      │
│  売買:    買い                   │
│  数量:    100株                  │
│  価格:    成行                   │
│  口座:    特定                   │
│─────────────────────────────────│
│  [キャンセル]   [注文を発注する]  │
└─────────────────────────────────┘
```

### 状態設計

```rust
pub struct OrderEntryPanel {
    issue_code: String,
    issue_name: String,
    side: Side,                         // Buy / Sell
    account_type: AccountType,          // 特定 / 一般 / NISA / N成長
    qty: String,
    price_type: PriceType,              // Market / Limit
    limit_price: String,
    tick_size: Option<f64>,             // 呼値単位（銘柄連動で更新）
    cash_margin: CashMarginType,        // 現物 / 信用新規 / 信用返済
    expire_day: ExpireDay,              // Today / Specified(date)
    holdings: Option<u64>,             // 保有株数（売り時のみ表示 / 全数量ボタン用）
    second_password: String,
    confirm_modal: bool,
    loading: bool,
    last_result: Option<OrderResult>,
    /// REPLAY モードで仮想注文モードになったとき true
    /// true の場合: パスワード入力欄を非表示、確認ボタンラベル変更、パスワードバリデーションをスキップ
    pub is_virtual: bool,
}

pub enum Side { Buy, Sell }
pub enum PriceType { Market, Limit }
pub enum AccountType { Tokutei, Ippan, Nisa, NisaGrowth }
pub enum CashMarginType { Cash, MarginNew6M, MarginClose6M, MarginNewGeneral, MarginCloseGeneral }
pub enum ExpireDay { Today, Specified(String) }

// Warning も受注番号が返る「成功の一種」なので Success に統合
pub struct OrderSuccess {
    pub order_num: String,
    pub warning: Option<String>,        // Some = 警告あり（それでも受付済み）
}
pub type OrderResult = Result<OrderSuccess, String>;

pub enum Message {
    SideChanged(Side),
    AccountTypeChanged(AccountType),
    QtyChanged(String),
    FillFromHoldings,                   // 「全数量」ボタン: holdings を qty にセット
    PriceTypeChanged(PriceType),
    LimitPriceChanged(String),
    PriceIncrementTick,                 // 「▲」ボタン: limit_price を呼値単位で+1
    PriceDecrementTick,                 // 「▼」ボタン: limit_price を呼値単位で-1
    CashMarginChanged(CashMarginType),
    ExpireDayChanged(ExpireDay),
    SecondPasswordChanged(String),
    HoldingsUpdated(Option<u64>),
    ConfirmClicked,
    ConfirmCancelled,
    Submitted,
    OrderCompleted(OrderResult),
    SyncIssue { issue_code: String, issue_name: String, tick_size: Option<f64> },
}

pub enum Action {
    Submit(Box<NewOrderRequest>),       // NewOrderRequest が 240 bytes のため Box 化
    FetchHoldings { issue_code: String },
}
```

### `view()` シグネチャ

```rust
// is_replay を受け取り、仮想モード UI の表示を制御する
pub fn view(&self, theme: &Theme, is_replay: bool) -> Element<Message>
```

`is_virtual_mode = self.is_virtual || is_replay` として評価する。
仮想モード時の UI 変更:

| 変更点 | 通常モード | 仮想モード |
|---|---|---|
| パスワード欄 | 表示 | 非表示 |
| バナー | なし | `"⏪ 仮想注文モード"` (is_virtual) / `"⏪ REPLAYモード中 — 注文は無効です"` (is_replay のみ) |
| 確認ボタンラベル | `"注文確認"` | `"仮想注文確認"` |
| パスワードバリデーション | 必須 | スキップ |
```

### 銘柄連動

チャートペインが銘柄変更時に `Effect::SyncIssueToOrderEntry` を発行する。
`dashboard.rs` がこれを受け取り、同一ウィンドウ内の `OrderEntry` ペインに
`pane::Event` として配信する。LinkGroup とは独立した単方向の連動。

### 注文結果のデータフロー

```
OrderEntryPanel.update(Submitted)
  → Action::Submit(req)
  → pane.rs: Effect::SubmitNewOrder(req)
  → dashboard.rs: Task::perform(connector::order::submit_new_order(...))
  → dashboard::Message::OrderApiResult(pane_id, result)
  → pane.rs: state.update(Event::OrderApiResult(result))
  → OrderEntryPanel.update(OrderCompleted(result))
```

> **注文成功後の連鎖更新（未実装）**: `OrderCompleted(Ok(_))` 後に
> `FetchBuyingPower` / `FetchOrders` を自動トリガーする処理は未実装。

> **トースト通知（未実装）**: 注文受付後の "注文受付: 注文番号 XXXXXXXX" トーストは未実装。

---

## 注文照会パネル（`src/screen/dashboard/panel/order_list.rs`）

`Panel` trait（`canvas::Program`）は実装しない。

```
表示項目:
  - 銘柄コード / 売買 / 注文株数・約定株数 / 注文単価・約定単価 / 状態 / 注文日時

注文状態の色分け:
  - "全部約定" → 通常
  - "一部約定" → 強調（黄）
  - "取消完了" → グレー
  - エラー系   → 赤
  - "受付中"/"注文中" → 薄色

行の操作:
  - クリック → 約定明細を展開
  - [訂正] ボタン → 訂正モーダルを開く
  - [取消] ボタン → 取消モーダルを開く
    ※ is_cancelable() == true の行のみ表示

                  [更新]  ← 手動リフレッシュボタン
```

### 状態設計

```rust
pub struct OrderListPanel {
    orders: Vec<OrderRecord>,
    prev_orders: Vec<OrderRecord>,          // 約定通知の diff 用
    expanded_order: Option<String>,         // 展開中の注文番号
    executions: HashMap<String, Vec<ExecutionRecord>>,
    correct_modal: Option<CorrectModal>,    // 訂正モーダル状態
    cancel_modal: Option<CancelModal>,      // 取消モーダル状態
    loading: bool,
    last_error: Option<String>,
}

pub enum Message {
    RefreshClicked,
    RowClicked(String),
    CorrectClicked(OrderRecord),
    CorrectNewPriceChanged(String),
    CorrectNewQtyChanged(String),
    CorrectPasswordChanged(String),
    CorrectSubmitted,
    CorrectCancelled,
    CancelClicked(OrderRecord),
    CancelPasswordChanged(String),
    CancelSubmitted,
    CancelCancelled,
    OrdersUpdated(Vec<OrderRecord>),
    ExecutionsUpdated(String, Vec<ExecutionRecord>),
    ModifyCompleted(Result<ModifyOrderResponse, String>),
    PollTick,
}

pub enum Action {
    FetchOrders,
    FetchOrderDetail { order_num: String, eig_day: String },
    SubmitCorrect(Box<CorrectOrderRequest>),
    SubmitCancel(Box<CancelOrderRequest>),
}
```

### 約定通知

`newly_executed()` メソッドで `prev_orders` との diff を比較し、
状態テキストが "全部約定" に遷移した行を検出してトーストで通知する。

### 自動リフレッシュ戦略

| 方式 | 状態 |
|---|---|
| 手動リフレッシュ（[更新] ボタン） | ✅ 実装済み |
| イベント駆動（注文発注・訂正・取消成功後） | 未実装（連鎖 Effect 未接続） |
| 自動ポーリング（10秒間隔） | 未実装 |
| 取引時間帯チェック | 未実装 |

> **ポーリング設計方針（未実装）**: `iced::time::every(Duration::from_secs(10))` を
> `dashboard.rs` の `subscription()` に追加し、`Message::PollOrders` を発行する。
> `OrderListPanel` 側では `PollTick` メッセージを受け取り `Action::FetchOrders` を返す。
> 取引時間外はアクションを返さない設計。

---

## 余力情報パネル（`src/screen/dashboard/panel/buying_power.rs`）

`Panel` trait（`canvas::Program`）は実装しない。

```
現物口座:
  現物株買付可能額:    ¥X,XXX,XXX
  NISA成長投資残高:    ¥X,XXX,XXX

信用口座:
  信用新規建可能額:    ¥X,XXX,XXX
  委託保証金率:        XX.XX%
  追証: [警告なし / ⚠ 追証確定（赤）]

                      [更新]  ← 手動リフレッシュボタン
```

```rust
pub enum Message {
    RefreshClicked,
    BuyingPowerUpdated { cash: BuyingPowerResponse, margin: MarginPowerResponse },
    FetchFailed(String),
}

pub enum Action {
    FetchBuyingPower,
}
```

> **余力の更新タイミング**:
> - パネルを開いた時（初回取得）: ✅ 実装済み
> - [更新] ボタン押下時（手動）: ✅ 実装済み
> - 注文発注成功後の自動トリガー: 未実装（連鎖 Effect 未接続）

---

## connector（`src/connector/order.rs`）

```rust
pub async fn submit_new_order(
    client: &reqwest::Client,
    session: &TachibanaSession,
    req: NewOrderRequest,
) -> Result<NewOrderResponse, TachibanaError>

pub async fn submit_correct_order(
    client: &reqwest::Client,
    session: &TachibanaSession,
    req: CorrectOrderRequest,
) -> Result<ModifyOrderResponse, TachibanaError>

pub async fn submit_cancel_order(
    client: &reqwest::Client,
    session: &TachibanaSession,
    req: CancelOrderRequest,
) -> Result<ModifyOrderResponse, TachibanaError>

// 現在の営業日（eig_day）を渡す。未取得の場合は eig_day_or_today() でローカル日付を使用
pub async fn fetch_orders(
    client: &reqwest::Client,
    session: &TachibanaSession,
    eig_day: &str,
) -> Result<Vec<OrderRecord>, TachibanaError>

pub async fn fetch_order_detail(
    client: &reqwest::Client,
    session: &TachibanaSession,
    order_num: &str,
    eig_day: &str,
) -> Result<Vec<ExecutionRecord>, TachibanaError>

// CLMZanKaiKanougaku と CLMZanShinkiKanoIjiritu を tokio::join! で並列取得
pub async fn fetch_buying_power(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(BuyingPowerResponse, MarginPowerResponse), String>

// sellable_qty（売付可能株数）を u64 に変換して返す
pub async fn fetch_holdings(
    client: &reqwest::Client,
    session: &TachibanaSession,
    issue_code: &str,
) -> Result<u64, TachibanaError>
```

---

## pane.rs（`src/screen/dashboard/pane.rs`）

### State struct（抜粋）

```rust
pub struct State {
    // 既存フィールド...
    /// REPLAY モードで仮想注文を有効にするフラグ
    /// dashboard.rs の sync_virtual_mode() から設定される
    pub is_virtual_mode: bool,
}
```

### Effect enum

```rust
pub enum Effect {
    // 既存...
    SubmitNewOrder(NewOrderRequest),
    SubmitCorrectOrder(CorrectOrderRequest),
    SubmitCancelOrder(CancelOrderRequest),
    FetchOrders,
    FetchOrderDetail(String, String),       // (order_num, eig_day)
    FetchBuyingPower,
    FetchHoldings { issue_code: String },
    SyncIssueToOrderEntry { issue_code: String, issue_name: String, tick_size: Option<f64> },
    /// REPLAY モード中の仮想注文送信（証券 API を呼ばない）
    SubmitVirtualOrder(crate::replay::virtual_exchange::VirtualOrder),
}
```

### Content enum

非 canvas パネル（注文パネル）は `Panel` trait を実装しない。
`pane.rs` の view/update match に専用アームを追加する。

```rust
pub enum Content {
    // 既存...
    OrderEntry(panel::order_entry::OrderEntryPanel),
    OrderList(panel::order_list::OrderListPanel),
    BuyingPower(panel::buying_power::BuyingPowerPanel),
}
```

### `Content::placeholder`

`TickerInfo` 不要なパネルを初期化する際に使用する公開ファクトリ。
`set_content_and_streams()` は `tickers[0]` に無条件アクセスするため、注文パネルでは使用不可。

```rust
// pub に変更済み（サイドバー注文ボタンの split_focused_and_init_order から呼び出すため）
pub fn placeholder(kind: ContentKind) -> Self { ... }
```

### `virtual_order_from_new_order_request`（プライベート）

`NewOrderRequest` を `VirtualOrder` に変換するヘルパー。`side` フィールドのマッピングは `match` で排他的に処理し、未知コードは `log::warn!` + `None` で破棄する。

```rust
// tachibana API: "3" = 買い, "1" = 売り
let side = match req.side.as_str() {
    "3" => PositionSide::Long,
    "1" => PositionSide::Short,
    unknown => {
        log::warn!("仮想注文: 未知の side コード ({unknown:?}) — 注文を破棄");
        return None;
    }
};
```

### panel::Message

注文関連 variant は `String` フィールドを含むため `Copy` 不可。
`panel::Message` は `#[derive(Debug, Clone)]`（`Copy` を削除済み）。

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // 既存...
    OrderEntry(order_entry::Message),
    OrderList(order_list::Message),
    BuyingPower(buying_power::Message),
}
```

---

## 仮想約定エンジン（`src/replay/virtual_exchange/`）

REPLAY モード中に証券 API を呼ばずに仮想注文を処理するエンジン。
`main.rs` が `Option<Arc<tokio::sync::Mutex<VirtualExchangeEngine>>>` として保持し、
HTTP API スレッドとも共有する。

### モジュール構成

```
src/replay/virtual_exchange/
├── mod.rs          VirtualExchangeEngine（公開 API ファサード）
├── order_book.rs   VirtualOrderBook（注文受付・約定判定）
└── portfolio.rs    VirtualPortfolio（ポジション管理・PnL 計算）
```

### 型定義

```rust
// --- portfolio.rs ---

pub enum PositionSide { Long, Short }

pub struct Position {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub entry_price: f64,
    pub entry_time_ms: u64,
    pub exit_price: Option<f64>,
    pub exit_time_ms: Option<u64>,  // Phase 2 PnL 履歴表示で使用予定
    pub realized_pnl: Option<f64>,
}

pub struct VirtualPortfolio {
    pub initial_cash: f64,
    pub cash: f64,
    positions: Vec<Position>,
}

#[derive(serde::Serialize)]
pub struct PortfolioSnapshot {
    pub cash: f64,
    pub unrealized_pnl: f64,
    pub realized_pnl: f64,
    pub total_equity: f64,          // cash + unrealized_pnl
    pub open_positions: Vec<PositionSnapshot>,
    pub closed_positions: Vec<PositionSnapshot>,
}

// --- order_book.rs ---

#[derive(serde::Serialize, serde::Deserialize)]
pub struct VirtualOrder {
    pub order_id: String,           // UUID
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub order_type: VirtualOrderType,
    pub placed_time_ms: u64,        // StepClock::now_ms() で記録
    pub status: VirtualOrderStatus,
}

pub enum VirtualOrderType {
    Market,
    Limit { price: f64 },
}

pub enum VirtualOrderStatus {
    Pending,
    Filled { fill_price: f64, fill_time_ms: u64 },
    Cancelled,
}

pub struct FillEvent {
    pub order_id: String,
    pub ticker: String,
    pub side: PositionSide,
    pub qty: f64,
    pub fill_price: f64,
    pub fill_time_ms: u64,
}
```

### 約定ルール

| 注文種別 | 約定条件 |
|---|---|
| 成行 | `on_tick()` 内で `trades[0].price` で即時約定 |
| 指値買い | `trade.price <= limit_price` のトレードが来た tick で約定 |
| 指値売り | `trade.price >= limit_price` のトレードが来た tick で約定 |

`place()` は `Pending` 状態で登録するのみ。約定は必ず次の `on_tick()` で行う。

### REPLAY モード Safety Guard

REPLAY 中の誤発注を防ぐための二重ガード:

**1. Dashboard レベルのブロック（`src/screen/dashboard.rs`）**

```rust
// Dashboard struct に is_replay フラグを追加
pub is_replay: bool,

// Effect ハンドラ内でブロック
pane::Effect::SubmitNewOrder(req) => {
    if is_replay {
        log::warn!("REPLAY中の発注はブロックされました（新規注文）: {:?}", req);
        Task::none()
    } else {
        // 既存の証券 API 呼び出し
    }
}
// SubmitCorrectOrder / SubmitCancelOrder も同様にブロック
```

**2. pane.rs レベルのルーティング**

```rust
// is_virtual_mode == true の場合、証券 API Effect の代わりに仮想注文 Effect を発行
panel::order_entry::Action::Submit(req) => {
    if is_virtual {
        Some(Effect::SubmitVirtualOrder(virtual_order_from_new_order_request(&req)))
    } else {
        Some(Effect::SubmitNewOrder(*req))
    }
}
```

### dashboard.rs の拡張

```rust
// 追加フィールド
pub struct Dashboard {
    // 既存...
    pub is_replay: bool,
}

// 追加 Event / Message
pub enum Event {
    // 既存...
    SubmitVirtualOrder(crate::replay::virtual_exchange::VirtualOrder),
}

pub enum Message {
    // 既存...
    VirtualOrderFilled(crate::replay::virtual_exchange::FillEvent),
}

// pane 間の仮想モード同期
fn sync_virtual_mode(&mut self) {
    // 全ペインの is_virtual_mode と OrderEntryPanel の is_virtual を同期する
}

// フォーカスなし・単一ペイン時の自動フォーカス（内部ヘルパー）
fn auto_focus_single_pane(&mut self, main_window: window::Id) { ... }

// 注文パネル専用 Split（TickerInfo 不要）
pub fn split_focused_and_init_order(
    &mut self,
    main_window: window::Id,
    content_kind: data::layout::pane::ContentKind,
) -> Task<Message> { ... }
```

`VirtualOrderFilled` ハンドラはトースト通知を表示する:
```
"[仮想] 約定: {ticker} {side} {qty:.4} @ {price:.2}"
```

### main.rs の拡張

```rust
pub struct Flowsurface {
    // 既存...
    virtual_engine: Option<Arc<tokio::sync::Mutex<VirtualExchangeEngine>>>,
}
```

**ライフサイクル:**
- Live → Replay 遷移: `VirtualExchangeEngine::new(1_000_000.0)` で初期化（既存なら `reset()`）
- Replay → Live 遷移: `engine.reset()`

**Tick 処理（毎 tick）:**
```rust
// replay dispatcher から Trade が来たとき
if let Some(engine) = &self.virtual_engine {
    let fills = tokio::task::block_in_place(|| {
        engine.blocking_lock().on_tick(&ticker, &trades, clock_ms)
    });
    // fills を VirtualOrderFilled メッセージとして発行
}
```

### HTTP API（ポート 9876）

| メソッド | パス | 説明 |
|---|---|---|
| `POST` | `/api/replay/order` | 仮想注文を発注する |
| `GET` | `/api/replay/portfolio` | ポートフォリオスナップショット取得 |
| `GET` | `/api/replay/state` | エンジン状態確認（REPLAY 中かどうか等） |

**POST /api/replay/order リクエスト JSON:**

```json
{
  "ticker": "BTCUSDT",
  "side": "Long",
  "qty": 0.1,
  "order_type": "market"
}
// 指値の場合:
{
  "ticker": "BTCUSDT",
  "side": "Short",
  "qty": 0.5,
  "order_type": { "limit": 92000.0 }
}
```

---

## スタイル（`src/style.rs`）

```rust
// 注文状態色（sOrderStatus テキストで分岐）
pub fn order_status_color(status_text: &str, theme: &Theme) -> Color

// 売買区分色（買=青系, 売=赤系）
pub fn side_color(side_str: &str, theme: &Theme) -> Color

// 追証警告色
pub fn margin_call_color(theme: &Theme) -> Color
```

> **現状**: これらの関数は実装済みだが、パネルの `view()` からまだ呼び出されていないため
> `dead_code` 警告が出る。各パネルの view で使用する際に解消予定。

---

## セキュリティ上の注意事項

- **第二パスワードはメモリ上にのみ保持**し、ログ・設定ファイルへの書き込みを禁止
- 注文送信後は第二パスワードフィールドをクリア
- `second_password` フィールドを含む構造体に `#[derive(Debug)]` を付けない
  （手動実装で `[REDACTED]` を返す）
- 注文確認ステップ（2段階）は必須とし、バイパス不可

---

## 実装状況

### 完了済み

| 機能 | ファイル |
|---|---|
| 注文・照会・余力・保有株数 API 型定義 | `exchange/src/adapter/tachibana.rs` |
| `serialize_order_request()` ヘルパー | `exchange/src/adapter/tachibana.rs` |
| `OrderRecord::is_cancelable()` | `exchange/src/adapter/tachibana.rs` |
| `ContentKind` / `Pane` enum 拡張 | `data/src/layout/pane.rs` |
| 注文入力パネル（UI・ロジック） | `src/screen/dashboard/panel/order_entry.rs` |
| 注文照会パネル（UI・ロジック） | `src/screen/dashboard/panel/order_list.rs` |
| 訂正・取消モーダル | `src/screen/dashboard/panel/order_list.rs` |
| 余力情報パネル（UI・ロジック） | `src/screen/dashboard/panel/buying_power.rs` |
| `Content` / `Effect` enum 拡張 | `src/screen/dashboard/pane.rs` |
| `panel::Message` の `Copy` → `Clone` 変更 | `src/screen/dashboard/panel.rs` |
| connector API 関数 | `src/connector/order.rs` |
| `dashboard.rs` の Effect ハンドラ接続 | `src/screen/dashboard.rs` |
| `eig_day_or_today()` フォールバック | `src/screen/dashboard.rs` |
| スタイル関数 | `src/style.rs` |
| **REPLAY 中の誤発注防止ガード** | `src/screen/dashboard.rs` |
| **`OrderEntryPanel.is_virtual` フィールド・仮想モード UI** | `src/screen/dashboard/panel/order_entry.rs` |
| **`Pane.is_virtual_mode` フィールド・仮想注文ルーティング** | `src/screen/dashboard/pane.rs` |
| **`VirtualPortfolio`（ポジション管理・PnL 計算）** | `src/replay/virtual_exchange/portfolio.rs` |
| **`VirtualOrderBook`（注文受付・約定判定）** | `src/replay/virtual_exchange/order_book.rs` |
| **`VirtualExchangeEngine`（ファサード）** | `src/replay/virtual_exchange/mod.rs` |
| **仮想注文エンジンの main.rs 統合** | `src/main.rs` |
| **HTTP API（POST /api/replay/order 等）** | `src/replay_api.rs` |
| **仮想約定トースト通知** | `src/screen/dashboard.rs` |
| **サイドバー注文ボタン・インラインパネル** | `src/screen/dashboard/sidebar.rs` |
| **注文パネル専用 Split（`split_focused_and_init_order`）** | `src/screen/dashboard.rs` |
| **`auto_focus_single_pane` 共通ヘルパー化** | `src/screen/dashboard.rs` |
| **`ContentKind::ALL` から注文系除外** | `data/src/layout/pane.rs` |

### 未実装・残課題

| 課題 | 優先度 | 備考 |
|---|---|---|
| `sOrderEigyouDay` フィールド名の確認 | **高** | 実注文照会ログで確認。誤ると訂正・取消が全失敗 |
| 注文成功後の `FetchBuyingPower` / `FetchOrders` 連鎖 | 中 | 注文入力パネルの `OrderCompleted(Ok(_))` で発行 |
| 注文受付トースト通知（証券 API） | 中 | 既存の Toast / Notification 機構を使用予定 |
| 自動ポーリング（10秒間隔） | 中 | `dashboard.rs` の `subscription()` に追加 |
| 取引時間帯チェック | 低 | ポーリング実装時に同時対応 |
| スタイル関数のパネル view への接続 | 低 | `dead_code` 警告解消のため |
| 逆指値 UI | 低 | 現在は通常注文固定。立花証券イベント API 接続後に検討 |
| BBO（最良気配）表示 | 低 | 立花証券イベント API 接続後に追加 |
| **仮想注文の UI 一覧表示（Phase 2）** | 中 | `VirtualOrderBook::orders()` を利用。仮想注文ペインの実装 |
| **仮想ポジションのクローズ（Phase 2）** | 中 | `VirtualPortfolio::record_close()` は実装済み・未接続 |
| **SeekBackward 時のエンジンリセット** | 中 | 現在は Live↔Replay 遷移時のみリセット |
| **PnL 履歴グラフ（Phase 2）** | 低 | `exit_time_ms` フィールドは実装済み・未使用 |

---

## ファイル変更サマリー

| ファイル | 変更種別 |
|---|---|
| `exchange/src/adapter/tachibana.rs` | 注文・照会・余力・保有株数 API 型追加 |
| `data/src/layout/pane.rs` | `ContentKind` / `Pane` enum に注文系 3 variant 追加・`ALL` を 11→8 に縮小（注文系除外）|
| `src/screen/dashboard/panel/order_entry.rs` | **新規**（±ティックボタン / 保有株表示 / 全数量ボタン / 仮想モード UI） |
| `src/screen/dashboard/panel/order_list.rs` | **新規**（訂正・取消モーダル / 約定通知 / Panel trait 不使用） |
| `src/screen/dashboard/panel/buying_power.rs` | **新規**（iced widget、Panel trait 不使用） |
| `src/screen/dashboard/panel.rs` | `pub mod` 宣言追加・`Message` enum 拡張（`Copy` → `Clone`） |
| `src/screen/dashboard/pane.rs` | `Content` / `Effect` enum 拡張・`is_virtual_mode` フィールド追加・仮想注文ルーティング |
| `src/screen/dashboard.rs` | Effect ハンドラ接続・`is_replay` フィールド・`VirtualOrderFilled` / `SubmitVirtualOrder` 追加 |
| `src/connector.rs` | `pub mod order;` 追記 |
| `src/connector/order.rs` | **新規** 注文・照会・余力・保有株数取得関数 |
| `src/style.rs` | 注文状態・売買・追証警告色追加 |
| `src/replay/mod.rs` | `pub mod virtual_exchange;` 追加 |
| `src/replay/virtual_exchange/mod.rs` | **新規** `VirtualExchangeEngine` ファサード |
| `src/replay/virtual_exchange/portfolio.rs` | **新規** `VirtualPortfolio`・`PortfolioSnapshot`（ユニットテスト 4 件） |
| `src/replay/virtual_exchange/order_book.rs` | **新規** `VirtualOrderBook`・`FillEvent`（ユニットテスト 7 件） |
| `src/replay/controller.rs` | `current_time_ms()` メソッド追加 |
| `src/replay_api.rs` | `ApiCommand::VirtualExchange` と 3 エンドポイント追加 |
| `src/main.rs` | `virtual_engine` フィールド・Tick ごとの on_tick フック・仮想注文ハンドラ・`OpenOrderPane` ハンドラ・`Menu::Order` アーム |
| `data/src/config/sidebar.rs` | `Menu::Order` バリアント追加 |
| `src/screen/dashboard/sidebar.rs` | `Message::OrderPaneSelected` / `Action::OpenOrderPane` 追加・注文ボタン・インラインパネル・相互排他ロジック |

---

## 実装時の知見

- `second_password` を持つ構造体は `#[derive(Debug)]` 不可。手動実装で `[REDACTED]` を返す。
- `Effect` が `Debug + Clone` を要求するため全リクエスト / レスポンス struct に `Clone` が必要。
- `NewOrderRequest` は 240 bytes のため `Action::Submit(Box<NewOrderRequest>)` にする（clippy 警告）。
- `serialize_order_request()` は `serde_json::to_value()` でマップに共通フィールドをマージする方式。逆指値フィールドもここで固定値として付与する。
- `row![].extend(...)` は iced の `Row` が `IntoIterator` を実装しないためコンパイルエラー。各ブランチにラベルを直接含めるか `Vec<Element>` を作って extend する。
- `iced::Padding` は `[u16; 4]` に対応していない。`iced::padding::left(N)` などを使う。
- `panel.rs` の `Content` match は view / invalidate / update_interval / last_tick / reorder_indicators / studies / kind / initialized など多箇所に存在する。
- `layout.rs` の `From<&pane::State>` と `configuration()` 関数も新 variant を処理する必要がある。
- `fetch_buying_power` は現物余力と信用余力の 2 API を `tokio::join!` で並列取得してタプルで返す。
- `iter_all_panes_mut()` は `(window::Id, pane_grid::Pane, &mut pane::State)` タプルを返す。
- `連鎖 if let ... && let ...` パターン（Rust 1.64+）でネストした `if let` をフラットにできる。
- **REPLAY ガードのボローチェッカー対策**: `dashboard.update()` 内で `self.is_replay` を `let is_replay = self.is_replay;` としてコピーしてから `get_mut_pane()` で `self` を可変借用する。フラグを先にコピーしないと E0503（不変参照と可変参照の競合）が発生する。
- **`Trade` struct に ticker フィールドはない**: `VirtualOrderBook::on_tick()` は ticker を引数で受け取る設計にした。呼び出し側（main.rs）が `StreamKind` から ticker 文字列を特定して渡す。
- **`tokio::task::block_in_place()`**: iced の同期 `update()` コンテキストから `Arc<tokio::sync::Mutex<T>>` を同期ロックするために使用。`block_in_place` は tokio の multi-thread runtime でのみ動作する。
- **仮想注文の `placed_time_ms`**: pane.rs の `virtual_order_from_new_order_request()` では `0` を入れている。正確な時刻は `dashboard.rs` → `main.rs` の `SubmitVirtualOrder` ハンドラで `replay.current_time_ms()` を取得して上書きする設計にする（Phase 2）。
- **`#[allow(dead_code)]` の活用**: Phase 2 で使用予定のメソッド（`record_close()`、`orders()`）と フィールド（`exit_time_ms`）には `#[allow(dead_code)]` を付けてコメントを残す。将来削除ではなく「設計上意図的」であることを示す。
- **注文パネルを `ALL` から除外すると `set_content_and_streams` の bypass が必要**: `set_content_and_streams` は `tickers[0]` に無条件アクセスするため、TickerInfo 不要な注文パネルには使えない。`Content::placeholder()` を `pub` にして `split_focused_and_init_order` から呼び出す設計にした。
- **`Menu::Order` の modal 処理**: `main.rs` の `view_with_menu()` は `match menu` で全バリアントを網羅する。`Order` は modal を出さずサイドバーインラインで表示するため、アームは `base` をそのまま返す。
- **`split_focused_and_init_order` は `Option<Task>` でなく `Task` を返す**: フォーカスなし複数ペイン時も Toast 警告を含む `Task` を返すため、呼び出し側で `None` 分岐が不要。`split_focused_and_init`（`Option<Task>` 返し）との設計差異に注意。
- **サイレント失敗を避ける**: `panes.split()` 失敗は頻度が低いが「ボタンを押したのに何も起きない」UX になる。`Task::none()` ではなく `Toast::warn("Could not split pane")` を返すことでデバッグ可能にする。
