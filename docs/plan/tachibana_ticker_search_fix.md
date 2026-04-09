# 立花証券 銘柄検索が表示されない問題 — 修正プラン

**作成日**: 2026-04-09
**対象**: "Search for a ticker" モーダルで Tachibana の銘柄が0件

---

## 1. 原因分析

### 根本原因

`fetch_ticker_metadata(Venue::Tachibana, ...)` が **空の HashMap を返している**。

```rust
// exchange/src/adapter.rs:771-772
// 立花証券は CLMIssueMstKabu で銘柄情報を取得（TODO: Phase 3 で実装）
Venue::Tachibana => Ok(HashMap::default()),
```

同様に `fetch_ticker_stats` も空を返している:

```rust
// exchange/src/adapter.rs:809-810
// 立花証券は現在値を CLMMfdsGetMarketPrice で取得（TODO: Phase 3 で実装）
Venue::Tachibana => Ok(HashMap::default()),
```

### データフロー（なぜ表示されないか）

```
TickersTable::new_with_settings()
  └─ fetch_ticker_metadata(Venue::Tachibana, &[Spot]) 
       └─ 空の HashMap を返す                          ← ★ ここが原因
            └─ tickers_info に Tachibana 銘柄が0件
                 └─ update_ticker_rows() でフィルタ:
                      tickers_info.contains_key(t) → false
                       └─ ticker_rows[] が空
                            └─ filtered_rows() → 0件
                                 └─ 検索結果に何も出ない
```

### 他取引所との比較

| 取引所 | fetch_ticker_metadata | fetch_ticker_stats | 状態 |
|--------|----------------------|-------------------|------|
| Binance | `/api/v3/exchangeInfo` → TickerInfo | `/api/v3/ticker/24hr` → TickerStats | **実装済** |
| Bybit | `/v5/market/instruments-info` → TickerInfo | `/v5/market/tickers` → TickerStats | **実装済** |
| Hyperliquid | meta API → TickerInfo | meta API → TickerStats | **実装済** |
| Tachibana | `HashMap::default()` (空) | `HashMap::default()` (空) | **未実装** |

---

## 2. 立花証券 API で銘柄リストを取得する方法

### CLMIssueMstKabu（株式銘柄マスタ）

立花証券の銘柄マスタは **EVENT I/F（ストリーミング）** 経由で取得する。REST API では取得不可。

#### 取得フロー

```
1. ログイン → TachibanaSession を取得
2. EVENT I/F に CLMEventDownload リクエストを送信:
   {
     "sCLMID": "CLMEventDownload",
     "sTargetCLMID": "CLMIssueMstKabu,CLMEventDownloadComplete"
   }
3. サーバーから CLMIssueMstKabu レコードが配信される（全銘柄分）
4. CLMEventDownloadComplete を受信したら完了 → 切断
```

**重要**: このリクエストは「配信要求」であり、通常の REST のような直接応答は返さない。
EVENT I/F（HTTP Long-polling または WebSocket）で受信する必要がある。

#### CLMIssueMstKabu レコードの形式

```json
{
  "sCLMID": "CLMIssueMstKabu",
  "sIssueCode": "1301",
  "sIssueName": "極 洋",
  "sIssueNameRyaku": "極洋",
  "sIssueNameKana": "キヨクヨウ",
  "sIssueNameEizi": "KYOKUYO",
  "sBaibaiTani": "100",
  "sGyousyuCode": "0050",
  "sGyousyuName": "水産・農林業",
  "sYusenSizyou": "00",
  "sBaibaiTeisiC": "",
  ...
}
```

#### 銘柄検索に必要なフィールド

| API フィールド | 用途 | マッピング先 |
|--------------|------|-------------|
| `sIssueCode` | 銘柄コード (例: "6501") | `Ticker::new(code, Exchange::Tachibana)` |
| `sIssueName` | 銘柄名称 (例: "日 立 製 作 所") | display_symbol / 検索対象 |
| `sIssueNameRyaku` | 略称 (例: "日立") | 検索対象 |
| `sIssueNameKana` | カナ名 (例: "ヒタチセイサクシヨ") | 検索対象 |
| `sIssueNameEizi` | 英語名 (例: "HITACHI") | 検索対象 |
| `sBaibaiTani` | 売買単位 (例: "100") | `TickerInfo.min_qty` |
| `sYusenSizyou` | 優先市場 (例: "00" = 東証) | フィルタ |
| `sBaibaiTeisiC` | 売買停止フラグ | フィルタ（停止中は除外） |

---

## 3. 課題: EVENT I/F が未実装

`CLMIssueMstKabu` を取得するには EVENT I/F 接続が必要だが、現在の実装には存在しない。

既存の EVENT I/F 関連コード:
- `exchange/src/connect.rs` にスタブのみ（`futures::stream::empty()` を返す）
- レビュー報告書 Section 3 に EVENT I/F のプロトコル詳細が記載済み

EVENT I/F の完全実装（WebSocket リアルタイムデータ）は Phase 3 の大きなスコープだが、
**銘柄マスタの1回取得だけなら HTTP Long-polling で十分**。

---

## 4. 修正方針

### 方針A: EVENT I/F 経由で CLMIssueMstKabu を取得する（推奨）

HTTP Long-polling を使って銘柄マスタを1回取得する最小実装。
WebSocket のフル実装は不要。

#### 実装ステップ

**Step 1: マスタダウンロード関数の追加** (`exchange/src/adapter/tachibana.rs`)

```rust
/// CLMEventDownload リクエスト（マスタ情報ダウンロード要求）
#[derive(Debug, Serialize)]
pub struct EventDownloadRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,           // "CLMEventDownload"
    #[serde(rename = "sTargetCLMID")]
    pub target_clm_id: String,           // "CLMIssueMstKabu,CLMEventDownloadComplete"
}

/// CLMIssueMstKabu レコード（株式銘柄マスタ）
#[derive(Debug, Deserialize, Clone)]
pub struct IssueMstKabuRecord {
    #[serde(rename = "sCLMID")]
    pub clm_id: String,
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,
    #[serde(rename = "sIssueName", default)]
    pub issue_name: String,
    #[serde(rename = "sIssueNameRyaku", default)]
    pub issue_name_short: String,
    #[serde(rename = "sIssueNameKana", default)]
    pub issue_name_kana: String,
    #[serde(rename = "sIssueNameEizi", default)]
    pub issue_name_english: String,
    #[serde(rename = "sBaibaiTani", default)]
    pub trading_unit: String,            // 売買単位
    #[serde(rename = "sYusenSizyou", default)]
    pub primary_market: String,          // 優先市場
    #[serde(rename = "sBaibaiTeisiC", default)]
    pub trading_halt_code: String,       // 売買停止フラグ
    #[serde(rename = "sGyousyuCode", default)]
    pub sector_code: String,
    #[serde(rename = "sGyousyuName", default)]
    pub sector_name: String,
}

/// EVENT I/F (HTTP Long-polling) で銘柄マスタをダウンロードする。
/// CLMEventDownloadComplete を受信するまでストリーミングで読み取る。
pub async fn fetch_issue_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<Vec<IssueMstKabuRecord>, TachibanaError> {
    // 1. CLMEventDownload リクエストを session.url_event に POST
    // 2. HTTP streaming (iter_lines) で受信
    // 3. 各行を JSON パース → sCLMID で振り分け
    // 4. "CLMIssueMstKabu" → Vec に蓄積
    // 5. "CLMEventDownloadComplete" → 終了
    todo!()
}
```

**Step 2: TickerInfo / TickerStats への変換** (`exchange/src/adapter/tachibana.rs`)

```rust
/// IssueMstKabuRecord → (Ticker, Option<TickerInfo>) に変換
pub fn issue_record_to_ticker_info(record: &IssueMstKabuRecord) -> Option<(Ticker, Option<TickerInfo>)> {
    // 売買停止中("9")の銘柄は除外
    if record.trading_halt_code == "9" {
        return None;
    }

    let display = if record.issue_name_short.is_empty() {
        None
    } else {
        Some(record.issue_name_short.as_str())
    };

    let ticker = Ticker::new_with_display(
        &record.issue_code,
        Exchange::Tachibana,
        display,
    );

    let trading_unit: f32 = record.trading_unit.parse().unwrap_or(100.0);

    // 日本株は min_ticksize = 1円 (整数)、呼値の刻みは価格帯で異なるが暫定1.0
    let info = TickerInfo::new(
        ticker,
        1.0,            // min_ticksize (暫定: 呼値テーブル CLMYobine で正確化可能)
        trading_unit,   // min_qty = 売買単位
        None,           // contract_size (現物なので不要)
    );

    Some((ticker, Some(info)))
}
```

**Step 3: fetch_ticker_metadata の接続** (`exchange/src/adapter.rs`)

```rust
// adapter.rs:771-772 を変更
Venue::Tachibana => {
    // セッションが必要 → auth.rs から取得
    // セッションがない場合は空を返す（未ログイン時）
    match crate::connector::auth::get_session() {
        Some(session) => {
            let client = reqwest::Client::new();
            let records = tachibana::fetch_issue_master(&client, &session).await
                .map_err(|e| AdapterError::FetchError(e.to_string()))?;
            let mut out = HashMap::new();
            for record in &records {
                if let Some((ticker, info)) = tachibana::issue_record_to_ticker_info(record) {
                    out.insert(ticker, info);
                }
            }
            Ok(out)
        }
        None => Ok(HashMap::default()),  // 未ログイン時は空
    }
}
```

**Step 4: fetch_ticker_stats の接続** (`exchange/src/adapter.rs`)

```rust
// adapter.rs:809-810 を変更
Venue::Tachibana => {
    // CLMMfdsGetMarketPrice で現在値を取得
    // tickers_info のキーから銘柄コードリストを生成
    // → fetch_market_prices() で一括取得（最大120銘柄ずつ）
    // → TickerStats に変換
    match crate::connector::auth::get_session() {
        Some(session) => {
            // 既存の fetch_market_prices() を活用
            todo!("銘柄コードリストを受け取り、TickerStats に変換")
        }
        None => Ok(HashMap::default()),
    }
}
```

**Step 5: 銘柄検索の日本語対応** (`src/screen/dashboard/tickers_table.rs`)

現在の検索ランキング `calc_search_rank()` は英語ティッカーシンボル（"BTCUSDT" 等）前提。
日本株の場合は以下も検索対象にする必要がある:
- 銘柄コード（"6501"）
- 銘柄名（"日立"）
- カナ名（"ヒタチ"）
- 英語名（"HITACHI"）

→ `Ticker::display_symbol()` に銘柄名略称を入れることで、既存の検索ロジックで
  銘柄コードと略称の両方がヒットするようになる。
  カナ・英語名は TickerInfo に追加フィールドが必要か、別の検索インデックスが必要。

---

### 方針B: 静的銘柄リスト（ハードコード/ファイル埋め込み）— 暫定対応

EVENT I/F 実装の前に、最小限の動作確認として銘柄リストをハードコードする案。

- 東証上場の主要銘柄（日経225構成銘柄 etc.）をコード内に埋め込む
- `fetch_ticker_metadata` で静的リストから `HashMap<Ticker, Option<TickerInfo>>` を生成
- ログイン不要で動作確認可能

**メリット**: 実装が速い、EVENT I/F 不要
**デメリット**: 銘柄の追加/廃止に追従できない、売買停止等のステータスが反映されない

---

## 5. 認証の課題

`fetch_ticker_metadata` は現在 `(Venue, &[MarketKind])` のみを引数に取り、
セッション情報を受け取る仕組みがない。

```rust
// 現在のシグネチャ
pub async fn fetch_ticker_metadata(
    venue: Venue,
    markets: &[MarketKind],
) -> Result<HashMap<Ticker, Option<TickerInfo>>, AdapterError>
```

立花証券はセッション認証が必要なため、以下のいずれかの対応が必要:

| 案 | 内容 | 影響範囲 |
|----|------|---------|
| A. グローバルセッション参照 | `auth.rs` の `get_session()` を内部で呼ぶ | tachibana 分岐のみ |
| B. シグネチャ変更 | 引数に `Option<Session>` を追加 | 全 Venue の呼び出し元 |
| C. Venue trait 化 | 各 Venue を trait impl で分離 | 大規模リファクタ |

**推奨: 案A**（最小影響。既に `auth.rs` にグローバル `static SESSION` がある）

---

## 6. 実装順序（推奨）

```
Phase 2.5（本修正）
│
├── Step 1: IssueMstKabuRecord 型定義              ← 型のみ、テスト可
├── Step 2: issue_record_to_ticker_info() 変換     ← ユニットテスト
├── Step 3: fetch_issue_master() HTTP streaming    ← mockito テスト + 本番テスト
├── Step 4: adapter.rs の fetch_ticker_metadata 接続
├── Step 5: adapter.rs の fetch_ticker_stats 接続
│           (既存の fetch_market_prices を活用)
└── Step 6: 銘柄名での検索対応（display_symbol 活用）
```

### テスト計画

| Step | テスト内容 | 種類 |
|------|----------|------|
| 1 | IssueMstKabuRecord の JSON デシリアライズ | ユニット |
| 2 | issue_record_to_ticker_info の変換・フィルタ | ユニット |
| 3 | fetch_issue_master の HTTP streaming パース | mockito 統合 |
| 3 | CLMEventDownloadComplete での終了判定 | mockito 統合 |
| 4 | fetch_ticker_metadata(Venue::Tachibana) が非空を返す | 統合 |
| 5 | fetch_ticker_stats(Venue::Tachibana) が TickerStats を返す | 統合 |
| 6 | 銘柄コード "6501" / 銘柄名 "日立" で検索ヒット | UI テスト |

---

## 7. 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `exchange/src/adapter/tachibana.rs` | IssueMstKabuRecord 型、fetch_issue_master()、issue_record_to_ticker_info() |
| `exchange/src/adapter.rs` | Venue::Tachibana の fetch_ticker_metadata / fetch_ticker_stats を接続 |
| `src/connector/auth.rs` | get_session() は既存のまま使用（変更なし） |
| `src/screen/dashboard/tickers_table.rs` | （必要に応じて）日本語検索対応 |

---

## 8. リスクと未確定事項

1. **EVENT I/F の HTTP Long-polling 実装詳細**
   - レビュー報告書 Section 3.4 にサンプルコードあり（`requests.session().get(url, stream=True).iter_lines()`）
   - Rust では `reqwest::Response::bytes_stream()` + `tokio::io::AsyncBufReadExt::lines()` で対応可能
   - ただし実際のデータ形式（JSON lines か SOH/STX 区切りか）は本番テストで要確認

2. **マスタダウンロードの所要時間**
   - 全銘柄（約4,000件）のダウンロードに要する時間が不明
   - 起動時に同期的に待つとUXが悪化する可能性
   - → バックグラウンドでダウンロードし、完了後にティッカーリストを更新する非同期設計が望ましい

3. **セッション未確立時の挙動**
   - ログイン前は銘柄リストが空になる
   - ログイン後にティッカーリストのリフレッシュが必要
   - 他取引所はログイン不要で銘柄取得できるが、立花証券はセッション必須

4. **呼値テーブル（CLMYobine）**
   - 日本株の min_ticksize は価格帯によって異なる（例: 3000円以下は1円、5000円以下は5円）
   - 暫定で 1.0 固定とするが、正確にするには CLMYobine マスタも取得が必要
   - → Phase 3 以降で対応可
