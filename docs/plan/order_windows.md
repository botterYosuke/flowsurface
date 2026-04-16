# 注文機能追加計画

## 概要

flowsurface に立花証券 e 支店 API を使った注文機能を追加する。
**注文発注がメイン**。照会・余力確認はそれを支える補助機能。

| ウィンドウ / 機能 | 役割 | 使用 API |
|---|---|---|
| **注文入力パネル** ★メイン | 買い・売り注文の入力と発注 | CLMKabuNewOrder |
| **注文訂正・取消** | 発注済み注文の変更・キャンセル | CLMKabuCorrectOrder / CLMKabuCancelOrder |
| **注文約定照会パネル** | 注文一覧と約定状況の確認 | CLMOrderList / CLMOrderListDetail |
| **余力情報パネル** | 買付可能額・保証金率の確認 | CLMZanKaiKanougaku / CLMZanShinkiKanoIjiritu |
| **保有株数取得** | 売り注文時の全数量ボタン | CLMGenbutuKabuList |

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
| `sGyakusasiOrderType` | 逆指値種別 | 0=通常, 1=逆指値, 2=通常+逆指値 |
| `sGyakusasiZyouken` | 逆指値条件（トリガー価格） | "0"=指定なし, 数値 |
| `sGyakusasiPrice` | 逆指値値段 | "*"=指定なし, "0"=成行, 数値=指値 |
| `sSecondPassword` | **第二パスワード（必須）** | 発注パスワード |

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

## 実装フェーズ

フェーズの実行順序:

```
フェーズ 1（API 型）→ フェーズ 2（data 拡張）→ フェーズ 7-骨格（型コンパイル確認）
  → フェーズ 3（注文入力）→ フェーズ 4（訂正・取消）→ フェーズ 5（照会）
  → フェーズ 6（余力）→ フェーズ 7-完成（connector 接続）→ フェーズ 8（スタイル）
```

> フェーズ 7 の骨格（Content enum / Effect の variant 追加のみ）を先に入れてコンパイルを通しておく。
> 各パネルの実装が進むにつれて徐々に肉付けする。

---

## フェーズ 1: API 型定義（`exchange` クレート）

**ファイル**: `exchange/src/adapter/tachibana.rs`

### 設計方針

- `p_no`（リクエスト通番）・`p_sd_date`（接続日付）・`sCLMID`・`sJsonOfmt` は
  **既存パターン通り各構造体フィールドに含め、`new()` で自動生成する**。
  `build_api_url_from(&req)` は構造体をそのまま JSON シリアライズするため、
  フィールドに含まれていないと API が受け付けない（`LoginRequest`, `MarketPriceRequest`,
  `DailyHistoryRequest` すべて同パターン）。
  > `build_request_body()` というヘルパーは存在しない。
  > 全リクエストは `build_api_url_from()` → URL クエリに JSON をそのまま付加する方式。

- 逆指値フィールド（`sGyakusasiOrderType` 等）は初期実装では常に
  通常注文固定値（`"0"` / `"*"`）を `new()` 内で固定付与する。
  逆指値 UI は後続フェーズで追加する。

- **レスポンスは既存の `ApiResponse<T>` ラッパーに統一する。**
  全レスポンスは `ApiResponse<T>` でラップし、`check()` でエラー判定する。

  ```rust
  // 既存パターン（再掲）
  let resp: ApiResponse<NewOrderResponse> = serde_json::from_str(&body)?;
  let data = resp.check()?;  // sResultCode != "0" → TachibanaError
  ```

- `second_password` を含む構造体には **`#[derive(Debug)]` を付けない**
  （`Debug` を手動実装するか `secrecy` クレートを使う）。

### 1-1. 注文発注 API 型

```rust
// 新規注文リクエスト
// NOTE: second_password を含むため #[derive(Debug)] を付けない → Debug を手動実装
#[derive(Serialize)]
pub struct NewOrderRequest {
    // 共通フィールド（new() で自動生成）
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMKabuNewOrder"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    // 業務フィールド
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
    // 逆指値（初期実装は通常注文固定。new() 内で設定）
    #[serde(rename = "sGyakusasiOrderType")]
    pub gyakusasi_order_type: &'static str,  // "0"=通常
    #[serde(rename = "sGyakusasiZyouken")]
    pub gyakusasi_zyouken: &'static str,     // "0"
    #[serde(rename = "sGyakusasiPrice")]
    pub gyakusasi_price: &'static str,       // "*"
    #[serde(rename = "sSecondPassword")]
    pub second_password: String,
}

impl NewOrderRequest {
    pub fn new(
        account_type: String, issue_code: String, market_code: String,
        side: String, condition: String, price: String, qty: String,
        cash_margin: String, expire_day: String, second_password: String,
    ) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMKabuNewOrder",
            json_ofmt: "5",
            account_type, issue_code, market_code, side, condition,
            price, qty, cash_margin, expire_day,
            gyakusasi_order_type: "0",
            gyakusasi_zyouken: "0",
            gyakusasi_price: "*",
            second_password,
        }
    }
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
#[derive(Debug, Deserialize)]
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

// 訂正注文リクエスト
// NOTE: second_password を含むため #[derive(Debug)] を付けない → Debug を手動実装
#[derive(Serialize)]
pub struct CorrectOrderRequest {
    // 共通フィールド（new() で自動生成）
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMKabuCorrectOrder"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    // 業務フィールド
    #[serde(rename = "sOrderNumber")]
    pub order_number: String,
    #[serde(rename = "sEigyouDay")]
    pub eig_day: String,
    #[serde(rename = "sCondition")]
    pub condition: String,              // "*"=変更なし
    #[serde(rename = "sOrderPrice")]
    pub price: String,                  // "*"=変更なし, "0"=成行変更
    #[serde(rename = "sOrderSuryou")]
    pub qty: String,                    // "*"=変更なし（増株不可）
    #[serde(rename = "sOrderExpireDay")]
    pub expire_day: String,             // "*"=変更なし
    #[serde(rename = "sSecondPassword")]
    pub second_password: String,
}

impl CorrectOrderRequest {
    pub fn new(
        order_number: String, eig_day: String,
        condition: String, price: String, qty: String,
        expire_day: String, second_password: String,
    ) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMKabuCorrectOrder",
            json_ofmt: "5",
            order_number, eig_day, condition, price, qty, expire_day, second_password,
        }
    }
}

impl std::fmt::Debug for CorrectOrderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CorrectOrderRequest")
            .field("order_number", &self.order_number)
            .field("price", &self.price)
            .field("qty", &self.qty)
            .field("second_password", &"[REDACTED]")
            .finish()
    }
}

// 取消注文リクエスト
// NOTE: second_password を含むため #[derive(Debug)] を付けない → Debug を手動実装
#[derive(Serialize)]
pub struct CancelOrderRequest {
    // 共通フィールド（new() で自動生成）
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMKabuCancelOrder"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    // 業務フィールド
    #[serde(rename = "sOrderNumber")]
    pub order_number: String,
    #[serde(rename = "sEigyouDay")]
    pub eig_day: String,
    #[serde(rename = "sSecondPassword")]
    pub second_password: String,
}

impl CancelOrderRequest {
    pub fn new(
        order_number: String, eig_day: String, second_password: String,
    ) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMKabuCancelOrder",
            json_ofmt: "5",
            order_number, eig_day, second_password,
        }
    }
}

impl std::fmt::Debug for CancelOrderRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelOrderRequest")
            .field("order_number", &self.order_number)
            .field("second_password", &"[REDACTED]")
            .finish()
    }
}

// 訂正・取消共通レスポンス（ApiResponse<ModifyOrderResponse> でラップして使用）
#[derive(Debug, Deserialize)]
pub struct ModifyOrderResponse {
    #[serde(rename = "sOrderNumber", default)]
    pub order_number: String,
    #[serde(rename = "sEigyouDay", default)]
    pub eig_day: String,
    #[serde(rename = "sOrderDate", default)]
    pub order_datetime: String,
}
```

### 1-2. 注文照会 API 型

```rust
// 注文一覧リクエスト
#[derive(Debug, Serialize)]
pub struct OrderListRequest {
    // 共通フィールド（new() で自動生成）
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMOrderList"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    // 業務フィールド
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,             // 空文字=全銘柄
    #[serde(rename = "sSikkouDay")]
    pub sikkou_day: String,             // YYYYMMDD（当日営業日）
    #[serde(rename = "sOrderSyoukaiStatus")]
    pub status_filter: String,          // ""=全件
}

impl OrderListRequest {
    pub fn new(issue_code: String, sikkou_day: String, status_filter: String) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMOrderList",
            json_ofmt: "5",
            issue_code, sikkou_day, status_filter,
        }
    }
}

// 注文一覧レスポンス（配列フィールドを含むラッパー）
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
    // NOTE: 注文番号と営業日はペアで訂正・取消に使用。
    // 実際のフィールド名は API リファレンス（mfds_json_api_ref_text.html）で
    // CLMOrderList の aOrderList 要素を確認して rename を確定すること。
    // 暫定: "sOrderEigyouDay" と仮定。フィールド名が違うとデシリアライズ時に
    // default("") になり、取消・訂正 API に空文字を渡してエラーになるので要確認。
    #[serde(rename = "sOrderEigyouDay", default)]
    pub eig_day: String,
}

// 約定明細リクエスト
#[derive(Debug, Serialize)]
pub struct OrderDetailRequest {
    // 共通フィールド（new() で自動生成）
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMOrderListDetail"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    // 業務フィールド
    #[serde(rename = "sOrderNumber")]
    pub order_num: String,
    #[serde(rename = "sEigyouDay")]
    pub eig_day: String,
}

impl OrderDetailRequest {
    pub fn new(order_num: String, eig_day: String) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMOrderListDetail",
            json_ofmt: "5",
            order_num, eig_day,
        }
    }
}

// 約定明細レスポンス
#[derive(Debug, Deserialize)]
pub struct OrderDetailResponse {
    #[serde(rename = "aYakuzyouSikkouList", default)]
    pub executions: Vec<ExecutionRecord>,
}

// 約定明細レコード (1件)
#[derive(Debug, Clone, Deserialize)]
pub struct ExecutionRecord {
    #[serde(rename = "sYakuzyouSuryou", default)]
    pub exec_qty: String,
    #[serde(rename = "sYakuzyouPrice", default)]
    pub exec_price: String,
    #[serde(rename = "sYakuzyouDate", default)]
    pub exec_datetime: String,
}
```

> **注文状態の判定**: `CLMOrderList` の `sOrderStatus` は状態名称テキスト（"受付中" 等）。
> 取消可能判定は `OrderRecord` に `is_cancelable() -> bool` メソッドを実装し、
> テキストで判定する（"受付中" / "注文中" / "一部約定" を取消可能とする）。
> ポーリングでの約定検出も同様にテキスト比較で実装する。

### 1-3. 余力情報 API 型

```rust
// 現物買付余力レスポンス（ApiResponse<BuyingPowerResponse> でラップして使用）
#[derive(Debug, Deserialize)]
pub struct BuyingPowerResponse {
    #[serde(rename = "sSummaryGenkabuKaituke", default)]
    pub cash_buying_power: String,
    #[serde(rename = "sSummaryNseityouTousiKanougaku", default)]
    pub nisa_growth_buying_power: String,
    #[serde(rename = "sHusokukinHasseiFlg", default)]
    pub shortage_flag: String,
}

// 信用新規可能委託保証金率レスポンス（ApiResponse<MarginPowerResponse> でラップして使用）
#[derive(Debug, Deserialize)]
pub struct MarginPowerResponse {
    #[serde(rename = "sSummarySinyouSinkidate", default)]
    pub margin_new_order_power: String,
    #[serde(rename = "sItakuhosyoukin", default)]
    pub maintenance_margin_rate: String,
    #[serde(rename = "sOisyouKakuteiFlg", default)]
    pub margin_call_flag: String,       // "0"=なし, "1"=確定
}
```

### 1-4. 保有株数 API 型

```rust
// 現物買付余力リクエスト（フィールドなし → 共通フィールドのみ）
#[derive(Debug, Serialize)]
pub struct BuyingPowerRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMZanKaiKanougaku"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
}

impl BuyingPowerRequest {
    pub fn new() -> Self {
        Self { p_no: next_p_no(), p_sd_date: current_p_sd_date(),
               clm_id: "CLMZanKaiKanougaku", json_ofmt: "5" }
    }
}

// 信用余力リクエスト
#[derive(Debug, Serialize)]
pub struct MarginPowerRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMZanShinkiKanoIjiritu"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
}

impl MarginPowerRequest {
    pub fn new() -> Self {
        Self { p_no: next_p_no(), p_sd_date: current_p_sd_date(),
               clm_id: "CLMZanShinkiKanoIjiritu", json_ofmt: "5" }
    }
}

// 現物保有リクエスト
#[derive(Debug, Serialize)]
pub struct GenbutuKabuRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMGenbutuKabuList"
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,        // "5"
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,             // 銘柄コード（空文字=全保有銘柄）
}

impl GenbutuKabuRequest {
    pub fn new(issue_code: String) -> Self {
        Self { p_no: next_p_no(), p_sd_date: current_p_sd_date(),
               clm_id: "CLMGenbutuKabuList", json_ofmt: "5", issue_code }
    }
}

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

**テスト**:
- 新規注文リクエストのシリアライズ（成行・指値の各パターン）
- 新規注文レスポンスのデシリアライズ（正常・エラー・警告付き）
- 訂正・取消リクエストのシリアライズ
- `OrderRecord::is_cancelable()` の単体テスト

---

## フェーズ 2: data クレート拡張

### 2-1. ContentKind 拡張

**ファイル**: `data/src/layout/pane.rs`

```rust
pub enum ContentKind {
    // 既存...
    OrderEntry,     // 注文入力パネル
    OrderList,      // 注文約定照会パネル
    BuyingPower,    // 余力情報パネル
}

// ALL 定数も忘れずに更新する
pub const ALL: [ContentKind; N+3] = [
    // 既存...
    ContentKind::OrderEntry,
    ContentKind::OrderList,
    ContentKind::BuyingPower,
];
```

### 2-2. Pane enum 拡張（レイアウト永続化）

**ファイル**: `data/src/layout/pane.rs`（`ContentKind` と同じファイル）

`Pane` enum はレイアウトの JSON 保存・復元に使われる。
注文パネルは設定（stream / indicators / settings）を持たないため **ユニット variant** として追加する。

```rust
pub enum Pane {
    // 既存...
    OrderEntry,
    OrderList,
    BuyingPower,
}
```

> **後方互換について**: ユニット variant の追加はデシリアライズを壊さない。
> 既存の保存済みレイアウト JSON には新 variant が存在しないため、
> 新コードで旧レイアウトを読み込んでも問題なし（新 variant が出現しないので）。
> `#[serde(other)]` は **struct variant には使えない** ため、既存フィールドには適用しない。
> 各フィールドの耐障害性は既存の `#[serde(deserialize_with = "ok_or_default", default)]` で対応済み。

**テスト**: `ContentKind` / `Pane` の serde ラウンドトリップ

---

## フェーズ 7-骨格: 型コンパイル確認（フェーズ 3〜6 の前に実施）

フェーズ 3〜6 の実装前に、型の骨格だけ追加してビルドを通す。
中身（`view` / `update`）は `todo!()` でよい。

### Content enum に variant を追加

**ファイル**: `src/screen/dashboard/pane.rs`

```rust
pub enum Content {
    // 既存...
    OrderEntry(OrderEntryPanel),
    OrderList(OrderListPanel),
    BuyingPower(BuyingPowerPanel),
}
```

### 非 canvas パネルの描画方針

既存の `Heatmap`, `Kline`, `TimeAndSales`, `Ladder` 等は `panel::view()` 経由で
`canvas::Program` として描画される。新パネルは `Panel` trait（`canvas::Program`）を
実装せず、`view() -> Element<Message>` を直接返す。

`pane.rs` の view/update match で専用アームを追加する:

```rust
// pane.rs の view 関数内（抜粋）
Content::OrderEntry(panel) => panel.view().map(|msg| {
    Message::PanelEvent(pane, panel::Message::OrderEntry(msg))
}),
Content::OrderList(panel) => panel.view().map(|msg| {
    Message::PanelEvent(pane, panel::Message::OrderList(msg))
}),
Content::BuyingPower(panel) => panel.view().map(|msg| {
    Message::PanelEvent(pane, panel::Message::BuyingPower(msg))
}),

// pane.rs の update 関数内（抜粋）
(Content::OrderEntry(panel), Event::PanelInteraction(panel::Message::OrderEntry(msg))) => {
    if let Some(action) = panel.update(msg) {
        match action {
            order_entry::Action::Submit(req) => Some(Effect::SubmitNewOrder(req)),
            order_entry::Action::FetchHoldings { issue_code } => {
                Some(Effect::FetchHoldings { issue_code })
            }
        }
    } else { None }
}
```

### Effect に variant を追加

```rust
pub enum Effect {
    // 既存...
    SubmitNewOrder(NewOrderRequest),
    SubmitCorrectOrder(CorrectOrderRequest),
    SubmitCancelOrder(CancelOrderRequest),
    FetchOrders,
    FetchOrderDetail(String, String),       // (order_num, eig_day)
    FetchBuyingPower,
    FetchHoldings { issue_code: String },   // 売り選択時に保有株数を取得（CLMGenbutuKabuList）
    // 銘柄連動: チャートペインが銘柄変更時に発行 → dashboard が OrderEntry ペインへ配信
    SyncIssueToOrderEntry { issue_code: String, issue_name: String, tick_size: Option<f64> },
}
```

### panel::Message に variant を追加

**ファイル**: `src/screen/dashboard/panel.rs`（既存の `Message` enum に追記）

> **⚠️ 注意**: 現在の `panel::Message` は `#[derive(Debug, Clone, Copy)]` されている。
> `OrderEntry(order_entry::Message)` 等の追加で `order_entry::Message` が `Copy` を
> 満たせないため（`String` フィールドを含む）、**`Copy` derive を削除する必要がある**。
> これは `PanelInteraction(super::panel::Message)` を使う全箇所に波及する。
> フェーズ 7-骨格の作業で `Copy` を削除し、影響箇所を `clone()` で対応してから
> 他のパネル実装に進むこと。

```rust
// #[derive(Debug, Clone)] ← Copy を外す（String を含む variant が追加されるため）
#[derive(Debug, Clone)]
pub enum Message {
    // 既存...
    OrderEntry(order_entry::Message),
    OrderList(order_list::Message),
    BuyingPower(buying_power::Message),
}
```

### dashboard.rs に Effect ハンドラの骨格を追加

**ファイル**: `src/screen/dashboard.rs`

`pane::Effect::SubmitNewOrder` 等を受け取ったときのハンドラを追加する。
フェーズ 7-完成までは `Task::none()` を返す骨格でよい。

---

## フェーズ 3: 注文入力パネル（★メイン実装）

**ファイル**: `src/screen/dashboard/panel/order_entry.rs`

> **注意**: このパネルはテキスト入力・ボタン・ドロップダウンなどの通常 iced widget で実装する。
> 既存の `Panel` trait（`canvas::Program` ベース）は実装しない。
> `view() -> Element<Message>` と `update(msg: Message) -> Option<Action>` を直接定義する。

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

> **BBO（最良気配）について**: 立花証券の株価気配は既存の crypto 向け depth ストリームとは
> 別系統のため、初期実装では **BBO 表示を省く**。銘柄名・コードのみ表示し、
> リアルタイム気配は将来フェーズで立花証券イベント API と接続する際に追加する。

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
    // 入力フォーム
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

    // 保有株数（売り注文時に表示 / 「全数量」ボタン用）
    holdings: Option<u64>,             // None = 未取得

    // BBO は初期実装では省略（将来フェーズで立花証券イベント API 接続時に追加）

    // 認証
    second_password: String,

    // UI 状態
    confirm_modal: bool,                // 確認モーダル表示中
    loading: bool,
    last_result: Option<OrderResult>,  // 直前の注文結果
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
    // BboUpdated は将来フェーズ（立花証券イベント API 接続後）に追加
    HoldingsUpdated(Option<u64>),       // 保有株数の取得結果
    ConfirmClicked,                     // 確認モーダルを開く
    ConfirmCancelled,                   // 確認モーダルを閉じる
    Submitted,                          // 実際に発注 Effect を発行
    OrderCompleted(OrderResult),        // API 応答を受け取り UI を更新
}

pub enum Action {
    Submit(NewOrderRequest),            // pane.rs が Effect::SubmitNewOrder に変換
    FetchHoldings { issue_code: String },   // 売り選択時に保有株数を取得
}
```

> **`FillFromHoldings`（全数量ボタン）**: `side == Sell` のときのみ表示。
> `holdings` が `None` の場合はボタンを無効化する。

> **`PriceIncrementTick` / `PriceDecrementTick`**: `price_type == Limit` のときのみ有効。
> `tick_size` が `None`（未取得）の場合は操作を無視する。
> `limit_price` が空文字のときは `"0"` を初期値にする（BBO は初期実装では持たない）。

> **逆指値は将来フェーズに先送り**。初期実装では `sGyakusasiOrderType = "0"`（通常注文）を
> `NewOrderRequest::new()` 内で固定付与する。

### 銘柄連動

チャートペインが銘柄変更時に `Effect::SyncIssueToOrderEntry` を発行する。
`dashboard.rs` がこれを受け取り、同一ウィンドウ内の `OrderEntry` ペインに
`pane::Event` として配信する。LinkGroup とは独立した単方向の連動。

連動時に `tick_size` と `holdings`（売り選択時）も更新する。

### 注文結果のデータフロー

```
OrderEntryPanel.update(Submitted)
  → Action::Submit(req)
  → pane.rs: Effect::SubmitNewOrder(req)
  → dashboard.rs: Task::perform(connector::order::submit_new_order(...))
  → dashboard::Message::OrderApiResult(pane_id, result)
  → pane.rs: state.update(Event::OrderApiResult(result))
  → OrderEntryPanel.update(OrderCompleted(result))

注文成功後の連鎖更新:
  OrderCompleted(Ok(_))
  → pane.rs: Effect::FetchBuyingPower    // 余力を自動更新
  → pane.rs: Effect::FetchOrders         // 注文照会を自動更新
  → トースト通知を発行（"注文受付: 注文番号 XXXXXXXX"）
```

> **約定通知**: 注文照会パネルのポーリングで状態テキストが "全部約定" に
> 変わった行を検出したとき、既存の `Toast` / `Notification` 機構でトースト通知を表示する。
> サウンドは将来フェーズとする。

---

## フェーズ 4: 注文訂正・取消 UI

注文照会パネルの各行に [訂正] [取消] ボタンを配置。

### 訂正モーダル

```
┌─────────────────────────────────┐
│  注文訂正: 注文番号 XXXXXXXX     │
│─────────────────────────────────│
│  銘柄: 7203 トヨタ自動車 / 買い  │
│  現在: 指値 2,500円 × 100株      │
│─────────────────────────────────│
│  変更後の値段: [________]        │  空欄=変更なし
│  変更後の株数: [________]        │  空欄=変更なし（増株不可）
│─────────────────────────────────│
│  発注パスワード: [__________]    │
│─────────────────────────────────│
│  [キャンセル]   [訂正を送信]     │
└─────────────────────────────────┘
```

### 取消確認モーダル

```
┌─────────────────────────────────┐
│  注文を取り消しますか？           │
│                                 │
│  注文番号: XXXXXXXX             │
│  銘柄:    7203 トヨタ / 買い     │
│  数量:    100株                  │
│─────────────────────────────────│
│  発注パスワード: [__________]    │
│─────────────────────────────────│
│  [戻る]   [取消を送信]           │
└─────────────────────────────────┘
```

---

## フェーズ 5: 注文照会パネル

**ファイル**: `src/screen/dashboard/panel/order_list.rs`

> **注意**: フェーズ 3 と同様に `Panel` trait（`canvas::Program`）は実装しない。

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
  - [訂正] ボタン → フェーズ 4 の訂正モーダルを開く
  - [取消] ボタン → フェーズ 4 の取消モーダルを開く
    ※ is_cancelable() == true の行のみ表示

                  [更新]  ← 手動リフレッシュボタン
```

### 自動リフレッシュ戦略

- **自動ポーリング**: 10秒間隔でバックグラウンドリフレッシュ（取引時間帯のみ）
- **イベント駆動**: 注文発注・訂正・取消の成功後に即時リフレッシュ
- **約定通知**: 前回取得との diff を比較し状態テキストが "全部約定" に
  変わった行を検出したらトーストで通知する

**ポーリング実装方針**: `iced::time::every(Duration::from_secs(10))` を
`dashboard.rs` の `subscription()` に追加し、`Message::PollOrders` を発行する。
`OrderListPanel` 側では `last_fetched_at` を管理し、取引時間外は Effect を発行しない。

```rust
// dashboard.rs subscription() 内の追加分（抜粋）
iced::time::every(Duration::from_secs(10))
    .map(|_| Message::PollOrders)
```

```rust
// OrderListPanel の状態
pub struct OrderListPanel {
    orders: Vec<OrderRecord>,
    prev_orders: Vec<OrderRecord>,          // 約定通知の diff 用
    expanded_order: Option<String>,         // 展開中の注文番号
    executions: HashMap<String, Vec<ExecutionRecord>>,
    last_fetched_at: Option<Instant>,
    polling_interval: Duration,             // デフォルト 10秒
}
```

---

## フェーズ 6: 余力情報パネル

**ファイル**: `src/screen/dashboard/panel/buying_power.rs`

> **注意**: フェーズ 3 と同様に `Panel` trait（`canvas::Program`）は実装しない。

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

> **余力の更新タイミング**:
> - パネルを開いた時（初回取得）
> - [更新] ボタン押下時（手動）
> - **注文発注成功後**（フェーズ 3 の注文結果フローから自動トリガー）
>
> 連続した自動ポーリングは将来フェーズで検討する。

---

## フェーズ 7-完成: connector 接続

### 注文 API 関数の追加

**ファイル**: `src/connector/order.rs`（**新規ファイル**）

> `src/connector.rs` はモジュール宣言のみのファイル。
> 実装は既存の `src/connector/auth.rs` / `fetcher.rs` と同様に
> `src/connector/order.rs` を新規作成して配置する。

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

pub async fn fetch_orders(
    client: &reqwest::Client,
    session: &TachibanaSession,
    eig_day: &str,  // 当日営業日。NewOrderResponse.eig_day から取得して dashboard で保持する
) -> Result<Vec<OrderRecord>, TachibanaError>

// NOTE: 営業日の取得方法
// 初回注文前は eig_day が不明のため、最初の submit_new_order のレスポンス
// （NewOrderResponse.eig_day）を dashboard.rs で保持する。
// 発注前に注文照会を開いた場合は当日日付（YYYYMMDD）をローカル時計から生成してよい
// （立花 API は営業日=当日日付で解釈するため）。
// dashboard.rs に Option<String> の eig_day フィールドを持たせ、
// 初回注文成功後にセットする。

pub async fn fetch_order_detail(
    client: &reqwest::Client,
    session: &TachibanaSession,
    order_num: &str,
    eig_day: &str,
) -> Result<Vec<ExecutionRecord>, TachibanaError>

pub async fn fetch_buying_power(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<BuyingPowerResponse, TachibanaError>

pub async fn fetch_margin_power(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<MarginPowerResponse, TachibanaError>

// 保有株数取得（CLMGenbutuKabuList）
// 売り注文時の「全数量」ボタン / 保有株表示用
// sellable_qty（売付可能株数）を u64 に変換して返す
pub async fn fetch_holdings(
    client: &reqwest::Client,
    session: &TachibanaSession,
    issue_code: &str,
) -> Result<u64, TachibanaError>
```

`src/connector.rs` に `pub mod order;` を追記する。

### ペイン選択 UI

サイドバーまたはペイン作成時に `OrderEntry` / `OrderList` / `BuyingPower` を ContentKind として選択できるようにする。

---

## フェーズ 8: スタイル

**ファイル**: `src/style.rs`

```rust
// 注文状態色（sOrderStatus テキストで分岐）
pub fn order_status_color(status_text: &str, theme: &Theme) -> Color
// 追証警告色
pub fn margin_call_color(theme: &Theme) -> Color
// 売買区分色（買=青, 売=赤）
pub fn side_color(side: &Side, theme: &Theme) -> Color
```

---

## セキュリティ上の注意事項

- **第二パスワードはメモリ上にのみ保持**し、ログ・設定ファイルへの書き込みを禁止
- 注文送信後は第二パスワードフィールドをクリア
- `second_password` フィールドを含む構造体に `#[derive(Debug)]` を付けない
  （`Debug` を手動実装するか `secrecy` クレートを検討）
- 注文確認ステップ（2段階）は必須とし、バイパス不可にする

---

## タスクチェックリスト

### フェーズ 1: API 型定義
- ✅ `NewOrderRequest` 構造体（serde rename 属性付き、`Debug` 手動実装 + `Clone`）
- ✅ `NewOrderResponse` 構造体（実 API フィールド名に準拠）
- ✅ `CorrectOrderRequest` 構造体（`Debug` 手動実装 + `Clone`）
- ✅ `CancelOrderRequest` 構造体（`Debug` 手動実装 + `Clone`）
- ✅ `ModifyOrderResponse` 構造体
- ✅ `OrderListRequest` 構造体（`sikkou_day` フィールド名を snake_case で）
- ✅ `OrderListResponse` / `OrderRecord` 構造体（`eig_day` → `#[serde(rename = "sOrderEigyouDay")]`）
- ✅ `OrderRecord::is_cancelable()` メソッド（状態テキストで取消可能判定）
- ✅ `OrderDetailRequest` / `OrderDetailResponse` / `ExecutionRecord` 構造体
- ✅ `BuyingPowerResponse` / `MarginPowerResponse` 構造体
- ✅ `GenbutuKabuRequest` / `GenbutuKabuResponse` / `HoldingRecord` 構造体
- ✅ `serialize_order_request()` ヘルパー（p_no / p_sd_date / sCLMID / sJsonOfmt を付与）
- ✅ 各種ユニットテスト（シリアライズ・デシリアライズ・共通フィールド付与）
- ✅ `OrderRecord::is_cancelable()` の単体テスト（取消可能3状態 / 不可4状態）

**知見**:
- `second_password` を持つ構造体は `#[derive(Debug)]` 不可。手動実装で `[REDACTED]` を返す。
- `Effect` が `Debug + Clone` を要求するため `Clone` は必須だった。
- `NewOrderRequest` は 240 bytes のため `Action::Submit(Box<NewOrderRequest>)` にする（clippy 警告）。
- `serialize_order_request()` は `serde_json::to_value()` でマップに共通フィールドをマージする方式。

### フェーズ 2: data クレート拡張
- ✅ `ContentKind` に `OrderEntry` / `OrderList` / `BuyingPower` 追加
- ✅ `ContentKind::ALL` を 11 要素に更新
- ✅ `Pane` enum にユニット variant として `OrderEntry` / `OrderList` / `BuyingPower` を追加
- ✅ `PaneSetup::new()` の match arms に新 variant 追加
- ✅ serde ラウンドトリップテスト

**知見**: `PaneSetup::new()` の 2 つの match（basis / tick_multiplier）と push_freq match にも
新 variant を追加する必要があった。

### フェーズ 7-骨格: 型コンパイル確認
- ✅ `panel/order_entry.rs` / `order_list.rs` / `buying_power.rs` を `todo!()` で作成
- ✅ `panel.rs` に `pub mod` 宣言追加
- ✅ `panel::Message` を `Copy` → `Clone` に変更し `OrderEntry` / `OrderList` / `BuyingPower` variant 追加
- ✅ `pane.rs` の `Content` enum に variant 追加（view/update のアームも追加）
- ✅ `pane.rs` の `Effect` enum に注文関連 variant 追加
- ✅ `layout.rs` の `From<&pane::State>` と `configuration()` に新 variant 追加
- ✅ `dashboard.rs` の Effect ハンドラに `Task::none()` 骨格追加
- ✅ `cargo check` でコンパイル通過確認
- ✅ `cargo clippy -- -D warnings` で警告なし確認
- ✅ `cargo test --workspace` で全テスト通過確認（346件）

**知見**:
- `pane.rs` の `Content` match は多箇所あった（view/invalidate/update_interval/last_tick/reorder_indicators/studies/kind/initialized/panel::Message 経由の view）
- `layout.rs` の `From<&pane::State>` と `configuration()` 関数も新 variant を処理する必要あり
- `panel::Message::OrderEntry` 等は `Copy` でないため `panel::Message` を `Clone` のみにした

### フェーズ 3: 注文入力パネル（★）
- ✅ `OrderEntryPanel` 状態設計（Panel trait は不使用、BBO フィールドは省略）
- ✅ フォーム view 実装（iced widget: text_input / button / pick_list）
- ✅ 価格ステップボタン（`[▲]` / `[▼]` で呼値単位 ±1）
- ✅ 保有株数表示（売り選択時）と「全数量」ボタン
- ✅ 注文確認モーダル
- ✅ `Message::Submitted` → `Action::Submit` → Effect 発行
- [ ] 注文成功後に `FetchBuyingPower` / `FetchOrders` を連鎖トリガー（Phase 7-完成で実装）
- [ ] 注文受付トースト通知（Phase 7-完成で実装）
- ✅ 注文成功・失敗・警告の表示
- ✅ 銘柄連動処理（`SyncIssueToOrderEntry` 受信・`tick_size` 更新）
- ✅ 売り選択時に `FetchHoldings` を発行

**知見**:
- `row![].extend(row.into_iter())` は iced の `Row` が `IntoIterator` を実装しないためコンパイルエラー。
  ラベルを各 `if/else` ブランチに直接含めるか、`Vec<Element>` を作って `extend` する。
  今回は各ブランチにラベルを含めることで解決した（`qty_row`/`price_row` を `Element<'_,Message>` に変換）。
- `panel::update()` の match も新 variant（OrderEntry/OrderList/BuyingPower）を追加しないとコンパイルエラー。
  これらは各パネル固有の update() で処理するため `_ => {}` でよい。

### フェーズ 4: 訂正・取消 UI
- ✅ 訂正モーダル view / update
- ✅ 取消確認モーダル view / update

**知見**:
- 訂正・取消モーダルは `order_list.rs` 内に実装（`CorrectModal` / `CancelModal` 内部構造体）
- 空欄フィールドは `"*"` にマップ（変更なし）するのを `update(CorrectSubmitted)` 内で実施
- モーダル構造体には `#[derive(Debug, Clone)]` を付与（`Message` が要求）

### フェーズ 5: 注文照会パネル
- ✅ `OrderListPanel` view / update（Panel trait は不使用）
- ✅ 訂正・取消ボタン（`is_cancelable()` == true の行のみ表示）
- ✅ 約定明細展開 view
- [ ] `dashboard.rs` の `subscription()` に `iced::time::every(10秒)` を追加（Phase 7-完成で実装）
- [ ] 取引時間帯チェックでポーリングの発火を制御（Phase 7-完成で実装）
- ✅ 約定通知（状態テキスト "全部約定" への遷移を検出: `newly_executed()` メソッドで diff 検出）
- ✅ 手動リフレッシュボタン

**知見**:
- `column(rows.iter())` のような `IntoIterator` ベースの `column()` は使える（`column![]` マクロと別物）
- `iced::Padding` は `[u16; 4]` に対応していない。`iced::padding::left(N)` などを使う
- `newly_executed()` は `prev_orders` との diff で "全部約定" 遷移を検出するメソッドとして実装
- ポーリングは `Message::PollTick` として受け取り `Action::FetchOrders` を返す設計

### フェーズ 6: 余力情報パネル
- ✅ `BuyingPowerPanel` view / update（Panel trait は不使用）
- ✅ 手動リフレッシュボタン
- [ ] 注文発注成功後の自動更新受け取り（`FetchBuyingPower` Effect の処理 — Phase 7-完成で実装）

**知見**:
- ヘルパー関数 `labeled_value<'a>(label: &'a str, ...)` のライフタイムに注意
  `label` の型に `'a` を明示しないとコンパイルエラーになる（`Element<'a, Message>` を返すため）

### フェーズ 7-完成: connector 接続
- ✅ `src/connector/order.rs` 新規作成
- ✅ 各 API 関数実装（`serialize_order_request()` + `post_request()` パターンで統一）
- ✅ `fetch_holdings` 関数（`CLMGenbutuKabuList` を使い `sellable_qty` を u64 で返す）
- ✅ `src/connector.rs` に `pub mod order;` 追記
- ✅ `dashboard.rs` の Effect ハンドラを実際の API 呼び出し Task に差し替え
- ✅ `dashboard.rs` で `NewOrderResponse.eig_day` を受け取り `self.eig_day` に保持
- ✅ `FetchHoldings` / `SyncIssueToOrderEntry` Effect ハンドラ実装
- ✅ 訂正・取消の `"*"` マッピング（`order_list.rs` の `update(CorrectSubmitted)` 内で実装）
- [ ] サイドバー UI（ContentKind 選択）— 既存のサイドバーで ContentKind::ALL から選べるため省略可

**知見**:
- exchange adapter の async 関数（`submit_new_order` 等）を追加する場合は、レスポンス型に `Clone` が必要（`dashboard::Message` が `#[derive(Clone)]` を要求するため）
- `serialize_order_request()` の引数は `&serde_json::json!({})` でフィールドなしリクエストも処理できる
- `iter_all_panes_mut()` は `(window::Id, pane_grid::Pane, &mut pane::State)` タプルを返す（`state` だけでなくタプル分解が必要）
- 連鎖 `if let ... && let ...` パターン（Rust 1.64+）でネストした `if let` をフラットにできる
- `fetch_buying_power` は現物余力と信用余力の 2 つの API を `tokio::join!` で並列取得し、タプルで返す設計

### フェーズ 8: スタイル
- ✅ 注文状態色（状態テキストで分岐）・売買色・追証警告色

**知見**:
- `order_status_color()` / `side_color()` / `margin_call_color()` を `src/style.rs` の末尾に追加
- これらは各パネルの view() から `theme: &Theme` を受け取って呼び出す設計
- 現時点では未使用（パネル view で使用すると `dead_code` 警告が消える）

---

## ログ検証結果（2026-04-16）

`log::debug!` を追加して `cargo run`（デバッグビルド）で起動確認。

### 確認できた動作

| 項目 | 結果 |
|---|---|
| セッション読み込み（keyring → 検証） | ✅ 正常（ログで確認） |
| `build_request()` フィールドマッピング | ✅ 単体テスト 21件全通過 |
| `serialize_order_request()` 共通フィールド付与 | ✅ 単体テスト確認済 |
| `FetchHoldings` trigger（売り切替時） | ✅ 単体テスト確認済 |
| `second_password` マスク（`Debug` 手動実装） | ✅ `[REDACTED]` 出力 |
| `ApiResponse::check()` エラー判定 | ✅ p_errno / sResultCode 両方チェック |
| `eig_day_or_today()` フォールバック | ✅ 実装確認（ローカル日付へ fallback） |

### 要注意項目（未解決）

#### `sOrderEigyouDay` フィールド名（暫定）

`OrderRecord.eig_day` のデシリアライズキーを `"sOrderEigyouDay"` と仮定している。
実際の `CLMOrderList` API レスポンスで別のフィールド名が使われている場合、
`eig_day` が `""` になり訂正・取消 API 呼び出し時にエラーが発生する。

**確認方法**: 実際の注文照会を行い、受け取った `OrderRecord` の `eig_day` が空文字か否かをログで確認。
空文字の場合は `tachibana.rs` の `#[serde(rename = "sOrderEigyouDay")]` を実際のフィールド名に修正する。

#### `dead_code` 警告（スタイル関数）

`style.rs` の `order_status_color` / `side_color` / `margin_call_color` が未使用で警告が出る。
計画通り将来フェーズでパネルの view() から呼び出す際に解消予定。

---

## ファイル変更サマリー

| ファイル | 変更種別 |
|---|---|
| `exchange/src/adapter/tachibana.rs` | 注文・照会・余力・保有株数 API 型追加 |
| `data/src/layout/pane.rs` | `ContentKind` 拡張・`ALL` 更新・`Pane` enum にユニット variant 追加 |
| `src/screen/dashboard/panel/order_entry.rs` | **新規** ★メイン（±ティックボタン / 保有株表示 / 全数量ボタン）BBO は将来フェーズ |
| `src/screen/dashboard/panel/order_list.rs` | **新規**（ポーリング / 約定通知 / Panel trait 不使用） |
| `src/screen/dashboard/panel/buying_power.rs` | **新規**（iced widget、Panel trait 不使用） |
| `src/screen/dashboard/panel.rs`（または `panel/mod.rs`） | `pub mod` 宣言追加・`Message` enum 拡張 |
| `src/screen/dashboard/pane.rs` | `Content` enum / `Effect` enum 拡張・非 canvas パネルの view/update アーム追加 |
| `src/screen/dashboard.rs` | Effect ハンドラ追加・注文結果 Message 追加・`subscription()` にポーリング追加 |
| `src/connector.rs` | `pub mod order;` 追記 |
| `src/connector/order.rs` | **新規** 注文・照会・余力・保有株数取得関数 |
| `src/style.rs` | 注文状態・売買・追証警告色追加 |
