# 立花証券 e支店 API 統合仕様書

**最終更新**: 2026-04-12
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
│   │   └── fetcher.rs  # 日足取得 (Tachibana 分岐)
│   └── screen/
│       ├── login.rs    # ログイン画面
│       └── dashboard/
│           └── tickers_table.rs  # 銘柄検索・表示
├── exchange/           # 取引所アダプター
│   └── src/
│       ├── adapter.rs      # Exchange/Venue enum, fetch_ticker_metadata 等
│       ├── adapter/
│       │   └── tachibana.rs # 立花証券 API クライアント (テスト95件)
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

### 4.10 Shift-JIS の `}` (0x7D) 境界判定問題

Shift-JIS の2バイト文字の第2バイト範囲に `}` (0x7D) が含まれるため、偽のレコード境界が検出される。
実測で **1113件** のパース失敗を確認（4207件は正常取得）。
**未解決**: パース失敗時に `buf.clear()` せず次の `}` まで蓄積し続ける修正が必要。

---

## 5. テスト

### テスト数

| クレート | テスト数 |
|---------|------:|
| `exchange` (tachibana.rs) | 95 |
| `flowsurface` (auth.rs, fetcher.rs 等) | 20 |
| `data` | 3 |
| **合計** | **118** |

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
| 4 | Shift-JIS `}` 境界判定の堅牢化（パース失敗1113件の解消） | 中 |
| 5 | WebSocket 対応（`tokio-tungstenite` 等） | 低 |
| 6 | 複数銘柄の同時板表示（`p_gyou_no` 複数指定） | 低 |
| 7 | OVER/UNDER 数量 / VWAP の表示 | 低 |
| 8 | 呼値テーブル (CLMYobine) による正確な min_ticksize | 低 |
| 9 | `spawn_init_issue_master()` の削除（未使用デッドコード） | 低 |
| 10 | 旧取引所アダプター削除（Binance, Bybit, OKX, MEXC, Hyperliquid） | 低 |

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
| `exchange/src/adapter/tachibana.rs` | 立花証券 API クライアント本体（テスト95件含む） |
| `exchange/src/adapter.rs` | `Exchange::Tachibana` / `Venue::Tachibana` 定義、`fetch_ticker_metadata` 等 |
| `exchange/src/connect.rs` | `depth_stream` → `connect_event_stream`、trade/kline は空 |
| `exchange/Cargo.toml` | `urlencoding = "2"`, reqwest `stream` feature 追加 |
| `src/main.rs` | `SessionRestoreResult` / `LoginCompleted` / `start_master_download` |
| `src/connector/auth.rs` | セッション管理（メモリ + keyring） |
| `src/connector/fetcher.rs` | `fetch_tachibana_daily_klines()` |
| `src/screen/login.rs` | ユーザーID入力・デモ/本番切替・電話認証案内 |
| `src/screen/dashboard/tickers_table.rs` | `UpdateMetadata` でデフォルト stats による行作成 |
| `data/src/config/tachibana.rs` | keyring への保存/読込/削除 |
| `data/src/config.rs` | `pub mod tachibana;` |
| `data/src/tickers_table.rs` | `Settings` のカスタム `Deserialize`（新 Venue 自動追加） |
| `src/chart/indicator/plot.rs` | クロスヘア描画のゼロ除算ガード |
| `src/chart/kline.rs` | `insert_hist_klines` の `latest_x` 更新 + 再試行防止 |
| `src/style.rs` | `Venue::Tachibana` の venue_icon（暫定 `Icon::Star`） |
