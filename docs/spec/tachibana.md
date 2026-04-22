# 立花証券 e支店 API 統合仕様書

> **関連ドキュメント**:
> | 知りたいこと | 参照先 |
> |---|---|
> | リプレイエンジン全体（状態モデル・HTTP API） | [replay.md](replay.md) |
> | 注文パネル・仮想約定エンジンの UI/型設計 | [order.md](order.md) |
> | 立花証券リプレイ固有の設計判断 | [本書 §8](#8-リプレイ対応の設計判断) |

**最終更新**: 2026-04-17
**対象コード**: `exchange/src/adapter/tachibana.rs` および関連ファイル

---

## 目次

1. [概要](#1-概要)
2. [API プロトコル仕様](#2-api-プロトコル仕様)
   - [2.1 エンドポイント](#21-エンドポイント)
   - [2.2 認証 (CLMAuthLoginRequest)](#22-認証-clmauthloginrequest)
   - [2.3 REST API 共通仕様](#23-rest-api-共通仕様)
   - [2.4 レスポンスフィールド名の命名規則](#24-レスポンスフィールド名の命名規則)
   - [2.5 時価情報 (CLMMfdsGetMarketPrice)](#25-時価情報-clmmfdsgetmarketprice)
   - [2.6 日足履歴 (CLMMfdsGetMarketPriceHistory)](#26-日足履歴-clmmfdsgetmarketpricehistory)
   - [2.7 MASTER I/F（銘柄マスタダウンロード）](#27-master-if銘柄マスタダウンロード)
   - [2.8 EVENT I/F（リアルタイム配信）](#28-event-ifリアルタイム配信)
3. [実装詳細](#3-実装詳細)
   - [3.1 主要な型定義](#31-主要な型定義)
   - [3.2 主要な関数](#32-主要な関数)
   - [3.3 Static 変数](#33-static-変数)
   - [3.4 ストリーム接続](#34-ストリーム接続)
   - [3.5 セッション管理](#35-セッション管理)
   - [3.6 起動フロー](#36-起動フロー)
   - [3.7 日足取得](#37-日足取得)
   - [3.8 銘柄検索](#38-銘柄検索)
   - [3.9 Exchange/Venue 定義](#39-exchangevenue-定義)
   - [3.10 注文機能](#310-注文機能)
   - [3.11 HTTP API（flowsurface 側ファサード）](#311-http-apiflowsurface-側ファサード)
4. [制約・前提条件](#4-制約前提条件)
5. [テスト](#5-テスト)
6. [未対応・将来課題](#6-未対応将来課題)
7. [リプレイ対応の設計判断](#7-リプレイ対応の設計判断)
8. [公式サンプル参照情報](#8-公式サンプル参照情報)
9. [変更ファイル一覧](#9-変更ファイル一覧)
10. [付録: プロトコル実装ノート](#10-付録-プロトコル実装ノート)

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

> **⚠️ 注意**: 0 からカウントを始めると前セッションより小さくなり拒否される。起動時にエポック秒でカウンタを初期化することで、再起動後も単調増加が保証される。

### 2.4 レスポンスフィールド名の命名規則

リクエストとレスポンスで名称が異なる:
- リクエスト `sCLMID`: `CLMMfdsGetMarketPriceHistory`（Get 付き）
- レスポンス配列キー: `aCLMMfdsMarketPriceHistory`（Get **なし**）
- 同様: `CLMMfdsGetMarketPrice` → `aCLMMfdsMarketPrice`

> **⚠️ 注意**: `#[serde(rename)]` の名称を誤ると `#[serde(default)]` により不一致が静かに無視され、空の配列/文字列が返る。Serde エラーが出ない分、実行時まで気づきにくい。

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

**範囲フィルタ**: API 自体は範囲指定を受け付けず常に全履歴を返す。クライアント側で `klines.retain(|k| k.time >= start && k.time <= end)` を行う（§7.2 参照）。

### 2.7 MASTER I/F（銘柄マスタダウンロード）

EVENT I/F とは別プロトコル。`sUrlMaster` に HTTP **GET** でストリーミング。

```
GET {sUrlMaster}?{"p_no":"...","p_sd_date":"...","sCLMID":"CLMEventDownload","sJsonOfmt":"4"}
```

- レスポンス: Shift-JIS エンコード、JSON オブジェクトの連続（`}` でレコード境界を判定）
- 全マスタ種類を一括配信（約21MB）。`CLMIssueMstKabu` レコードのみ抽出
- `sCLMID == "CLMEventDownloadComplete"` で配信完了

> **⚠️ Shift-JIS の `}` (0x7D) 境界判定**: Shift-JIS の2バイト文字の第2バイト範囲に `}` (0x7D) が含まれるため、偽のレコード境界が検出される。`parse_sjis_stream_records(data: &[u8]) -> Vec<Vec<u8>>` で対処。Shift-JIS リードバイト（`0x81..=0x9F | 0xE0..=0xEF`）を検出したら `in_multibyte = true` フラグを立て、トレイルバイトを `}` 境界判定から除外する。

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

WebSocket (`sUrlEventWebSocket`) は `fastwebsockets` + `hyper` のハンドシェイクでサーバーが HTTP 400 を返すため使用不可。**HTTP Long-polling (`sUrlEvent`)** を採用。`reqwest` の `bytes_stream()` でストリーミング。

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

> **⚠️ 注意**: パラメータ順序を変えると 400 エラー。`reqwest` の `.query()` はアルファベット順にソートするため使用不可。手動で固定順序のクエリ文字列を組み立てる。

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

> **⚠️ 必須**: `p_cmd` チェックなしで処理すると、FD フレーム内の `DPP`（終値）/ `DV`（出来高）が Trade として誤パースされクラッシュする。`p_cmd=ST` の場合のみ Trade を生成、`p_cmd=FD` の場合のみ板情報を生成する。

#### 板情報フィールド（FD コマンド）

| フィールド | 意味 |
|-----------|------|
| `p_1_GAP1`〜`p_1_GAP10` | 売気配価格（最良→上位） |
| `p_1_GAV1`〜`p_1_GAV10` | 売気配数量 |
| `p_1_GBP1`〜`p_1_GBP10` | 買気配価格（最良→下位） |
| `p_1_GBV1`〜`p_1_GBV10` | 買気配数量 |
| `p_1_QAP` / `p_1_QBP` | 最良売/買気配価格（スカラー） |
| `p_1_QOV` / `p_1_QUV` | OVER / UNDER 数量 |

> **⚠️ フィールド名**: 板フィールドは `GAP`/`GAV`/`GBP`/`GBV` であり `QAP1`〜`QAP10` ではない。末尾マッチ（`_GAP`/`_GBP`）でフィールドを抽出する。

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

> **注意**: 銘柄検索に Tachibana が表示されない場合は以下の 3 層を確認すること:
> 1. `fetch_ticker_metadata` がキャッシュを返しているか（マスタDL完了か）
> 2. `ticker_rows` に `UpdateMetadata` 経由でデフォルト stats が追加されたか
> 3. `Settings.selected_exchanges` に `Venue::Tachibana` が含まれているか

### 3.9 Exchange/Venue 定義 (`exchange/src/adapter.rs`)

- `Exchange::Tachibana`: `MarketKind::Spot`、`Venue::Tachibana` に対応
- `fetch_ticker_metadata(Tachibana)`: `cached_ticker_metadata()` からキャッシュ返却
- `fetch_ticker_stats(Tachibana)`: 空の HashMap を返す（未実装）
- `fetch_klines(Tachibana)`: `InvalidRequest` エラー（`fetch_daily_history()` を使用）
- `supports_kline_timeframe(Tachibana)`: `Timeframe::D1` のみ

### 3.10 注文機能 (`src/connector/order.rs`)

`src/connector/auth.rs` / `fetcher.rs` と同じパターンで、`exchange` クレートの注文 API 関数をラップする。`Task::perform` から直接呼び出せるよう、引数にセッション・クライアントを取らず `get_session()` で内部取得する。詳細な型定義・UI 設計は [docs/spec/order.md](order.md) を参照。

| 関数 | 用途 |
|------|------|
| `submit_new_order(req)` | 新規注文（`NewOrderRequest` → `NewOrderResponse`） |
| `submit_correct_order(req)` | 訂正注文（`CorrectOrderRequest` → `ModifyOrderResponse`） |
| `submit_cancel_order(req)` | 取消注文（`CancelOrderRequest` → `ModifyOrderResponse`） |
| `fetch_orders(eig_day)` | 注文一覧取得 |
| `fetch_order_detail(order_num, eig_day)` | 約定明細取得 |
| `fetch_buying_power()` | 現物余力・信用余力を **並列取得** (`tokio::join!`) して返す |
| `fetch_holdings(issue_code)` | 売付可能株数取得（全数量ボタン用） |

注文パネル（`src/screen/dashboard/panel/`）:

| ファイル | 型 | 概要 |
|---------|-----|------|
| `order_entry.rs` | `OrderEntryPanel` | 売買区分・価格種別・口座区分・現物/信用・数量入力・発注ボタン。`build_request()` で `NewOrderRequest` を構築。 |
| `order_list.rs` | `OrderListPanel` | 注文一覧の表示・取消・訂正。`newly_executed()` で約定済み注文を通知。 |
| `buying_power.rs` | `BuyingPowerPanel` | 現物買付余力・信用余力を表示。 |

**注文リクエストの注意点**:
- `second_password`（発注パスワード）は `Debug` を手動実装し `[REDACTED]` でマスク
- `side`: `"1"` = 売、`"3"` = 買（立花証券 API フィールド値。HTTP API の `"buy"`/`"sell"` とは異なる）
- `cash_margin`: `"0"` = 現物、`"2"` = 信用新規(制度6ヶ月)、`"4"` = 信用返済(制度)、`"6"` = 信用新規(一般)、`"8"` = 信用返済(一般)
- `condition`: `"0"` = 指定なし、`"2"` = 寄付、`"4"` = 引け、`"6"` = 不成
- `price`: `"0"` = 成行、数値文字列 = 指値

### 3.11 HTTP API（flowsurface 側ファサード）

`src/replay_api.rs`（ポート 9876）が立花証券注文・照会機能を HTTP でも公開している。E2E テストと外部エージェントが同一 API を利用する。**全エンドポイントでログイン済みセッションが必須**（未ログイン時は `{"error": "..."}` を返す）。

| メソッド | パス | リクエスト | レスポンス |
|---|---|---|---|
| GET  | `/api/buying-power` | — | 現物 + 信用余力（下記）|
| POST | `/api/tachibana/order` | `NewOrderBody`（下記）| 新規注文受付結果（下記）|
| GET  | `/api/tachibana/orders[?eig_day=YYYYMMDD]` | — | `{"orders": [OrderRecord, ...]}` |
| GET  | `/api/tachibana/order/:order_num[?eig_day=YYYYMMDD]` | — | `{"executions": [ExecutionRecord, ...]}` |
| POST | `/api/tachibana/order/correct` | `CorrectOrderBody`（下記）| 訂正結果（下記 `ModifyResult`）|
| POST | `/api/tachibana/order/cancel` | `CancelOrderBody`（下記）| 取消結果（下記 `ModifyResult`）|
| GET  | `/api/tachibana/holdings?issue_code=XXXX` | — | `{"holdings_qty": <u64>}` |

いずれのエンドポイントも失敗時は HTTP 200 で `{"error": "<message>"}` を返す（リプレイ API 共通の応答規約）。

#### `GET /api/buying-power`

`connector::order::fetch_buying_power()` が現物余力 (`CLMZanKaiSummary`) と信用余力 (`CLMMarginZanKaiSummary`) を `tokio::join!` で並列取得する。

```jsonc
{
  "cash_buying_power":       "1234567",   // 現物買付余力（円）文字列
  "nisa_growth_buying_power": "500000",   // NISA 成長投資枠（円）
  "shortage_flag":           "0",         // 不足フラグ（立花生値）
  "margin_new_order_power":  "2000000",   // 信用新規建余力
  "maintenance_margin_rate": "30.00",     // 委託保証金維持率 %
  "margin_call_flag":        "0"          // 追証フラグ
}
```

> **命名規則**: 立花 API の生レスポンス（`CLMZanKaiSummary` の `sUritukeKanougaku` 等）を snake_case に正規化してから返す。[§2.4 レスポンスフィールド名の命名規則](#24-レスポンスフィールド名の命名規則) を参照。

#### `POST /api/tachibana/order`（新規注文）

`NewOrderBody`:

```jsonc
{
  "issue_code":      "7203",       // 必須。銘柄コード
  "qty":             "100",        // 必須。数量（文字列）
  "side":            "3",          // 必須。"1"=売 "3"=買（立花 API 生値）
  "price":           "0",          // 必須。"0"=成行 / 数値文字列=指値
  "account_type":    "1",          // 任意（default "1"）。口座区分
  "market_code":     "00",         // 任意（default "00"）。市場コード
  "condition":       "0",          // 任意（default "0"）。執行条件
  "cash_margin":     "0",          // 任意（default "0"）。現物 / 信用区分
  "expire_day":      "0",          // 任意（default "0"）。当日=0
  "second_password": "xxxx"        // 必須（本番ビルド）。下記フォールバック参照
}
```

各フィールドの意味は [§3.10 注文機能](#310-注文機能-srcconnectororderrs) を参照。

**`second_password` のフォールバック**:
- 本番ビルド (`--release`): ボディに含まれないと 400。
- debug ビルド: ボディ省略時は環境変数 `DEV_SECOND_PASSWORD` をフォールバックとして使用する。E2E テストで平文パスワードを JSON に載せないための措置。

レスポンス:

```jsonc
{
  "order_number":    "12345678",
  "eig_day":         "20260422",
  "delivery_amount": "4205000",
  "commission":      "275",
  "consumption_tax": "27",
  "order_datetime":  "20260422090000",
  "warning_code":    "0000",
  "warning_text":    ""
}
```

#### `GET /api/tachibana/orders`

`eig_day` (YYYYMMDD) を渡すと当日以外の注文も絞れる。省略時は全件。

`OrderRecord`:

```jsonc
{
  "order_num":      "12345678",
  "issue_code":     "7203",
  "order_qty":      "100",
  "current_qty":    "100",
  "order_price":    "4205",
  "order_datetime": "20260422090000",
  "status_text":    "注文中",
  "executed_qty":   "0",
  "executed_price": "0",
  "eig_day":        "20260422"
}
```

#### `GET /api/tachibana/order/:order_num`

特定注文の約定明細を取得。`eig_day` を渡すと当該営業日の約定に絞る。

`ExecutionRecord`:

```jsonc
{
  "exec_qty":      "100",
  "exec_price":    "4205",
  "exec_datetime": "20260422090015"
}
```

#### `POST /api/tachibana/order/correct`（訂正）

`CorrectOrderBody`:

```jsonc
{
  "order_number":    "12345678",   // 必須
  "eig_day":         "20260422",   // 必須
  "condition":       "0",          // 任意（default "*"=変更なし）
  "price":           "4200",       // 任意（default "*"）
  "qty":             "100",        // 任意（default "*"）
  "expire_day":      "0",          // 任意（default "*"）
  "second_password": "xxxx"        // debug では DEV_SECOND_PASSWORD フォールバックあり
}
```

立花 API の訂正仕様に従い、**変更しないフィールドは `"*"` を送る**。ボディ省略時のデフォルト `"*"` はこのためのもの。

#### `POST /api/tachibana/order/cancel`（取消）

`CancelOrderBody`:

```jsonc
{
  "order_number":    "12345678",
  "eig_day":         "20260422",
  "second_password": "xxxx"
}
```

#### `ModifyResult`（訂正 / 取消共通のレスポンス）

```jsonc
{
  "order_number":   "12345678",
  "eig_day":        "20260422",
  "order_datetime": "20260422091200"
}
```

#### `GET /api/tachibana/holdings?issue_code=XXXX`

`issue_code` は必須。売付可能株数（保有数量）を返す。全数量ボタンの初期値表示で使用する。

```jsonc
{ "holdings_qty": 300 }
```

---

## 4. 制約・前提条件

1. **HTTP Long-polling 方式**: WebSocket (`fastwebsockets`) は立花証券サーバーが 400 を返すため不使用
2. **東証立会時間のみ**: 9:00-11:30, 12:30-15:30 JST。時間外は板データ更新なし
3. **板は10本板**: 最良気配から10本（売10+買10）
4. **単一銘柄**: 現在は1ストリームにつき1銘柄。`p_gyou_no` 複数指定で拡張可能
5. **日足のみ**: 分足・時間足は提供されない。`supports_kline_timeframe` は `D1` のみ
6. **調整値使用**: `daily_record_to_kline` はデフォルトで `use_adjusted: true`

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
| TOYOTA (7203) 板受信（売10本+買10本） | ✅ |
| KP（現在値）定期受信（約5秒間隔） | ✅ |
| TOYOTA (7203) 日足取得（約25年分、6189件） | ✅ |
| KIOXIAHD (285A) 日足チャート表示（317件） | ✅ |
| 銘柄マスタDL（4207件） | ✅ |
| "7203" 検索で TOYOTA 表示 | ✅ |

---

## 6. 未対応・将来課題

| # | 課題 | 優先度 |
|---|------|:------:|
| 1 | `fetch_ticker_stats` の実装（全銘柄現在値取得） | 中 |
| 2 | セッション切れ検出 → 自動再ログイン | 中 |
| 3 | KP（現在値）の Ladder 反映 | 中 |
| 4 | WebSocket 対応（`tokio-tungstenite` 等） | 低 |
| 5 | 複数銘柄の同時板表示（`p_gyou_no` 複数指定） | 低 |
| 6 | OVER/UNDER 数量 / VWAP の表示 | 低 |
| 7 | 呼値テーブル (CLMYobine) による正確な min_ticksize | 低 |
| 8 | `spawn_init_issue_master()` の削除（未使用デッドコード） | 低 |

---

## 7. リプレイ対応の設計判断

立花証券のリプレイサポート（D1 のみ）は、他取引所とは異なる 4 つの API 制約から独自の設計判断を伴っている。本節はその意思決定の背景を記録する。実装の仕様と操作手順は [docs/replay.md](replay.md) §10「取引所別対応状況」を参照。

### 7.1 なぜ D1 のみ対応か

立花証券 API が提供する Kline は日足のみで、分足・時間足のエンドポイントが存在しない。`supports_kline_timeframe` が `D1` のみを返すのはこの制約によるもので、将来 API が拡張されない限り timeframe 追加は不可能。

**Trades / Depth 未対応の理由**:
- **Trades（歩み値）**: 過去の歩み値を取得する API が存在しない。EVENT I/F の ST コマンドはリアルタイム配信のみで、履歴クエリができない
- **Depth（板情報）**: 過去の板スナップショットを取得する API が存在しない

結果として立花証券でのリプレイは「D1 kline の段階表示」のみのサポートとなる。Heatmap / Ladder / TimeAndSales ペインはリプレイ中に意味のあるデータを表示できない。

### 7.2 なぜ range フィルタが post-fetch か

`fetch_tachibana_daily_klines(issue_code, range)` の実装方針として、**API リクエストに range 引数を渡さず、取得後にクライアント側で `klines.retain(|k| k.time >= start && k.time <= end)` を行う**。

**理由**: 立花証券 API 自体が範囲指定を受け付けず、常に全履歴（約 20 年分 ≒ 数千本）を返す仕様のため、post-fetch フィルタ以外の選択肢がない。非効率ではあるが、日足データは軽量（1 銘柄あたり 数百 KB 程度）なので実用上の問題はない。

**代替案として検討したが却下**:
- API 側の範囲指定 → 存在しない
- ローカルキャッシュ → 初回取得の重複は避けられず、複雑化するだけ
- マルチリクエスト分割 → API 側が分割に対応していない

### 7.3 なぜ kline timestamp ベースの離散ステップか

`StepForward` / `StepBackward` は他 timeframe では「次/前のバー境界への離散ジャンプ」と同じ統一経路を使うが、D1 の場合には **休場日（土日祝）を自動スキップする** 必要がある。

固定幅 `±86_400_000ms`（1 日）では休場日のタイムスタンプで止まり、対応する kline が存在しない「空振りステップ」が発生する。プリフェッチ済みの EventStore の klines から `next_time_after(current)` / `prev_time_before(current)` で実データ由来の timestamp を取得することで、休場日を自動的に飛び越せる。

**他取引所への波及**: 離散ステップ統一は Phase 6 で全 timeframe に適用され、M1〜D1 すべて同じ `next_time_after` 経路を使うようになった。立花証券 D1 の要件がトリガーだったが、設計としては全取引所共通のメリットがある。

### 7.4 なぜ D1 自動再生に粗補正モードが必要だったか

> **アーキテクチャ注記（R3 以降）**: 本節で述べた「粗補正モード（`COARSE_CUTOFF_MS` / `COARSE_BAR_MS`）」は R3 アーキテクチャ刷新で廃止済み。現在は `StepClock + EventStore + dispatch_tick` による統一ステップ制御に置き換えられており、D1 も `BASE_STEP_DELAY_MS = 100ms`（1 ステップ = 1 本）で進行する。本節は「なぜ粗補正が必要だったか」という設計判断の背景として残している。

リプレイエンジンの基本は「実時間連動」（M1 なら 1x = 実時間 60 秒で 1 本進む）だが、D1 にこれを適用すると **1 本進むのに実時間 24 時間、最大 10x でも 2.4 時間** かかり実用不能になる。

解決策として `COARSE_CUTOFF_MS = 3_600_000ms`（1 時間）境界を導入し、`delta_to_next >= COARSE_CUTOFF_MS` の場合は threshold を `COARSE_BAR_MS = 1_000ms`（1 秒）に切り替える「粗補正モード」を採用した。これにより 1x で 1 秒/本、10x で 100ms/本 で D1 が進行した。

### 7.5 なぜ `is_all_d1_klines()` 分岐を撤廃したか

初期の Tachibana Phase 3 実装では、Tick ハンドラを `is_all_d1_klines()` による 2 分岐（D1 専用 / 非 D1 専用）にしていた。しかしこの all-or-nothing 判定はペイン構成に依存するため、**M1 + D1 混在ペイン** で D1 側が実質停止する問題があった。

Phase 6 で `process_tick()` による統一経路へ移行し、`is_all_d1_klines()` / `advance_d1` / `process_d1_tick` を削除した。統一経路は `delta_to_next` ベースの threshold 切替（§7.4）で D1 要件を満たしつつ、M1+D1 混在も自然に扱える。

### 7.6 日足自動再生の UX 判断

Phase 2 では「D1 リプレイは Play 押下で `Paused` 開始」としていたが、Phase 3 で自動再生を有効化した（`resume_status = Playing`）。背景:

- 手動 Step のみだと長期間のヒストリカルスキャンが不便
- 粗補正モード（§7.4）導入で 1x = 1 本/秒、10x = 10 本/秒と実用時間で進行可能になった
- 手動 Step は引き続き併用できる（Playing 中でも Step ボタンは有効）

ユーザーが「次の足を予測したい」ユースケースでは Pause → Step が有効で、連続スキャンには Play が有効という使い分けが可能。

---

## 8. 公式サンプル参照情報

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

## 9. 変更ファイル一覧

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

---

## 10. 付録: プロトコル実装ノート

本節は開発中に発見したプロトコルの挙動・制約をまとめたものである。各項目の仕様上の意味は本文の関連セクションに統合済み。

### 接続・認証

| # | 現象 | 原因 | 対処 |
|---|------|------|------|
| 1 | EVENT I/F が HTTP 400 を返す | `fastwebsockets` + `hyper` の WebSocket ハンドシェイクを拒否 | HTTP Long-polling (`sUrlEvent`) に切替（§2.8 参照） |
| 2 | `p_errno: 6` で全リクエストが拒否される | アプリ再起動時に `p_no` が前セッションより小さい | `AtomicU64` をエポック秒で初期化（§2.3 参照） |

### データパース

| # | 現象 | 原因 | 対処 |
|---|------|------|------|
| 3 | `fields_to_depth()` で板データが取得できない | フィールド名を `QAP1`〜`QAP10` と仮定していたが実データは `GAP1`〜`GAP10` | `_GAP`/`_GBP` の末尾マッチに変更（§2.8 参照） |
| 4 | 板受信時に `assertion failed: p.y.is_finite()` クラッシュ | FD フレームの `DPP`/`DV` が Trade として誤パース | `p_cmd=ST` でのみ Trade を生成（§2.8 参照） |
| 5 | `DailyHistoryResponse` が空の配列を返す | `#[serde(rename)]` に `aCLMMfdsGetMarketPriceHistory`（Get 付き）を誤指定 | `aCLMMfdsMarketPriceHistory`（Get なし）に修正（§2.4 参照） |
| 6 | MASTER DL で1113件がパース失敗 | Shift-JIS 2バイト文字の第2バイトに `}` が含まれ偽の境界検出 | `parse_sjis_stream_records()` で Shift-JIS 対応バイト列読み込み（§2.7 参照） |

### アプリ統合

| # | 現象 | 原因 | 対処 |
|---|------|------|------|
| 7 | 銘柄検索に Tachibana が表示されない | `selected_exchanges` への自動追加・`ticker_rows` のデフォルト stats 作成・マスタDL完了の 3 つが欠落 | `Settings` の `Deserialize` 拡張 + `UpdateMetadata` ハンドラで行作成（§3.8 参照） |
