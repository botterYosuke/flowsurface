# 立花証券 銘柄検索が表示されない問題 — 修正プラン

**作成日**: 2026-04-09
**更新日**: 2026-04-10 (v6: 起動時再ログイン仕様変更)
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

- **ログイン成功時に1回だけ取得**（`await` で同期的に完了を待つ）し、メモリに保持
- `fetch_ticker_metadata` はキャッシュから返す
- 銘柄マスタは1日1回の更新で十分（上場・廃止は頻繁ではない）
- ダッシュボード初期化時に `fetch_ticker_metadata` が参照するため、`spawn` ではなく `await` で完了を待つ必要がある

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

/// ログイン成功時に呼び出し、銘柄マスタをキャッシュに格納する。
/// ダッシュボード初期化前に完了している必要があるため、spawn ではなく await で呼び出す。
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
- `src/connector/auth.rs` のログイン成功後に `tachibana::init_issue_master()` を **await** で同期待ち
- ダッシュボード初期化時の `fetch_ticker_metadata` がキャッシュを参照するため、完了を待つ必要がある
- これにより既存の関数シグネチャへの影響はゼロ

---

## 6. 実装順序

```
Phase 2.5（本修正）
│
├── ✅ Step 1: MasterRecord 型定義
├── ✅ Step 2: master_record_to_ticker_info() 変換
├── ✅ Step 3: fetch_all_master() HTTP GET streaming
├── ✅ Step 4: マスタキャッシュ (RwLock)
└── ✅ Step 5: adapter.rs + auth.rs 接続
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
| `src/connector/auth.rs` | ログイン成功後に `init_issue_master()` を await（1行追加） |
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
   - ダウンロードに数秒〜十数秒かかる可能性があり、ログイン時の `await` 待ちでユーザーに遅延が見える
   - 現状はダッシュボード初期化前にキャッシュ完了が必要なため `await` を採用（Section 3 参照）
   - UX改善として、マスタ取得を `spawn` でバックグラウンド化し銘柄一覧を遅延更新する設計も考えられるが、スコープ外

2. **Shift-JIS エンコーディングと `}` (0x7D) 境界判定**
   - ストリームのバイトデータは Shift-JIS（サンプル: `byte_data.decode('shift-jis')`）
   - `encoding_rs::SHIFT_JIS` で UTF-8 に変換が必要
   - Shift-JIS の2バイト文字の第2バイト範囲は `0x40-0x7E` / `0x80-0xFC` であり、`}` (0x7D) は範囲内に含まれる
   - 日本語文字の一部が `}` バイトを含む場合、偽のレコード境界が検出される

   **Python サンプルとの比較** (`e_api_get_master_tel.py:494-501`):
   - Python: `byte_data[-1:] == b'}'` — 蓄積バッファの **末尾バイトのみ** を判定
   - Rust（現実装）: `if byte == b'}'` — ストリームの **全 `}` バイト** を個別に判定
   - Python は chunk 境界に `}` が来た場合のみ誤判定するが、Rust は全ての Shift-JIS `}` trail byte で誤判定する
   - したがって Rust 実装のほうが偽陽性の頻度が高い
   - なお、Python サンプルも `byte_data = b''` でバッファクリアしており、誤判定時のレコード消失リスクは同じ

   **現在の実装の問題**: パース失敗時に `buf.clear()` するため、偽境界で分割された前後の **2レコードが消失** する

   **対策（残タスク）**: パース失敗時にバッファをクリアせず次の `}` まで蓄積し続ける:
   ```rust
   Err(_) => {
       // buf.clear() しない → 次の '}' まで蓄積を続ける
       log::trace!("Partial record, continuing to next '}'");
   }
   ```

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

---

## 11. 実装メモ（2026-04-09）

### 実装完了

全5ステップを実装し、`cargo check` 通過、既存テスト全45件パス。

### プランからの変更点

1. **display_symbol に英語名を使用**: プランでは `sIssueNameRyaku`（日本語略称）を display_symbol に使う設計だったが、`Ticker::new_with_display()` は `assert!(display.is_ascii())` を持つため日本語は不可。代わりに `sIssueNameEizi`（英語名）を使用。28文字超はバイト位置で切り捨て（英語名は ASCII 前提のため安全だが、非 ASCII 混入時は `display = None` にフォールバックすべき — 後続タスクとして対応可）。

2. **`spawn` ではなく `await` で同期待ち**: ダッシュボード初期化時に `fetch_ticker_metadata` がキャッシュを参照するため、ログイン完了前にマスタダウンロードを完了させる必要がある。`spawn_init_issue_master()` は exchange crate に存在するが**未使用のデッドコード**であり、削除対象。

3. **`encoding_rs` は追加不要**: 既に `exchange/Cargo.toml` に含まれていた（既存の `decode_response_body` が使用）。

4. **reqwest に `stream` feature を追加**: `bytes_stream()` メソッドに必要。`exchange/Cargo.toml` の reqwest features に `"stream"` を追加。

### 変更ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `exchange/src/adapter/tachibana.rs` | MasterRecord 型、master_record_to_ticker_info()、fetch_all_master()、RwLock キャッシュ、cached_ticker_metadata() |
| `exchange/src/adapter.rs` | `Venue::Tachibana` → `cached_ticker_metadata()` 呼び出し（1行変更） |
| `src/connector/auth.rs` | ログイン成功後に `init_issue_master()` を await で呼び出し |
| `exchange/Cargo.toml` | reqwest features に `"stream"` 追加 |

### 既知の問題

- `src/connector/auth.rs` の `perform_login_*` 系テスト3件が **変更前から** 失敗している（mockito が GET でモック設定しているが、`login()` は POST を送信するため不一致）。本修正とは無関係。

### 残タスク（軽微）

1. **`}` 境界判定の堅牢化**（優先）: パース失敗時に `buf.clear()` せず次の `}` まで蓄積し続ける修正（Shift-JIS 第2バイト 0x7D 問題の対策）。現状は偽境界で前後2レコードが消失するリスクがある（Section 9 Risk 2 参照）。
2. **display 切り捨ての安全化**: `d[..28]` のバイト切り捨て前に `.is_ascii()` チェックを追加し、非 ASCII 混入時は `display = None` にフォールバック。panic 防止のため対応推奨。
3. **`spawn_init_issue_master()` の削除**: `exchange/src/adapter/tachibana.rs:708-715` に存在するが未使用のデッドコード。
4. **`reqwest::Client` の再利用**: `src/connector/auth.rs:57` で `reqwest::Client::new()` を新規作成しているが、ログイン用の `client`（49行目）を再利用すれば接続プールを共有できる。

---

## 12. UX改善: 起動時再ログイン + マスタダウンロードの非同期化（2026-04-10）

### 問題

ログイン時（手動・自動ともに）に `init_issue_master`（約21MB、12秒）を `await` で同期待ちしていたため、
ダッシュボードへの遷移がブロックされ、ログイン画面が長時間表示される UX 問題があった。

加えて、起動タスクが `chain`（直列）で構成されていたため、セッション復元タスクが
Sidebar の全取引所メタデータ取得完了（Binance で最大3.5秒）を待ってから開始されていた。

### 計測結果（タイミングログによる実測）

#### Before（chain + await）

```
+0.0s  new() — tasks queued: open_login_window → launch_sidebar → restore_task
+1.4s  ログインウィンドウ表示（GPU初期化完了）
+5.0s  Sidebar 全取引所メタデータ取得完了（Binance が 3.5s）
+5.0s  try_restore_session START（Sidebar 完了待ちで 4.8s 遅延）
+5.1s  validate_session OK（131ms）
+5.1s  transition_to_dashboard
+6.6s  start_master_download BEGIN（window 起動待ちで 1.5s 遅延）
+18.7s master download complete（12s）→ UpdateMetadata Tachibana: 4208
```

- ログイン画面表示時間: **約4秒**
- セッション復元開始遅延: **4.8秒**

#### After（batch 並列化 + ログインウィンドウ遅延表示）

```
+0.0s  new() — tasks queued: batch[launch_sidebar, restore_task]（ウィンドウなし）
+0.0s  try_restore_session START（即時開始）
+0.1s  validate_session OK（95ms）
+0.1s  SessionRestoreResult(Some) → transition_to_dashboard（ログイン画面を経由しない）
+0.8s  start_master_download BEGIN（dashboard と並列）
+12.8s master download complete → UpdateMetadata Tachibana: 4208
```

- ログイン画面表示時間: **0秒**（セッション有効時はログイン画面を表示しない）
- セッション復元開始遅延: **0秒**

#### セッション無効時

```
+0.0s  new() — tasks queued: batch[launch_sidebar, restore_task]（ウィンドウなし）
+0.0s  try_restore_session START
+0.1s  validate_session FAILED / No saved session
+0.1s  SessionRestoreResult(None) → ログインウィンドウを開く
+1.5s  ログインウィンドウ表示（GPU初期化完了）
```

### 起動仕様

```
起動 → セッション復元試行（ウィンドウなし、~100ms）
  ├─ 成功 → メイン画面を直接表示（ログイン画面を経由しない）
  └─ 失敗 → ログイン画面を表示
```

1. 起動直後にウィンドウを開かず、まず `try_restore_session()` で再ログインを試行する
2. 再ログイン失敗（セッション未保存 or 失効）→ ログイン画面を表示
3. 再ログイン成功 → メイン画面を直接表示（ログイン画面は一切表示されない）

### 修正内容

| 変更 | Before | After | 効果 |
|------|--------|-------|------|
| 起動タスク構成 (`new()`) | `chain`（直列: login window → sidebar → restore） | `Task::batch`（sidebar + restore の2タスク並列、**ウィンドウなし**） | セッション復元が即時開始、ログイン画面を経由しない |
| ログインウィンドウ表示 | `new()` で常に開く | `SessionRestoreResult(None)` 時のみ開く | セッション有効時はログイン画面が一切表示されない |
| ダッシュボード遷移+マスタDL | `chain`（直列: dashboard → master） | `Task::batch`（2タスク並列） | マスタDLが遷移と同時開始、**-0.9s** |
| `init_issue_master` の呼び出し元 | `auth.rs`（ログイン関数内で `await`） | `main.rs` の `start_master_download()`（バックグラウンド `Task::perform`） | ログイン完了がマスタDLをブロックしない |
| マスタDL完了後の反映 | なし（ログイン前に完了済み前提） | `UpdateMetadata(Tachibana, metadata)` メッセージ経由で TickersTable に反映 | ダッシュボード表示後にティッカーが非同期で追加される |

### 変更ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `src/main.rs` | `new()`: ログインウィンドウを開かず `login_window: None` で開始。`SessionRestoreResult(None)` でログインウィンドウを遅延表示。`LoginCompleted`/`SessionRestoreResult(Some)`: ダッシュボード遷移とマスタDLを `Task::batch` で並列化。`start_master_download()` ヘルパー追加 |
| `src/connector/auth.rs` | `perform_login_with_base_url()`, `try_restore_session()` から `init_issue_master` の await を削除。未使用 import `self` を削除 |
