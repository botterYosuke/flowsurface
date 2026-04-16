# 立花証券 e支店 API 統合仕様書

**最終更新**: 2026-04-16
**対象コード**: `exchange/src/adapter/tachibana.rs` および関連ファイル

---

## 1. 概要

flowsurface に立花証券 e支店 API を統合し、国内株式のチャート・板情報・歩み値を表示する。

### アーキテクチャ

```
flowsurface/
├── src/                # Iced GUI (ダッシュボード・チャート・ウィジェット)
│   ├── main.rs         # アプリエントリ・セッション復元・ログインフロー
│   ├── connector/
│   │   ├── auth.rs     # セッション管理 (メモリ + keyring)
│   │   ├── fetcher.rs  # 日足取得 (Tachibana 分岐)
│   │   └── order.rs    # 注文 API ラッパー (Task::perform から呼び出し)
│   └── screen/
│       ├── login.rs    # ログイン画面
│       └── dashboard/
│           ├── tickers_table.rs  # 銘柄検索・表示
│           └── panel/
│               ├── order_entry.rs   # 注文入力パネル
│               ├── order_list.rs    # 注文一覧パネル
│               └── buying_power.rs  # 余力表示パネル
├── exchange/           # 取引所アダプター
│   └── src/
│       ├── adapter.rs      # Exchange/Venue enum, fetch_ticker_metadata 等
│       ├── adapter/
│       │   └── tachibana.rs # 立花証券 API クライアント (テスト110件)
│       └── connect.rs      # depth/trade/kline ストリーム
└── data/               # データモデル・設定
    └── src/config/
        └── tachibana.rs # keyring 永続化
```

### データフロー全体図

```
立花証券 API
 ├─ 認証: POST {BASE_URL}/auth/ → TachibanaSession (仮想URL群)
 ├─ MASTER I/F: GET {sUrlMaster}?{json} → 銘柄マスタ (Shift-JIS ストリーミング)
 ├─ 時価情報: POST {sUrlPrice} → 日足履歴 / スナップショット
 └─ EVENT I/F: GET {sUrlEvent}?{params} → 板情報・歩み値 (HTTP Long-polling)

        ↓ exchange/src/adapter/tachibana.rs

 ├─ TachibanaSession → auth.rs (メモリ + keyring 永続化)
 ├─ MasterRecord → cached_ticker_metadata() → TickersTable
 ├─ DailyHistoryRecord → daily_record_to_kline() → KlineChart
 ├─ FD フレーム → fields_to_depth() → DepthPayload → Ladder
 └─ ST フレーム → fields_to_trade() → Trade → Ladder
```

---

## 2. API プロトコル仕様

### 2.1 エンドポイント

| 環境 | URL |
|------|-----|
| 本番 | `https://kabuka.e-shiten.jp/e_api_v4r8/` |
| デモ | `https://demo-kabuka.e-shiten.jp/e_api_v4r8/` |

### 2.2 認証 (CLMAuthLoginRequest)

```
POST {BASE_URL}/auth/
Content-Type: application/json
Body: {"sCLMID":"CLMAuthLoginRequest","sUserId":"xxx","sPassword":"yyy","sJsonOfmt":"5","p_no":"...","p_sd_date":"..."}
```

- パスワードは URL エンコード必須（`urlencoding::encode()`）
- `sJsonOfmt: "5"` で JSON キー名付き応答を取得
- `p_no` はリクエスト通番（`AtomicU64` カウンタで動的生成）

応答で仮想 URL 群を取得:

| フィールド | 用途 |
|-----------|------|
| `sUrlRequest` | 業務機能 |
| `sUrlMaster` | マスタダウンロード |
| `sUrlPrice` | 時価情報・日足履歴 |
| `sUrlEvent` | EVENT I/F (HTTP Long-polling) |
| `sUrlEventWebSocket` | EVENT I/F (WebSocket) ※未使用 |

**制約**:
- 電話認証が事前に必要（ユーザー手動）
- `sKinsyouhouMidokuFlg: "1"` の場合は仮想URLが空 → `UnreadNotices` エラー
- 仮想 URL は 1 日有効。失効時は `p_errno = "2"`

### 2.3 REST API 共通仕様

- HTTP メソッド: **POST**（v4r8 で GET → POST に変更）
- Body: JSON 文字列を直接送信（`reqwest::Client::json()` ではなく `.body()` を使用）
- レスポンス: **Shift-JIS** エンコード → `encoding_rs::SHIFT_JIS` でデコード
- 数値フィールドは全て文字列型（`"pDPP": "3250"`）。`"*"` は未取得を意味する
- URL 形式が独自: `reqwest::Client::get(url).query(&params)` は不可。JSON を body に直接送信

**エラーチェック**: `ApiResponse<T>` ラッパーで `p_errno` → `sResultCode` の順に検査

| `p_errno` | 意味 |
|-----------|------|
| `0` / `""` | 正常 |
| `2` | セッション切断 |
| `-62` | 稼働時間外 |

**リクエスト通番 (`p_no`)**: アプリ再起動時に `p_no` が前セッションの値を下回ると `p_errno: 6` で拒否される。`AtomicU64` カウンタを `compare_exchange(0, epoch_secs)` で初期化し、常に前回値を超える。

### 2.4 レスポンスフィールド名の命名規則

リクエストとレスポンスで名称が異なる:
- リクエスト `sCLMID`: `CLMMfdsGetMarketPriceHistory`（Get 付き）
- レスポンス配列キー: `aCLMMfdsMarketPriceHistory`（Get **なし**）
- 同様: `CLMMfdsGetMarketPrice` → `aCLMMfdsMarketPrice`

### 2.5 時価情報 (CLMMfdsGetMarketPrice)

最大120銘柄を同時取得可能。

主要情報コード:
- `pDPP` = 現在値、`pDOP` = 始値、`pDHP` = 高値、`pDLP` = 安値
- `pDV` = 出来高、`pPRP` = 前日終値、`tDPP:T` = 現在値時刻

### 2.6 日足履歴 (CLMMfdsGetMarketPriceHistory)

1リクエスト1銘柄。最大約20年分の日足データ（OHLCV）。
株式分割調整値も提供（`*xK` フィールド）。

| フィールド | 意味 | 調整値 |
|-----------|------|--------|
| `pDOP` | 始値 | `pDOPxK` |
| `pDHP` | 高値 | `pDHPxK` |
| `pDLP` | 安値 | `pDLPxK` |
| `pDPP` | 終値 | `pDPPxK` |
| `pDV` | 出来高 | `pDVxK` |
| `sDate` | 日付 (YYYYMMDD) | — |

日付の epoch 変換: JST 深夜0時 → `date_str_to_epoch_ms()` で UTC ミリ秒に変換。

### 2.7 MASTER I/F（銘柄マスタダウンロード）

EVENT I/F とは別プロトコル。`sUrlMaster` に HTTP **GET** でストリーミング。

```
GET {sUrlMaster}?{"p_no":"...","p_sd_date":"...","sCLMID":"CLMEventDownload","sJsonOfmt":"4"}
```

- レスポンス: Shift-JIS エンコード、JSON オブジェクトの連続（`}` でレコード境界を判定）
- 全マスタ種類を一括配信（約21MB）。`CLMIssueMstKabu` レコードのみ抽出
- `sCLMID == "CLMEventDownloadComplete"` で配信完了

**CLMIssueMstKabu の利用可能フィールド**（readme.txt 明記分のみ）:

| フィールド | 意味 | 例 |
|-----------|------|-----|
| `sIssueCode` | 銘柄コード | `"7203"` |
| `sIssueName` | 銘柄名称 | `"トヨタ自動車"` |
| `sIssueNameRyaku` | 略称 | `"トヨタ"` |
| `sIssueNameKana` | カナ名 | `"トヨタ  ジドウシヤ"` |
| `sIssueNameEizi` | 英語名 | `"TOYOTA"` |
| `sYusenSizyou` | 優先市場 | `"00"` (東証) |
| `sGyousyuCode` | 業種コード | `"0050"` |
| `sGyousyuName` | 業種名 | `"水産・農林業"` |

### 2.8 EVENT I/F（リアルタイム配信）

#### 接続方式

WebSocket (`sUrlEventWebSocket`) は `fastwebsockets` + `hyper` のハンドシェイクで HTTP 400 拒否。
**HTTP Long-polling (`sUrlEvent`)** を採用。`reqwest` の `bytes_stream()` でストリーミング。

参照: 公式サンプル `e_api_sample_v4r8.py` の `requests.session().get(url, stream=True).iter_lines()` 方式。

#### パラメータ形式

JSON ではなく **URL クエリストリング形式**。パラメータの**順序は固定**（変更不可）。

```
p_rid=22&p_board_no=1000&p_gyou_no=1&p_mkt_code=00&p_eno=0&p_evt_cmd=ST,KP,FD&p_issue_code=7203
```

| パラメータ | 意味 | 値 | 順序 |
|-----------|------|-----|:---:|
| `p_rid` | リクエストID | `22`（固定値） | 1（先頭、変更不可） |
| `p_board_no` | ボード番号 | `1000`（固定値） | 2 |
| `p_gyou_no` | 行番号（銘柄スロット） | `1` | 3 |
| `p_mkt_code` | 市場コード（東証=`00`） | `00` | 4 |
| `p_eno` | イベント番号（開始位置） | `0` | 5 |
| `p_evt_cmd` | 購読コマンド | `ST,KP,FD` | 6 |
| `p_issue_code` | 銘柄コード | `7203` | 7（最後） |

参照: `docs/e-shiten/samples/e_api_websocket_receive_tel.py` 573行目「先頭の項目。順番の変更は不可。」

#### 受信データフォーマット

```
\x01  (SOH) = レコード（項目値）区切り
\x02  (STX) = カラム名:値 区切り
\x03  (ETX) = 値のサブ区切り（複数値フィールド内）
```

- エンコーディング: **ASCII**（REST の Shift-JIS とは異なる）
- HTTP Long-polling では **1行 = 1イベント**（改行区切り）

#### フィールド名の命名規則

`p_{行番号}_{情報コード}` 形式。例: `p_1_DPP\x023319` → フィールド名 `p_1_DPP`、値 `3319`

参照: `e_api_websocket_receive_tel.py` 752行目「`p_1_DPP` は、p:プレーン文字列_1:行番号_DPP:現在値」

#### コマンド判別

各イベント行の `p_cmd` フィールドでコマンドを判定:
- `p_cmd=FD` → 板情報 → `fields_to_depth()` で処理
- `p_cmd=ST` → 歩み値 → `fields_to_trade()` で処理
- `p_cmd=KP` → 現在値 → 現在は未処理（約5秒間隔で受信）

**重要**: `p_cmd` チェックなしで処理すると、FD フレーム内の `DPP`（終値）/ `DV`（出来高）が Trade として誤パースされクラッシュする。

#### 板情報フィールド（FD コマンド）

| フィールド | 意味 |
|-----------|------|
| `p_1_GAP1`〜`p_1_GAP10` | 売気配価格（最良→上位） |
| `p_1_GAV1`〜`p_1_GAV10` | 売気配数量 |
| `p_1_GBP1`〜`p_1_GBP10` | 買気配価格（最良→下位） |
| `p_1_GBV1`〜`p_1_GBV10` | 買気配数量 |
| `p_1_QAP` / `p_1_QBP` | 最良売/買気配価格（スカラー） |
| `p_1_QOV` / `p_1_QUV` | OVER / UNDER 数量 |

**注意**: 板フィールドは `GAP`/`GAV`/`GBP`/`GBV` であり、当初想定の `QAP`/`QBP` ではない。

#### FD フレーム全フィールド（TOYOTA 7203 実データ、69フィールド）

```
p_no, p_date, p_cmd(=FD),
p_1_AV, p_1_BV, p_1_DHF, p_1_DHP, p_1_DHP:T, p_1_DJ,
p_1_DLF, p_1_DLP, p_1_DLP:T, p_1_DOP, p_1_DOP:T,
p_1_DPG, p_1_DPP, p_1_DPP:T, p_1_DV, p_1_DYRP, p_1_DYWP,
p_1_GAP1〜GAP10, p_1_GAV1〜GAV10,
p_1_GBP1〜GBP10, p_1_GBV1〜GBV10,
p_1_LISS, p_1_PRP, p_1_QAP, p_1_QAS, p_1_QBP, p_1_QBS,
p_1_QOV, p_1_QUV, p_1_VWAP
```

#### 歩み値フィールド（ST コマンド）

| フィールド | 意味 |
|-----------|------|
| `p_1_DPP` | 約定価格 |
| `p_1_DV` | 約定数量 |
| `p_1_DPP:T` | 約定時刻 |
| `p_1_DYSS` | 売買区分 (`"1"` = 売) |

---

## 3. 実装詳細

### 3.1 主要な型定義 (`exchange/src/adapter/tachibana.rs`)

| 型 | 用途 |
|----|------|
| `TachibanaSession` | 仮想URL群（`Serialize`/`Deserialize` 対応、keyring 永続化用） |
| `TachibanaError` | `LoginFailed` / `UnreadNotices` / `Http` / `Json` / `ApiError { code, message }` |
| `ApiResponse<T>` | 業務API 共通ラッパー（`p_errno`/`sResultCode` + `#[serde(flatten)] data: T`） |
| `LoginRequest` / `LoginResponse` | 認証リクエスト/レスポンス |
| `MarketPriceRequest` / `MarketPriceRecord` / `MarketPriceResponse` | スナップショット取得 |
| `DailyHistoryRequest` / `DailyHistoryRecord` / `DailyHistoryResponse` | 日足履歴取得 |
| `MasterRecord` | マスタダウンロードの各レコード（`sCLMID` で種別判定） |
| `NewOrderRequest` / `NewOrderResponse` | CLMKabuNewOrder 新規注文（`second_password` は `Debug` でマスク） |
| `CorrectOrderRequest` / `CancelOrderRequest` | CLMKabuCorrectOrder / CLMKabuCancelOrder 訂正・取消注文 |
| `ModifyOrderResponse` | 訂正・取消注文共通レスポンス |
| `OrderListRequest` / `OrderListResponse` / `OrderRecord` | CLMOrderList 注文一覧 |
| `OrderDetailRequest` / `OrderDetailResponse` / `ExecutionRecord` | CLMOrderListDetail 約定明細 |
| `BuyingPowerResponse` | CLMZanKaiKanougaku 現物買付余力 |
| `MarginPowerResponse` | CLMZanShinkiKanoIjiritu 信用新規可能委託保証金率 |
| `GenbutuKabuRequest` / `GenbutuKabuResponse` / `HoldingRecord` | CLMGenbutuKabuList 現物保有株数 |

### 3.2 主要な関数

#### REST API

| 関数 | 用途 |
|------|------|
| `login(client, base_url, user_id, password)` | 認証。パスワードを URL エンコードして POST |
| `fetch_market_prices(client, session, issue_codes)` | スナップショット取得（最大120銘柄） |
| `fetch_daily_history(client, session, issue_code)` | 日足履歴取得（最大約20年分） |
| `validate_session(client, session)` | セッション有効性検証（`p_errno` で判定） |
| `daily_record_to_kline(record, use_adjusted)` | `DailyHistoryRecord` → `Kline` 変換 |
| `next_p_no()` | リクエスト通番の動的生成 |

#### MASTER I/F

| 関数 | 用途 |
|------|------|
| `fetch_all_master(client, session)` | 全マスタ一括ストリーミングDL → `CLMIssueMstKabu` のみ抽出 |
| `init_issue_master(client, session)` | DL + `ISSUE_MASTER_CACHE` に格納 |
| `get_cached_issue_master()` | キャッシュ済みマスタを返す |
| `cached_ticker_metadata()` | キャッシュから `HashMap<Ticker, Option<TickerInfo>>` を構築 |
| `master_record_to_ticker_info(record)` | `MasterRecord` → `(Ticker, TickerInfo)` 変換 |

#### EVENT I/F

| 関数 | 用途 |
|------|------|
| `parse_event_frame(data)` | SOH/STX 区切りパーサー |
| `fields_to_depth(fields)` | FD コマンドから板情報を抽出（`_GAP`/`_GBP` 末尾マッチ） |
| `fields_to_trade(fields)` | ST コマンドから歩み値を抽出（`_DPP`/`_DV` 末尾マッチ） |
| `build_event_params(issue_code, market_code)` | パラメータ構築（公式準拠の固定順序） |
| `connect_event_stream(ticker_info, push_freq)` | HTTP Long-polling ストリーム（`impl Stream<Item = Event>`） |
| `set_event_http_url(url)` / `set_event_ws_url(url)` | URL の static 保持 |

#### 注文 API

| 関数 | 用途 |
|------|------|
| `serialize_order_request(req, clm_id)` | 注文リクエストに `p_no`/`p_sd_date`/`sCLMID`/`sJsonOfmt` を付与して JSON 化 |
| `submit_new_order(client, session, req)` | CLMKabuNewOrder — 新規注文発注 |
| `submit_correct_order(client, session, req)` | CLMKabuCorrectOrder — 訂正注文発注 |
| `submit_cancel_order(client, session, req)` | CLMKabuCancelOrder — 取消注文発注 |
| `fetch_orders(client, session, eig_day)` | CLMOrderList — 注文一覧取得 |
| `fetch_order_detail(client, session, order_num, eig_day)` | CLMOrderListDetail — 約定明細取得 |
| `fetch_buying_power(client, session)` | CLMZanKaiKanougaku — 現物買付余力取得 |
| `fetch_margin_power(client, session)` | CLMZanShinkiKanoIjiritu — 信用余力取得 |
| `fetch_holdings(client, session, issue_code)` | CLMGenbutuKabuList — 売付可能株数取得（保有なし → `Ok(0)`） |

### 3.3 Static 変数

| 変数 | 型 | 用途 |
|------|-----|------|
| `REQUEST_COUNTER` | `AtomicU64` | リクエスト通番カウンタ |
| `ISSUE_MASTER_CACHE` | `RwLock<Option<Arc<Vec<MasterRecord>>>>` | 銘柄マスタキャッシュ |
| `EVENT_HTTP_URL` | `RwLock<Option<String>>` | HTTP Long-polling URL |
| `EVENT_WS_URL` | `RwLock<Option<String>>` | WebSocket URL（将来用） |

### 3.10 注文機能 (`src/connector/order.rs`)

`src/connector/auth.rs` / `fetcher.rs` と同じパターンで、`exchange` クレートの注文 API 関数をラップする。`Task::perform` から直接呼び出せるよう、引数にセッション・クライアントを取らず `get_session()` で内部取得する。

| 関数 | 用途 |
|------|------|
| `submit_new_order(req)` | 新規注文（`NewOrderRequest` → `NewOrderResponse`） |
| `submit_correct_order(req)` | 訂正注文（`CorrectOrderRequest` → `ModifyOrderResponse`） |
| `submit_cancel_order(req)` | 取消注文（`CancelOrderRequest` → `ModifyOrderResponse`） |
| `fetch_orders(eig_day)` | 注文一覧取得 |
| `fetch_order_detail(order_num, eig_day)` | 約定明細取得 |
| `fetch_buying_power()` | 現物余力・信用余力を **並列取得** (`tokio::join!`) して返す |
| `fetch_holdings(issue_code)` | 売付可能株数取得（全数量ボタン用） |

### 3.11 注文パネル (`src/screen/dashboard/panel/`)

| ファイル | 型 | 概要 |
|---------|-----|------|
| `order_entry.rs` | `OrderEntryPanel` | 売買区分・価格種別・口座区分・現物/信用・数量入力・発注ボタン。`build_request()` で `NewOrderRequest` を構築。 |
| `order_list.rs` | `OrderListPanel` | 注文一覧の表示・取消・訂正。`newly_executed()` で約定済み注文を通知。 |
| `buying_power.rs` | `BuyingPowerPanel` | 現物買付余力・信用余力を表示。 |

**注文リクエストの注意点**:
- `second_password`（発注パスワード）は `Debug` を手動実装し `[REDACTED]` でマスク
- `side`: `"1"` = 売、`"3"` = 買
- `cash_margin`: `"0"` = 現物、`"2"` = 信用新規(制度6ヶ月)、`"4"` = 信用返済(制度)、`"6"` = 信用新規(一般)、`"8"` = 信用返済(一般)
- `condition`: `"0"` = 指定なし、`"2"` = 寄付、`"4"` = 引け、`"6"` = 不成
- `price`: `"0"` = 成行、数値文字列 = 指値

### 3.4 ストリーム接続 (`exchange/src/connect.rs`)

| ストリーム | Tachibana の実装 |
|-----------|-----------------|
| `depth_stream` | `connect_event_stream()` を呼び出し（FD + ST を処理） |
| `trade_stream` | `futures::stream::empty()`（depth_stream 内で TradesReceived も発行するため） |
| `kline_stream` | `futures::stream::empty()`（日足のみ、REST ポーリング） |

### 3.5 セッション管理 (`src/connector/auth.rs`)

| 関数 | 用途 |
|------|------|
| `store_session(session)` | メモリに保存 + `set_event_ws_url()` / `set_event_http_url()` 呼び出し |
| `get_session()` | メモリからセッション取得 |
| `persist_session(session)` | keyring に JSON 保存 |
| `try_restore_session()` | keyring から復元 → `validate_session()` で検証 → 失効なら削除 |
| `clear_session()` | メモリ + keyring から削除 |
| `perform_login(user_id, password, is_demo)` | ログイン実行（`TachibanaError` → ユーザー向けメッセージ変換） |

keyring 設定（`data/src/config/tachibana.rs`）:
- service: `"flowsurface.tachibana"`, key: `"session"`

### 3.6 起動フロー (`src/main.rs`)

```
起動（ウィンドウなし）
 ├─ Task::batch[launch_sidebar, try_restore_session()]
 │
 ├─ SessionRestoreResult(Some(session))
 │    → store_session() → transition_to_dashboard()
 │    → .chain(start_master_download(session))
 │
 ├─ SessionRestoreResult(None)
 │    → ログインウィンドウを遅延表示
 │
 └─ LoginCompleted(Ok(session))
      → store_session() + persist_session()
      → transition_to_dashboard()
      → .chain(start_master_download(session))
```

`start_master_download()`: `init_issue_master()` → `cached_ticker_metadata()` → `UpdateMetadata(Tachibana, metadata)` メッセージ送信。

### 3.7 日足取得 (`src/connector/fetcher.rs`)

`kline_fetch_task` 内で `Venue::Tachibana` を判定し、`fetch_tachibana_daily_klines(issue_code)` に分岐。
この関数は `get_session()` → `fetch_daily_history()` → `daily_record_to_kline(_, true)`（調整値使用）の一連を実行。

### 3.8 銘柄検索 (`src/screen/dashboard/tickers_table.rs`)

`UpdateMetadata` ハンドラで、`tickers_info` に新規追加したティッカーのうち `ticker_rows` に未登録のものについて、デフォルト stats（`mark_price=0, daily_price_chg=0, volume=0`）で `TickerRowData` を作成。
これにより `fetch_ticker_stats` が未実装でも銘柄検索に表示される。

`Settings` のカスタム `Deserialize` 実装で、保存済みリストに存在しない新 `Venue` を自動補完。

### 3.9 Exchange/Venue 定義 (`exchange/src/adapter.rs`)

- `Exchange::Tachibana`: `MarketKind::Spot`、`Venue::Tachibana` に対応
- `fetch_ticker_metadata(Tachibana)`: `cached_ticker_metadata()` からキャッシュ返却
- `fetch_ticker_stats(Tachibana)`: 空の HashMap を返す（未実装）
- `fetch_klines(Tachibana)`: `InvalidRequest` エラー（`fetch_daily_history()` を使用）
- `supports_kline_timeframe(Tachibana)`: `Timeframe::D1` のみ

---

## 4. 発見された問題と解決策

### 4.1 WebSocket 400 エラー

`fastwebsockets` + `hyper` の WebSocket ハンドシェイクが立花証券サーバーで HTTP 400 拒否。
**解決**: HTTP Long-polling (`sUrlEvent`) にフォールバック。

### 4.2 パラメータ順序の制約

EVENT I/F のパラメータ順序を変えると 400 エラー。
**解決**: 公式準拠の順序に固定（`p_rid` が先頭、`p_issue_code` が最後）。

### 4.3 板フィールド名の不一致（GAP vs QAP）

当初 `QAP1`〜`QAP10` と想定したが、実データは `GAP1`〜`GAP10`。
**解決**: `_GAP`/`_GAV`/`_GBP`/`_GBV` の末尾マッチに変更。

### 4.4 FD フレームでの Trade 誤生成（クラッシュ）

FD フレーム内の `DPP`（終値）/ `DV`（出来高）が Trade として誤パースされ、`assertion failed: p.y.is_finite()` でクラッシュ。
**解決**: `p_cmd=ST` の場合のみ Trade を生成。`p_cmd=FD` の場合のみ板情報を生成。

### 4.5 クロスヘア描画のゼロ除算パニック

`src/chart/indicator/plot.rs`: Y 軸レンジがゼロ（`lowest == highest`）の場合にゼロ除算で NaN が発生。
**解決**: `span.abs() < f32::EPSILON` の場合はクロスヘア描画をスキップ。

### 4.6 保存状態から復元した KlineChart が表示されない

`insert_hist_klines()` が `latest_x` を更新していなかった。保存状態復元時は `latest_x = 0`（1970年）のまま固定され、実データ範囲と重ならない。
**解決**: `insert_hist_klines()` で `latest_x` を更新。新データが追加されなかった場合は `mark_failed()` で再試行を防止。

### 4.7 レスポンスフィールド名の不一致

`DailyHistoryResponse` の `#[serde(rename)]` が `aCLMMfdsGetMarketPriceHistory`（Get 付き）だったが、実際のレスポンスは `aCLMMfdsMarketPriceHistory`（Get なし）。`#[serde(default)]` により不一致が静かに無視されていた。
**解決**: リネーム修正。

### 4.8 p_no カウンターのリセット問題

アプリ再起動時に `p_no` が 1 からリセットされ、`p_errno: 6` で拒否。
**解決**: `REQUEST_COUNTER` を `compare_exchange(0, epoch_secs)` で初期化。

### 4.9 銘柄検索に Tachibana が表示されない（3層構造）

| 原因 | 内容 | 解決 |
|------|------|------|
| 1 | `fetch_ticker_metadata` が空を返す | マスタDL + キャッシュ実装 |
| 2 | `ticker_rows` が `TickerStats` なしでは作成されない | `UpdateMetadata` でデフォルト stats による行作成 |
| 3 | `selected_exchanges` に Tachibana が欠落 | `Settings` の `Deserialize` で新 Venue 自動追加 |
| 4 | マスタDLが完了しない（全21MB待ち） | `CLMIssueMstKabu` 区間終了後に早期リターン |
| 5 | 非ASCII display 名でパニック | `display.filter(\|d\| d.is_ascii())` でフォールバック |

### 4.10 Shift-JIS の `}` (0x7D) 境界判定問題 ✅ 解決済み

Shift-JIS の2バイト文字の第2バイト範囲に `}` (0x7D) が含まれるため、偽のレコード境界が検出される問題。
実測で **1113件** のパース失敗を確認（4207件は正常取得）していたが解決済み。

**解決**: `parse_sjis_stream_records(data: &[u8]) -> Vec<Vec<u8>>` を実装。
Shift-JIS リードバイト（`0x81..=0x9F | 0xE0..=0xEF`）を検出したら `in_multibyte = true` フラグを立て、
トレイルバイトを `}` 境界判定から除外する。`fetch_all_master` の内部でもこのロジックを適用している。

---

## 5. テスト

### テスト数

| クレート | テスト数 |
|---------|------:|
| `exchange` (tachibana.rs) | 110 |
| `flowsurface` (auth.rs 等) | 6 |
| `data` | 3 |
| **合計** | **119** |

※ コードベース全体（リプレイ・チャート・connector 等含む）では約383件のテストが存在する。

### 実データテスト結果

| テスト | 結果 |
|--------|------|
| TOYOTA (7203) 板受信（売10本+買10本） | OK |
| KP（現在値）定期受信（約5秒間隔） | OK |
| TOYOTA (7203) 日足取得（約25年分、6189件） | OK |
| KIOXIAHD (285A) 日足チャート表示（317件） | OK |
| 銘柄マスタDL（4207件） | OK |
| "7203" 検索で TOYOTA 表示 | OK |

---

## 6. 制約・前提条件

1. **HTTP Long-polling 方式**: WebSocket (`fastwebsockets`) は 400 エラーのため不使用
2. **東証立会時間のみ**: 9:00-11:30, 12:30-15:30 JST。時間外は板データ更新なし
3. **板は10本板**: 最良気配から10本（売10+買10）
4. **単一銘柄**: 現在は1ストリームにつき1銘柄。`p_gyou_no` 複数指定で拡張可能
5. **日足のみ**: 分足・時間足は提供されない。`supports_kline_timeframe` は `D1` のみ
6. **調整値使用**: `daily_record_to_kline` はデフォルトで `use_adjusted: true`

---

## 7. 未対応・将来課題

| # | 課題 | 優先度 |
|---|------|:------:|
| 1 | `fetch_ticker_stats` の実装（全銘柄現在値取得） | 中 |
| 2 | セッション切れ検出 → 自動再ログイン | 中 |
| 3 | KP（現在値）の Ladder 反映 | 中 |
| 4 | ~~Shift-JIS `}` 境界判定の堅牢化~~ → ✅ 解決済み (§4.10) | — |
| 5 | WebSocket 対応（`tokio-tungstenite` 等） | 低 |
| 6 | 複数銘柄の同時板表示（`p_gyou_no` 複数指定） | 低 |
| 7 | OVER/UNDER 数量 / VWAP の表示 | 低 |
| 8 | 呼値テーブル (CLMYobine) による正確な min_ticksize | 低 |
| 9 | `spawn_init_issue_master()` の削除（未使用デッドコード） | 低 |
| 10 | 旧取引所アダプター削除（Binance, Bybit, OKX, MEXC, Hyperliquid） | 低 |

---

## 8. リプレイ対応の設計判断

立花証券のリプレイサポート（D1 のみ）は、他取引所とは異なる 4 つの API 制約から独自の設計判断を伴っている。本節はその意思決定の背景を記録する。実装の仕様と操作手順は [docs/replay_header.md](replay_header.md) §10 を参照。

### 8.1 なぜ D1 のみ対応か

立花証券 API が提供する Kline は日足のみで、分足・時間足のエンドポイントが存在しない。`supports_kline_timeframe` が `D1` のみを返すのはこの制約によるもので、将来 API が拡張されない限り timeframe 追加は不可能。

**Trades / Depth 未対応の理由**:
- **Trades（歩み値）**: 過去の歩み値を取得する API が存在しない。EVENT I/F の ST コマンドはリアルタイム配信のみで、履歴クエリができない
- **Depth（板情報）**: 過去の板スナップショットを取得する API が存在しない

結果として立花証券でのリプレイは「D1 kline の段階表示」のみのサポートとなる。Heatmap / Ladder / TimeAndSales ペインはリプレイ中に意味のあるデータを表示できない。

### 8.2 なぜ range フィルタが post-fetch か

`fetch_tachibana_daily_klines(issue_code, range)` の実装方針として、**API リクエストに range 引数を渡さず、取得後にクライアント側で `klines.retain(|k| k.time >= start && k.time <= end)` を行う**。

**理由**: 立花証券 API 自体が範囲指定を受け付けず、常に全履歴（約 20 年分 ≒ 数千本）を返す仕様のため、post-fetch フィルタ以外の選択肢がない。非効率ではあるが、日足データは軽量（1 銘柄あたり 数百 KB 程度）なので実用上の問題はない。

**代替案として検討したが却下**:
- API 側の範囲指定 → 存在しない
- ローカルキャッシュ → 初回取得の重複は避けられず、複雑化するだけ
- マルチリクエスト分割 → API 側が分割に対応していない

### 8.3 なぜ kline timestamp ベースの離散ステップか

`StepForward` / `StepBackward` は他 timeframe では「次/前のバー境界への離散ジャンプ」と同じ統一経路を使うが、D1 の場合には **休場日（土日祝）を自動スキップする** 必要がある。

固定幅 `±86_400_000ms`（1 日）では休場日のタイムスタンプで止まり、対応する kline が存在しない「空振りステップ」が発生する。プリフェッチ済みの `ReplayKlineBuffer.klines` から `next_time_after(current)` / `prev_time_before(current)` で実データ由来の timestamp を取得することで、休場日を自動的に飛び越せる。

**他取引所への波及**: 離散ステップ統一は Phase 6 で全 timeframe に適用され、M1〜D1 すべて同じ `next_time_after` 経路を使うようになった。立花証券 D1 の要件がトリガーだったが、設計としては全取引所共通のメリットがある。

### 8.4 なぜ D1 自動再生に粗補正モードが必須か

リプレイエンジンの基本は「実時間連動」（M1 なら 1x = 実時間 60 秒で 1 本進む）だが、D1 にこれを適用すると **1 本進むのに実時間 24 時間、最大 10x でも 2.4 時間** かかり実用不能になる。

解決策として `COARSE_CUTOFF_MS = 3_600_000ms`（1 時間）境界を導入し、`delta_to_next >= COARSE_CUTOFF_MS` の場合は threshold を `COARSE_BAR_MS = 1_000ms`（1 秒）に切り替える「粗補正モード」を採用した。これにより:

- 1x → 1 秒 / 本
- 10x → 100 ms / 本

で D1 が進行する。詳細なアルゴリズムと代替案の比較は [docs/replay_header.md](replay_header.md) §7.1 / §15.2 を参照。

**立花証券特有の副次要件**: `drain_all_trade_buffers()` が `pending_trade_streams` を除外するスキップ条件を持つため、trade_buffers が空の Tachibana ケースでは drain が no-op になり、無駄な iteration が発生しない。一方で **Binance の D1 kline + Heatmap ペイン混在** ケースでは drain が必要なので、単純な「D1 なら drain スキップ」ではなく trade_buffers の空判定ベースにした経緯がある（Phase 3 実装判断）。

### 8.5 なぜ `is_all_d1_klines()` 分岐を撤廃したか

初期の Tachibana Phase 3 実装では、Tick ハンドラを `is_all_d1_klines()` による 2 分岐（D1 専用 / 非 D1 専用）にしていた。しかしこの all-or-nothing 判定はペイン構成に依存するため、**M1 + D1 混在ペイン** で D1 側が実質停止する問題があった（非 D1 経路に落ちるため D1 は 24 時間/本ペース）。

Phase 6 で `process_tick()` による統一経路へ移行し、`is_all_d1_klines()` / `advance_d1` / `process_d1_tick` を削除した。統一経路は `delta_to_next` ベースの threshold 切替（§8.4）で D1 要件を満たしつつ、M1+D1 混在も自然に扱える。

この設計変更の背景と代替案の比較は [docs/replay_header.md](replay_header.md) §15.2 を参照。

### 8.6 日足自動再生の UX 判断

Phase 2 では「D1 リプレイは Play 押下で `Paused` 開始」としていたが、Phase 3 で自動再生を有効化した（`resume_status = Playing`）。背景:

- 手動 Step のみだと長期間のヒストリカルスキャンが不便
- 粗補正モード（§8.4）導入で 1x = 1 本/秒、10x = 10 本/秒と実用時間で進行可能になった
- 手動 Step は引き続き併用できる（Playing 中でも Step ボタンは有効）

ユーザーが「次の足を予測したい」ユースケースでは Pause → Step が有効で、連続スキャンには Play が有効という使い分けが可能。

---

## 9. 公式サンプル参照情報

| ファイル | 参照箇所 | 内容 |
|---------|---------|------|
| `e_api_websocket_receive_tel.py` | 573-589行目 | `func_make_websocket_url()` — パラメータ順序仕様 |
| 同上 | 745-749行目 | 区切り子仕様: SOH, STX, ETX |
| 同上 | 752行目 | フィールド名形式 `p_{行番号}_{情報コード}` |
| `e_api_sample_v4r8.py` | 412-415行目 | HTTP Long-polling 実装 |
| 同上 | 460-468行目 | SOH/STX パーサー |
| `e_api_get_master_tel.py` | 461-528行目 | マスタDLストリーミング |

サンプルファイルの場所: `docs/e-shiten/samples/`

---

## 10. 変更ファイル一覧

| ファイル | 内容 |
|---------|------|
| `exchange/src/adapter/tachibana.rs` | 立花証券 API クライアント本体（テスト110件含む）、注文型・注文API追加 |
| `exchange/src/adapter.rs` | `Exchange::Tachibana` / `Venue::Tachibana` 定義、`fetch_ticker_metadata` 等 |
| `exchange/src/connect.rs` | `depth_stream` → `connect_event_stream`、trade/kline は空 |
| `exchange/Cargo.toml` | `urlencoding = "2"`, reqwest `stream` feature 追加 |
| `src/main.rs` | `SessionRestoreResult` / `LoginCompleted` / `start_master_download` |
| `src/connector/auth.rs` | セッション管理（メモリ + keyring） |
| `src/connector/fetcher.rs` | `fetch_tachibana_daily_klines()` |
| `src/connector/order.rs` | 注文 API ラッパー（`submit_new_order` / `fetch_orders` / `fetch_buying_power` 等） |
| `src/screen/login.rs` | ユーザーID入力・デモ/本番切替・電話認証案内 |
| `src/screen/dashboard/tickers_table.rs` | `UpdateMetadata` でデフォルト stats による行作成 |
| `src/screen/dashboard/panel/order_entry.rs` | 注文入力パネル（売買・価格種別・数量・発注） |
| `src/screen/dashboard/panel/order_list.rs` | 注文一覧パネル（訂正・取消） |
| `src/screen/dashboard/panel/buying_power.rs` | 現物/信用余力表示パネル |
| `data/src/config/tachibana.rs` | keyring への保存/読込/削除 |
| `data/src/config.rs` | `pub mod tachibana;` |
| `data/src/tickers_table.rs` | `Settings` のカスタム `Deserialize`（新 Venue 自動追加） |
| `src/chart/indicator/plot.rs` | クロスヘア描画のゼロ除算ガード |
| `src/chart/kline.rs` | `insert_hist_klines` の `latest_x` 更新 + 再試行防止 |
| `src/style.rs` | `Venue::Tachibana` の venue_icon（暫定 `Icon::Star`） |
