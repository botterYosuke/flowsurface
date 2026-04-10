# 立花証券 e支店 API 対応 移行プラン

## 概要

flowsurface（暗号資産チャートアプリ）を立花証券 e支店 API を使った国内株チャートアプリに変える。

---

## 現状の理解

### 現在のアーキテクチャ
```
flowsurface/
├── src/              # Iced GUI (ダッシュボード・チャート・ウィジェット)
├── exchange/         # 取引所アダプター (Binance, Bybit, OKX, MEXC, Hyperliquid)
└── data/             # データモデル・設定・集計
```

- **GUI**: Rust + Iced 0.14 (WGPU レンダリング)
- **通信**: reqwest (REST) + fastwebsockets (WebSocket)
- **データ**: TimeSeries / TickAggr 集計、足・ヒートマップ・フットプリント・板

### 現在のデータフロー
```
取引所 REST / WebSocket
    ↓ exchange/adapter/*.rs (取引所別パーサー)
    ↓ connector/stream.rs (ストリーム管理)
    ↓ connector/fetcher.rs (履歴データ)
    ↓ data/aggr/* (時間・ティック集計)
    ↓ src/chart/* (チャート描画)
```

---

## 立花証券 e支店 API 概要

### エンドポイント
- **本番**: `https://kabuka.e-shiten.jp/e_api_v4r8/`
- **デモ**: `https://demo-kabuka.e-shiten.jp/e_api_v4r8/`

### アクセスモデル
```
認証機能: {BASE_URL}/auth/?{JSON引数}
業務機能: {sUrlRequest}?{JSON引数}   ← ログイン応答で取得
マスタ機能: {sUrlMaster}?{JSON引数}
時価情報: {sUrlPrice}?{JSON引数}
EVENT I/F: {sUrlEvent}?{JSON引数}    (long-polling)
EVENT WS:  {sUrlEventWebSocket}      (WebSocket)
```

すべての引数は URL の `?` 以降に **JSON文字列** で渡す。

### 主要 API

#### 1. 認証 (CLMAuthLoginRequest)
```json
// 要求
{"sCLMID":"CLMAuthLoginRequest","sUserId":"xxx","sPassword":"yyy"}

// 応答 (重要フィールド)
{
  "sCLMID": "CLMAuthLoginAck",
  "sResultCode": "0",
  "sUrlRequest":  "https://...(仮想URL)/",
  "sUrlMaster":   "https://...(仮想URL)/",
  "sUrlPrice":    "https://...(仮想URL)/",
  "sUrlEvent":    "https://...(仮想URL)/",
  "sUrlEventWebSocket": "wss://...(仮想URL)/"
}
```
- 電話認証が必要（事前に電話して認証済みの状態でログイン）
- 仮想URLはセッション毎に異なる（1日有効）
- 未読書面がある場合 (`sKinsyouhouMidokuFlg:"1"`) は仮想URLが空で利用不可

#### 2. 時価情報問合取得 (スナップショット)
```json
// 要求: {sUrlPrice}?{"sCLMID":"CLMMfdsGetMarketPrice","sTargetIssueCode":"6501,7203","sTargetColumn":"pDPP,pDOP,pDHP,pDLP,pDV,tDPP:T"}
// 応答
{
  "aCLMMfdsMarketPrice": [
    {"sIssueCode":"6501", "pDPP":"xxxx", "pDOP":"yyyy", ...}
  ]
}
```
主要情報コード:
- `pDPP` = 現在値（終値）
- `pDOP` = 始値, `pDHP` = 高値, `pDLP` = 安値
- `pDV`  = 出来高
- `pPRP` = 前日終値
- `tDPP:T` = 現在値時刻

最大120銘柄まで同時取得可能。

#### 3. 蓄積情報問合取得 (日足履歴)
```json
// 要求: {sUrlPrice}?{"sCLMID":"CLMMfdsGetMarketPriceHistory","sIssueCode":"6501","sSizyouC":"00"}
// 応答
{
  "aCLMMfdsGetMarketPriceHistory": [
    {"sDate":"YYYYMMDD","pDOP":"xxx","pDHP":"xxx","pDLP":"xxx","pDPP":"xxx","pDV":"xxx",
     "pDOPxK":"xxx","pDHPxK":"xxx","pDLPxK":"xxx","pDPPxK":"xxx","pDVxK":"xxx"}
  ]
}
```
- 最大約20年分の日足データ（OHLCV）
- 株式分割調整値も提供 (`*xK` フィールド)
- 1リクエスト1銘柄

#### 4. リアルタイム配信 (EVENT I/F)
- WebSocket: `sUrlEventWebSocket` に接続 → 時価・注文約定通知を受信
- Long-polling: `sUrlEvent` にポーリング
- 詳細プロトコルは `api_event_if_v4r7.pdf` 参照（公開HTMLには記載なし）

#### 5. 板情報
- 公式説明: 「リアルタイム株価や**板情報の取得**が可能」「株価（4本値、**板**、日足など）」
- REST スナップショット: `CLMMfdsGetMarketPrice` に板情報コードを指定して取得（ポーリング前提）
- リアルタイム: EVENT I/F でボード配信として流れてくる設計（`api_event_if_v4r7.pdf` 参照）
- **制限**: サンプル記載「FD 設定しても時価は配信されない」→ FD 経由のリアルタイム板は制限あり
- 結論: 板データはあるが「kabuステーション」と比べると副次的な扱い。HFT・板トレード用途には不十分。

#### 5. 銘柄マスタ
- `CLMIssueMstOther` でインデックス・為替銘柄コード取得
- 銘柄詳細情報問合取得で銘柄名・市場等を取得

---

## 移行戦略

**方針**: 既存の Iced GUI・チャートレンダリング・集計ロジックをできる限り活かしつつ、
exchange crate の Binance 等を削除し、立花証券アダプターに置き換える。

---

## フェーズ別実装計画

### フェーズ 0: 調査・準備
- ✅ `exchange/src/adapter.rs` の `Exchange` enum を確認。`Tachibana` 追加可能と判断
- [ ] 現在の `login.rs` スクリーンを流用できるか確認
- [ ] EVENT I/F の接続プロトコル詳細を確認（WebSocketサブプロトコル等）

**調査結果メモ（2026-04-09）**:
- `Exchange` enum は `enum-map::Enum` derive 使用。新バリアント追加は機械的に全 match arm を追加するだけ
- `Venue` と `Exchange` は分離している。`Exchange::Tachibana` + `Venue::Tachibana` をペアで追加
- `connect.rs` の depth/trade/kline stream は暫定的に `futures::stream::empty()` を返すスタブで対応
- `src/style.rs` の venue_icon は暫定的に `Icon::Star` を使用（TODO: 専用アイコン追加要）

### フェーズ 1: 立花証券 API クライアント実装

**ファイル**: `exchange/src/adapter/tachibana.rs`

実装内容:
1. **認証マネージャー** ✅
   - `login(client, base_url, user_id, password)` → `TachibanaSession` を返す
   - `sKinsyouhouMidokuFlg` チェック（"1" なら `UnreadNotices` エラー）
   - セッション状態は呼び出し側で `TachibanaSession` を保持する責務
   - logout は未実装（TODO）

2. **REST クライアント** ✅
   - `fetch_market_prices(client, session, issue_codes)` → スナップショット取得（最大120銘柄）
   - `fetch_daily_history(client, session, issue_code)` → 日足履歴取得（最大約20年分）
   - URL構築: `build_api_url(base_url, json)` → `{virtual_url}?{json_string}` 形式

3. **WebSocket クライアント** (未実装 - TODO Phase 3)
   - `connect_event_ws(ws_url)` → EVENT I/F WebSocket接続
   - 時価更新イベントを `Exchange::Event` に変換

**実装済みの型定義**:
- `TachibanaError` (thiserror): LoginFailed / UnreadNotices / Http / Json / ApiError
- `TachibanaSession`: 仮想URL群（url_request/master/price/event/event_ws）
- `LoginRequest` / `LoginResponse`: sCLMID="CLMAuthLoginRequest/Ack"
- `MarketPriceRequest` / `MarketPriceRecord` / `MarketPriceResponse`: CLMMfdsGetMarketPrice
- `DailyHistoryRequest` / `DailyHistoryRecord` / `DailyHistoryResponse`: CLMMfdsGetMarketPriceHistory
- `build_api_url()` / `build_api_url_from()`: URL構築ユーティリティ

**TDD テスト結果 (2026-04-10)**:
- 50テスト PASS（exchange crate 全テスト）
- mockito で HTTP クライアントの統合テストも完備
- p_no 並行ユニークネステスト、validate_session 未知 p_errno テスト追加済み

**設計上の注意点**:
- すべての数値が文字列型で返される（"pDPP": "3250" など）。表示時に `parse::<f64>()` が必要
- 値が "*" の場合は未取得/非対応を意味する。表示前に確認が必要
- URL形式が独自: reqwest の `.query()` は使えない。`?{json}` を手動で付加

**型定義**:
```rust
struct TachibanaSession {
    url_request: String,
    url_master: String,
    url_price: String,
    url_event: String,
    url_event_ws: String,
}

struct MarketPriceSnapshot {
    issue_code: String,
    current_price: f64,   // pDPP
    open: f64,            // pDOP
    high: f64,            // pDHP
    low: f64,             // pDLP
    volume: f64,          // pDV
    prev_close: f64,      // pPRP
    timestamp: DateTime<Jst>,
}

struct DailyCandle {
    date: NaiveDate,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    // split-adjusted
    open_adj: f64,
    high_adj: f64,
    low_adj: f64,
    close_adj: f64,
    volume_adj: f64,
}
```

### フェーズ 2: データモデル適応

**ファイル群**: `data/src/`

変更内容:

1. **`exchange/src/adapter.rs`**（正しいファイル、`data/src/stream.rs` ではない）
   - ✅ `Exchange::Tachibana` / `Venue::Tachibana` を追加（既存取引所との並走 - Phase 5 で削除）
   - `StreamKind` を株向けに: `DailyCandles`, `RealtimePrice`, `OrderBook`（TODO）
   - `Ticker` 型を「銘柄コード（4桁）+ 市場コード」に対応（TODO）

**実装メモ（2026-04-09）**:
- `Exchange::Tachibana` は `MarketKind::Spot` で分類している（暫定。将来的に専用の MarketKind を検討）
- `Venue::Tachibana` を `Venue::ALL` に追加したため `tickers_table.rs` のドロップダウンに立花証券が出現する
- `available_markets(Venue::Tachibana)` は `&[MarketKind::Spot]` を返す
- `fetch_ticker_metadata` / `fetch_ticker_stats` / `fetch_klines` は暫定的に空/エラーを返す

2. **`data/src/aggr/time.rs`**
   - 日足データのみなので分・時間足は廃止 or 制限
   - 「1日」以上の時間軸のみサポート（日足・週足・月足を日足から計算）

3. **`data/src/tickers_table.rs`**
   - 銘柄リスト管理を日本株コードベースに
   - 銘柄名（日本語）対応（Shift-JIS → UTF-8 変換）

4. **`data/src/chart/kline.rs`**
   - 日足 OHLCV データの受け入れ
   - 分割調整の切り替え（調整あり/なし）

5. **`data/src/panel/`**
   - `ladder.rs` (板情報): 板データは EVENT I/F ストリーミングで取得可能だが制約あり（後述）
     → MVP では REST スナップショット（`CLMMfdsGetMarketPrice` で板情報コード指定）で実装し、後でストリーミングに昇格
   - `timeandsales.rs`: EVENT I/F からのティックデータに対応

### フェーズ 3: コネクタ適応

**ファイル群**: `src/connector/`

1. **`src/connector/fetcher.rs`**
   - `fetch_klines` → `fetch_daily_history(issue_code)` に変更
   - レート制限: 立花 API のレート制限に合わせる（仕様確認要）
   - 銘柄変更時に履歴再取得

2. **`src/connector/stream.rs`**
   - WebSocket 接続先を EVENT I/F に変更
   - セッション認証後に接続
   - 接続 URL は毎ログイン時に更新（仮想URL）

3. **新規: `src/connector/auth.rs`**（既存 `login.rs` を参考に）
   - セッション管理
   - 自動再ログイン（セッション切れ対応）

### フェーズ 4: UI 変更

**ファイル群**: `src/screen/`, `src/modal/`

1. **`src/screen/login.rs`（既存を改修）**
   - フィールド: ユーザーID + パスワード（既存で対応可能）
   - 電話認証の案内テキスト追加
   - デモ環境 / 本番環境 切り替えオプション
   - 認証エラーメッセージ（コード 100xx 系）

2. **`src/screen/dashboard.rs`**
   - 取引所セレクタを削除（立花証券固定）
   - 銘柄コード入力に変更（4桁コード）
   - 銘柄名表示エリア追加（マスタから取得）
   - 市場時間外インジケータ（東証: 9:00-11:30, 12:30-15:30 JST）

3. **チャートパネル**
   - ヒートマップ（板ヒートマップ）: 板データは存在するが制約あり
     → MVP では削除、板ストリーミング確認後に復活を検討
   - フットプリント: ティックデータ依存なため要検討
   - 日足チャート: メイン表示として維持
   - 比較チャート: 複数銘柄の日足比較（流用可能）

4. **`src/modal/stream_config.rs`（改修）**
   - 銘柄コード入力モーダルに変更
   - 銘柄名自動補完（マスタデータから）
   - 複数銘柄の監視リスト管理

5. **ウォッチリストパネル**（新規追加 or 流用）
   - 複数銘柄のスナップショット一覧
   - 騰落率・現在値・出来高を一覧表示

### フェーズ 5: 既存コードの削除・整理

削除対象:
- `exchange/src/adapter/binance.rs`
- `exchange/src/adapter/bybit.rs`
- `exchange/src/adapter/hyperliquid.rs`
- `exchange/src/adapter/mexc.rs`
- `exchange/src/adapter/okex.rs`
- `exchange/src/limiter.rs`（レート制限は立花用に再実装）
- `data/src/` の OpenInterest 関連
- `src/chart/heatmap.rs`（板ヒートマップ）→ MVP では削除、板ストリーム確認後に復活判断

維持・流用:
- `src/chart/kline.rs` → 日足チャートにそのまま使用
- `src/chart/comparison.rs` → 複数銘柄比較に流用
- `src/widget/` の描画ウィジェット群（そのまま流用）
- `data/src/config/` の設定管理（theme, layout, proxy）
- `src/audio.rs`（必要に応じて維持）

---

## 技術的注意点

### 1. 認証フロー
```
事前: 電話認証（ユーザーが手動で実施）
     ↓
アプリ起動 → ログイン画面
     ↓
POST /auth/?{"sCLMID":"CLMAuthLoginRequest","sUserId":"...","sPassword":"..."}
     ↓
応答から仮想URL群を取得・保存（セッションに保持）
     ↓
以降はすべて仮想URLでアクセス
```

### 2. WebSocket EVENT I/F
- 接続URL: `sUrlEventWebSocket`（ログイン毎に更新）
- 再接続時は再ログインが必要な場合あり
- 詳細プロトコルは「EVENT I/F 利用方法、データ仕様」を参照

### 3. 文字コード
- APIレスポンスに日本語が含まれる場合は Shift-JIS の可能性あり
- reqwest でデコード処理が必要（`encoding_rs` crate）

### 4. 市場時間
- 前場: 9:00 - 11:30 (JST)
- 後場: 12:30 - 15:30 (JST)
- 立会時間外はリアルタイムデータ更新なし

### 5. URL引数の JSON エスケープ
- URL の `?` 以降に JSON 文字列を直接渡す独自形式
- reqwest の通常クエリパラメータ設定では対応できない可能性あり
- 手動で URL 構築: `format!("{}?{}", virtual_url, serde_json::to_string(&req)?)`

### 6. レート制限
- 公開仕様なし → デモ環境で動作確認しながら適切な間隔を設定
- スナップショット取得は数秒間隔でポーリング推奨

---

## 実装優先順位

| 優先度 | 作業 | 理由 |
|--------|------|------|
| 高 | フェーズ1: API クライアント基盤 | 他すべての前提 |
| 高 | フェーズ3: 認証・コネクタ | 動作確認に必要 |
| 高 | フェーズ4: ログイン画面改修 | ユーザー入口 |
| 中 | フェーズ2: 日足データモデル | チャート表示に必要 |
| 中 | フェーズ4: ダッシュボード改修 | メイン画面 |
| 低 | フェーズ5: 旧コード削除 | 後からでも可 |
| 低 | ウォッチリスト | 便利機能 |

---

## 未確定事項（要調査）

1. **EVENT I/F の WebSocket プロトコル詳細**
   - サブプロトコル, メッセージ形式, 購読方法
   - 「EVENT I/F 利用方法、データ仕様」PDF を参照

2. **時価情報の情報コード一覧**
   - `pDPP`, `pDOP` 等のコード体系の完全版
   - 「FD」セクションの情報コード一覧

3. **電話認証のタイミング**
   - アプリ毎回起動時に必要か？ セッション維持方法？

4. **板情報の制約詳細**（未確定事項から「既知だが要確認」に変化）
   - 板データは存在する（公式: 「リアルタイム株価や板情報の取得が可能」）
   - REST スナップショット: `CLMMfdsGetMarketPrice` で板情報コード指定すれば取得可
   - リアルタイム: EVENT I/F 経由のボード配信（ストリーミング）で取得可能だが制限あり
   - **重要注意**: サンプル記載「FD 設定しても時価は配信されない」→ e支店APIではFDによるリアルタイム板配信が制限される可能性あり
   - `api_event_if_v4r7.pdf` で板の購読方法・制約を確認してからラダー復活を判断する

5. **デモ環境の利用時間帯**
   - デモで動作確認する際の制約を確認

---

## 参考リンク

- API リファレンス: https://www.e-shiten.jp/e_api/mfds_json_api_refference.html
- GitHub サンプルコード: https://github.com/e-shiten-jp
- 日足取得サンプル (Python): https://github.com/e-shiten-jp/e_api_get_histrical_price_daily.py
- 株価スナップショット取得: https://github.com/e-shiten-jp/e_api_get_price_from_file_tel.py

---

## 実装進捗サマリー (2026-04-09)

### 完了済み

| フェーズ | 作業 | ファイル | 状態 |
|----------|------|----------|------|
| 0 | Exchange/Venue enum の調査 | exchange/src/adapter.rs | ✅ |
| 1 | 認証型: LoginRequest/Response | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | セッション型: TachibanaSession | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | URL構築ユーティリティ | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | 時価情報型: MarketPrice* | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | 日足履歴型: DailyHistory* | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | HTTP クライアント関数 (login/fetch) | exchange/src/adapter/tachibana.rs | ✅ |
| 1 | TDD テスト (50テスト) | exchange/src/adapter/tachibana.rs | ✅ |
| 2 | Exchange::Tachibana / Venue::Tachibana 追加 | exchange/src/adapter.rs | ✅ |
| 2 | connect.rs に Tachibana スタブ | exchange/src/connect.rs | ✅ |
| 2 | style.rs venue_icon 対応 | src/style.rs | ✅ |
| 2 | tickers_table.rs available_markets 対応 | src/screen/dashboard/tickers_table.rs | ✅ |
| 2 | DailyHistoryRecord → Kline 変換 (daily_record_to_kline) | exchange/src/adapter/tachibana.rs | ✅ |
| 3 | auth.rs: セッション管理（store/get/clear）+ perform_login | src/connector/auth.rs | ✅ |
| 3 | main.rs: LoginSubmit → 非同期ログイン → LoginCompleted | src/main.rs | ✅ |
| 3 | fetcher.rs: Tachibana 日足取得パス (fetch_tachibana_daily_klines) | src/connector/fetcher.rs | ✅ |
| 4 | login.rs 改修: ユーザーID入力・デモ/本番切替・エラー表示・電話認証案内 | src/screen/login.rs | ✅ |

**テスト総数**: 74テスト PASS（全 workspace: flowsurface 20 + exchange 50 + 統合テスト 1 + data 3）

### 未完了（次の優先作業）

| 優先度 | 作業 | フェーズ |
|--------|------|----------|
| 中 | Ticker 型を4桁銘柄コードに適応 | 2 |
| 中 | セッション切れ検出と自動再ログイン | 3 |
| 低 | EVENT I/F WebSocket ストリーム実装 | 3 |
| 低 | 旧取引所アダプター削除 | 5 |

### 設計上の重要な知見

1. **数値はすべて文字列**: API の全数値フィールドは文字列型（`"pDPP": "3250"`）。
   表示/計算時に `parse::<f64>().unwrap_or(0.0)` が必要。`"*"` は未取得を意味する。

2. **URL 形式が独自**: `reqwest::Client::get(url).query(&params)` は不可。
   `format!("{}?{}", base_url, serde_json::to_string(&req)?)` で手動構築。

3. **仮想URLはセッション毎に変わる**: ハードコードは不可。
   `TachibanaSession` を起動時に生成し、コネクタに渡す設計が必要。

4. **電話認証は事前に必要**: アプリ起動時にユーザーが手動で電話認証済みの前提。
   `sKinsyouhouMidokuFlg: "1"` の場合は仮想URLが空 → `UnreadNotices` エラーで案内。

5. **enum-map の Enum derive**: `Exchange` enum に新バリアント追加時は
   serialize されたデータ（設定ファイル等）の互換性に注意。

6. **テスト戦略**: Rust の TDD は `#[cfg(test)] mod tests` をファイル内に配置。
   HTTP クライアントテストは `mockito` crate（dev-dependency に追加済み）。

7. **Kline 変換の時刻処理**: `DailyHistoryRecord.date` は "YYYYMMDD" 形式。
   JST 深夜0時 (UTC-9h) に変換する。`2024-01-01 JST` = `2023-12-31 15:00:00 UTC` = `1704034800000 ms`。
   `date_str_to_epoch_ms()` 関数で実装済み（tachibana.rs 内）。

8. **MinTicksize for 日本株**: 日本株価格は整数円なので `MinTicksize::new(0)` (10^0 = 1円単位)。
   将来的に呼値の細かいルール（価格帯別tick）に対応が必要な場合はここを修正。

9. **login.rs の Message 追加**: `UserIdChanged(String)`, `IsDemoProd(bool)` を追加。
   main.rs の `LoginSubmit` ハンドラで `self.login_screen.user_id` と `self.login_screen.is_demo`
   を参照してAPIに渡す実装が次のステップ。
   `tachibana_error_message(code: &str)` で API エラーコードを日本語メッセージに変換できる。

10. **セッション管理アーキテクチャ（Phase 3 で確立）**:
    - `static RwLock<Option<TachibanaSession>>` でグローバルにセッションを保持
    - `auth::store_session()` / `auth::get_session()` でアクセス
    - ログイン成功時に `store_session` + `persist_session`（keyring永続化）→ fetcher.rs から `get_session` で参照
    - 起動時に `try_restore_session()` で keyring → `validate_session()` → ログイン画面スキップ
    - `validate_session` は許可リスト方式（`p_errno` が `"0"` / `""` のみOk、未知コードはErr）
    - `Ticker::as_str()` は private → `to_full_symbol_and_type()` で銘柄コードを取得
    - `perform_login()` は `Result<TachibanaSession, String>` を返す（Iced の `Task::perform` 互換）

11. **main.rs の LoginSubmit → LoginCompleted パターン**:
    - `LoginSubmit` で `Task::perform(auth::perform_login(...), Message::LoginCompleted)` を発行
    - `LoginCompleted(Ok(session))` → セッション保存 → メインウィンドウ起動
    - `LoginCompleted(Err(msg))` → `login_screen.set_error(Some(msg))` でエラー表示
    - `TachibanaError` → ユーザー向けメッセージ変換は `tachibana_error_to_message()` で実施

12. **fetcher.rs の Tachibana 分岐**:
    - `kline_fetch_task` 内で `Venue::Tachibana` を判定して専用パスに分岐
    - `fetch_tachibana_daily_klines()` が `auth::get_session()` → `fetch_daily_history()` → `daily_record_to_kline()` の一連を実行
    - 調整値（`*xK` フィールド）をデフォルトで使用（`use_adjusted: true`）
