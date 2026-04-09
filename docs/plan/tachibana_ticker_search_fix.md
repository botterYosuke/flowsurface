# 立花証券 銘柄検索が表示されない問題 — 修正プラン

**作成日**: 2026-04-09
**更新日**: 2026-04-09 (v3: サンプルコード検証後の修正)
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

## 2. 立花証券 API で銘柄マスタを取得する方法

### CLMIssueMstKabu（株式銘柄マスタ）

銘柄マスタは **MASTER I/F（`sUrlMaster`）** 経由のストリーミングダウンロードで取得する。
EVENT I/F（`sUrlEvent`）ではない。

> **参照**: `docs/e-shiten/samples/e_api_get_master_tel.py/`

#### 取得フロー（サンプルコード準拠）

```
1. ログイン → TachibanaSession を取得（url_master を含む）
2. MASTER I/F に HTTP GET リクエストを送信:
   URL: {session.url_master}?{json_params}
   パラメータ:
     - p_no: リクエスト番号
     - p_sd_date: タイムスタンプ (YYYY.MM.DD-hh:mm:ss.sss)
     - sCLMID: "CLMEventDownload"
     - sJsonOfmt: "4"        ← JSON形式指定（ファイル保存用フォーマット）
3. HTTP ストリーミング応答で全マスタレコードが配信される
   - 各レコードは JSON オブジェクト（`}` で1レコードの終わりを判定）
   - エンコーディング: Shift-JIS
   - 全マスタ一括（CLMIssueMstKabu 以外も含む。約21MB）
4. sCLMID == "CLMEventDownloadComplete" のレコードを受信したら完了
```

**注意点**:
- `sTargetCLMID` による選択的ダウンロードはサンプルで使われていない（全マスタ一括）
- HTTP GET（POST ではない）。URL にクエリパラメータとして JSON を付与
- 応答データは **Shift-JIS** エンコード
- データサイズが **約21MB** と大きい（全マスタ種類を含む）

#### CLMIssueMstKabu レコードの形式

```json
{
  "sCLMID": "CLMIssueMstKabu",
  "sIssueCode": "1301",
  "sIssueName": "極 洋",
  "sIssueNameRyaku": "極洋",
  "sIssueNameKana": "キヨクヨウ",
  "sIssueNameEizi": "KYOKUYO",
  "sGyousyuCode": "0050",
  "sGyousyuName": "水産・農林業",
  "sYusenSizyou": "00",
  ...
}
```

#### 利用可能なフィールド（readme.txt より）

> 株式 銘柄マスタ（CLMIssueMstKabu）では、**銘柄コード、銘柄名、銘柄名略称、銘柄名（カナ）、銘柄名（英語表記）、優先市場、業種コード、業種コード名** のみ利用できます。
> その他のデータについてはｅ支店サポートセンターにご確認ください。

| API フィールド | 用途 | 利用可否 |
|--------------|------|:-------:|
| `sIssueCode` | 銘柄コード (例: "6501") | Yes |
| `sIssueName` | 銘柄名称 (例: "日 立 製 作 所") | Yes |
| `sIssueNameRyaku` | 略称 (例: "日立") | Yes |
| `sIssueNameKana` | カナ名 (例: "ヒタチセイサクシヨ") | Yes |
| `sIssueNameEizi` | 英語名 (例: "HITACHI") | Yes |
| `sYusenSizyou` | 優先市場 (例: "00" = 東証) | Yes |
| `sGyousyuCode` | 業種コード | Yes |
| `sGyousyuName` | 業種コード名 | Yes |
| `sBaibaiTani` | 売買単位 | **不確実** (readme に記載なし) |
| `sBaibaiTeisiC` | 売買停止フラグ | **不確実** (readme に記載なし) |

#### EVENT I/F との違い

| | MASTER I/F (マスタダウンロード) | EVENT I/F (リアルタイム通知) |
|---|---|---|
| URL | `session.url_master` | `session.url_event` |
| HTTP メソッド | GET | GET |
| データ形式 | JSON オブジェクト（Shift-JIS） | SOH(^A)/STX(^B)/ETX(^C) 区切りバイナリ |
| 用途 | 銘柄マスタ等の一括ダウンロード | 株価・約定のリアルタイムプッシュ |
| 頻度 | 1日1回 | 常時接続 |

---

## 3. 修正方針

MASTER I/F (HTTP GET ストリーミング) で銘柄マスタを1回取得する最小実装。

### スコープ

本修正のゴールは **「銘柄が検索モーダルに表示される」** こと。

| 項目 | 本修正に含む | 理由 |
|------|:-----------:|------|
| `fetch_ticker_metadata` の実装 | Yes | これが空なのが根本原因 |
| `fetch_ticker_stats` の実装 | **No** | 補足情報（24h変動率等）であり、空でも検索・選択は機能する |
| 日本語名・カナ名での検索 | **No** | 銘柄コードで検索できれば最低限動作する。UX改善は別タスク |

### キャッシュ戦略

全マスタダウンロード（約21MB）は毎回実行するのではなく、以下の方針とする:

- **ログイン成功時に1回だけ取得**（バックグラウンド）し、メモリに保持
- `fetch_ticker_metadata` はキャッシュから返す
- 銘柄マスタは1日1回の更新で十分（上場・廃止は頻繁ではない）
- ダウンロード中は銘柄リストが空（ローディング状態）になる

---

## 4. 実装ステップ

### Step 1: IssueMstKabuRecord 型定義 (`exchange/src/adapter/tachibana.rs`)

```rust
/// 全マスタダウンロードの各レコードをパースするための汎用型。
/// sCLMID でレコード種別を判定し、CLMIssueMstKabu のみ抽出する。
#[derive(Debug, Deserialize, Clone)]
pub struct MasterRecord {
    #[serde(rename = "sCLMID")]
    pub clm_id: String,
    // CLMIssueMstKabu 固有フィールド（他マスタでは空/未設定）
    #[serde(rename = "sIssueCode", default)]
    pub issue_code: String,
    #[serde(rename = "sIssueName", default)]
    pub issue_name: String,
    #[serde(rename = "sIssueNameRyaku", default)]
    pub issue_name_short: String,
    #[serde(rename = "sIssueNameKana", default)]
    pub issue_name_kana: String,
    #[serde(rename = "sIssueNameEizi", default)]
    pub issue_name_english: String,
    #[serde(rename = "sYusenSizyou", default)]
    pub primary_market: String,
    #[serde(rename = "sGyousyuCode", default)]
    pub sector_code: String,
    #[serde(rename = "sGyousyuName", default)]
    pub sector_name: String,
}
```

注意: `sBaibaiTani`(売買単位)と `sBaibaiTeisiC`(売買停止)は readme で利用可能と明記されていないため、
型定義に含めない。`min_qty` は日本株のデフォルト値 100 を使用する。

### Step 2: TickerInfo への変換 (`exchange/src/adapter/tachibana.rs`)

```rust
/// MasterRecord (CLMIssueMstKabu) → (Ticker, TickerInfo) に変換。
pub fn master_record_to_ticker_info(record: &MasterRecord) -> Option<(Ticker, TickerInfo)> {
    // CLMIssueMstKabu 以外のレコードは無視
    if record.clm_id != "CLMIssueMstKabu" {
        return None;
    }
    // 銘柄コードが空なら無視
    if record.issue_code.is_empty() {
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

    let info = TickerInfo::new(
        ticker,
        1.0,    // min_ticksize (暫定: 呼値テーブルで正確化可能)
        100.0,  // min_qty = 日本株デフォルト売買単位
        None,   // contract_size (現物なので不要)
    );

    Some((ticker, info))
}
```

### Step 3: fetch_all_master() HTTP GET ストリーミング (`exchange/src/adapter/tachibana.rs`)

```rust
/// MASTER I/F で全マスタを一括ダウンロードする。
/// CLMEventDownloadComplete を受信するまでストリーミングで読み取り、
/// CLMIssueMstKabu レコードのみを抽出して返す。
pub async fn fetch_all_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<Vec<MasterRecord>, TachibanaError> {
    // 1. URL構築: session.url_master に JSON クエリパラメータを付与
    //    パラメータ: p_no, p_sd_date, sCLMID="CLMEventDownload", sJsonOfmt="4"
    // 2. HTTP GET (stream=true) で session.url_master に接続
    // 3. バイトストリームを受信し、'}' でレコード境界を判定
    // 4. 各レコードを Shift-JIS → UTF-8 にデコードし JSON パース
    // 5. sCLMID == "CLMIssueMstKabu" のレコードのみ Vec に蓄積
    // 6. sCLMID == "CLMEventDownloadComplete" で終了
    todo!()
}
```

**実装上の要点** (サンプルコード `e_api_get_master_tel.py:461-528` 準拠):
- `urllib3.PoolManager().request('GET', url, preload_content=False)` → Rust では `reqwest::Client::get(url).send().await` + `response.bytes_stream()`
- レコード境界の判定: バイトを蓄積し、末尾が `b'}'` なら1レコード完了
- Shift-JIS デコード: `encoding_rs` crate を使用 (`encoding_rs::SHIFT_JIS.decode()`)
- 約21MBのデータを全てメモリに保持するのではなく、`CLMIssueMstKabu` のみフィルタして蓄積

### Step 4: マスタキャッシュの導入 (`exchange/src/adapter/tachibana.rs`)

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

static ISSUE_MASTER_CACHE: RwLock<Option<Arc<Vec<MasterRecord>>>> = RwLock::const_new(None);

/// ログイン成功時に呼び出し、銘柄マスタをキャッシュに格納する
pub async fn init_issue_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(), TachibanaError> {
    let records = fetch_all_master(client, session).await?;
    *ISSUE_MASTER_CACHE.write().await = Some(Arc::new(records));
    Ok(())
}

/// キャッシュ済みの銘柄マスタを返す。未取得なら None。
pub async fn get_cached_issue_master() -> Option<Arc<Vec<MasterRecord>>> {
    ISSUE_MASTER_CACHE.read().await.clone()
}
```

`RwLock` を使用（`OnceLock` ではなく）。再ログイン時にキャッシュを更新可能。

### Step 5: adapter.rs の fetch_ticker_metadata 接続 (`exchange/src/adapter.rs`)

```rust
// adapter.rs:771-772 を変更
Venue::Tachibana => {
    let mut out = HashMap::new();
    if let Some(records) = tachibana::get_cached_issue_master().await {
        for record in records.iter() {
            if let Some((ticker, info)) = tachibana::master_record_to_ticker_info(record) {
                out.insert(ticker, Some(info));
            }
        }
    }
    Ok(out)
}
```

セッション不要（キャッシュから読むだけ）。未ログイン/ダウンロード中は空の HashMap を返す。

---

## 5. 認証の課題

`fetch_ticker_metadata` のシグネチャ変更は不要。
キャッシュ導入により、`fetch_ticker_metadata` 内でセッション参照する必要がなくなった。

`init_issue_master()` はログイン処理の中で呼び出す:
- `src/connector/auth.rs` のログイン成功後に `tachibana::init_issue_master()` を spawn
- バックグラウンドで実行し、完了後に銘柄リストが利用可能になる
- これにより既存の関数シグネチャへの影響はゼロ

---

## 6. 実装順序

```
Phase 2.5（本修正）
│
├── Step 1: MasterRecord 型定義                    ← 型のみ、テスト可
├── Step 2: master_record_to_ticker_info() 変換    ← ユニットテスト
├── Step 3: fetch_all_master() HTTP GET streaming  ← mockito テスト + 本番テスト
├── Step 4: マスタキャッシュ (RwLock)               ← init_issue_master / get_cached_issue_master
└── Step 5: adapter.rs の fetch_ticker_metadata 接続
```

### テスト計画

| Step | テスト内容 | 種類 |
|------|----------|------|
| 1 | MasterRecord の JSON デシリアライズ（Shift-JIS デコード後） | ユニット |
| 2 | master_record_to_ticker_info: CLMIssueMstKabu → TickerInfo 変換 | ユニット |
| 2 | master_record_to_ticker_info: CLMIssueMstKabu 以外は None | ユニット |
| 3 | fetch_all_master: `}` 区切りのストリーミングパース | mockito 統合 |
| 3 | fetch_all_master: CLMEventDownloadComplete での終了判定 | mockito 統合 |
| 3 | fetch_all_master: Shift-JIS エンコードされた日本語の正常デコード | mockito 統合 |
| 5 | fetch_ticker_metadata(Venue::Tachibana) がキャッシュから非空を返す | 統合 |

---

## 7. 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `exchange/src/adapter/tachibana.rs` | MasterRecord 型、fetch_all_master()、master_record_to_ticker_info()、キャッシュ |
| `exchange/src/adapter.rs` | Venue::Tachibana の fetch_ticker_metadata をキャッシュ参照に変更 |
| `src/connector/auth.rs` | ログイン成功後に `init_issue_master()` を spawn（1行追加） |
| `Cargo.toml` (exchange) | `encoding_rs` 依存追加（Shift-JIS デコード用） |

---

## 8. スコープ外（後続タスク）

| 項目 | 理由 | 時期 |
|------|------|------|
| `fetch_ticker_stats` の実装 | 全銘柄の現在値取得は多数のAPI呼び出しが必要。遅延取得等の設計が別途必要 | Phase 3 |
| 日本語名・カナ名での検索 | 銘柄コードで検索可能。UX改善は別タスク | Phase 3 |
| 呼値テーブル (CLMYobine) の取得 | min_ticksize を価格帯別に正確化。暫定1.0で動作する | Phase 3 |
| 売買単位 (sBaibaiTani) の正確な取得 | readme で利用可能と明記されていない。暫定100で動作する | Phase 3 |
| EVENT I/F のフル実装 (WebSocket) | リアルタイム板情報・約定データの配信。本修正とは別プロトコル | Phase 3 |

---

## 9. リスクと未確定事項

1. **全マスタ一括ダウンロードのサイズ（約21MB）**
   - CLMIssueMstKabu 以外のマスタ（日付情報、呼値テーブル等）も全て含まれる
   - ストリーミング中に `sCLMID` でフィルタし、不要レコードは破棄する
   - それでもダウンロード自体に時間がかかる可能性あり → バックグラウンド実行必須

2. **Shift-JIS エンコーディング**
   - ストリームのバイトデータは Shift-JIS（サンプル: `byte_data.decode('shift-jis')`）
   - `encoding_rs::SHIFT_JIS` で UTF-8 に変換が必要
   - Shift-JIS の2バイト文字が `}` (0x7D) を含むケースは稀だが、レコード境界判定で注意

3. **URL構築形式**
   - サンプルでは `url_master?{json_body}` の形式（JSON をクエリパラメータとして付与）
   - 既存の `post_request()` は POST 用なので、GET ストリーミング用の新関数が必要

4. **sJsonOfmt パラメータ**
   - サンプルでは `"4"` を指定（ファイル保存用フォーマット）
   - 他の値だとレスポンス形式が異なる可能性がある → サンプルに倣い `"4"` 固定

5. **sTargetCLMID による選択的ダウンロード**
   - サンプルでは使っていないが、API 仕様上は存在する可能性がある
   - 使えれば CLMIssueMstKabu のみダウンロードでき、21MB → 大幅に削減できる
   - → まずサンプル通りの全マスタダウンロードで実装し、後から最適化

## 10. 参照資料

| 資料 | パス |
|------|------|
| マスタダウンロード サンプル | `docs/e-shiten/samples/e_api_get_master_tel.py/` |
| EVENT I/F サンプル | `docs/e-shiten/samples/e_api_event_receive_tel.py/` |
| マスタ読み出し サンプル | `docs/e-shiten/samples/e_api_get_master_tel.py/read_master.py` |
