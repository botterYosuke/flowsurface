# 立花証券 e支店 API 実装レビュー報告書

**レビュー日**: 2026-04-09
**対象**: `exchange/src/adapter/tachibana.rs` および関連ファイル
**比較対象**: 公式サンプル `e_api_sample_v4r8.py` (v2.0-000, 2025.09.27)

---

## 1. 総合評価

実装品質は高く、型安全性・テスト網羅性ともに良好。
ただし公式サンプルとの比較で **プロトコルレベルの重要な差異** が複数発見された。
本番接続前に修正が必要な項目がある。

| 評価項目 | 状態 |
|----------|------|
| 型設計・構造体 | ✅ 良好 |
| テスト (21→50件) | ✅ 良好（+13テスト + マスタ/セッション/p_no並行テスト追加） |
| Shift-JIS デコード | ✅ 公式準拠 |
| sJsonOfmt: "5" | ✅ 公式準拠 |
| p_errno / sResultCode 二重チェック | ✅ 全API共通化済み |
| HTTP メソッド (POST/GET) | ✅ 修正済み (2026-04-09) |
| p_no リクエスト通番 | ✅ 修正済み (2026-04-09) |
| パスワード URL エンコード | ✅ 修正済み (2026-04-09) |
| 業務API のエラーチェック | ✅ 修正済み (2026-04-09) |
| EVENT I/F プロトコル理解 | 🔍 新知見あり |
| セッション永続化 | 💡 改善提案 |

---

## 2. 重大な差異（要修正）

### 2.1 ✅ HTTP POST 未対応（v4r8 の主要変更）— 修正済み

**修正内容**: 共通ヘルパー `post_request()` を導入し、`login` / `fetch_market_prices` / `fetch_daily_history` の全3関数を POST に統一。

```rust
// 新設の共通ヘルパー（tachibana.rs）
async fn post_request(
    client: &reqwest::Client,
    url: &str,
    json_body: &str,
) -> Result<String, TachibanaError> {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(json_body.to_string())
        .send()
        .await?;
    decode_response_body(resp).await
}
```

**設計判断**:
- `build_api_url` / `build_api_url_from`（GET 用の `?json` URL 構築）は残存。テスト・他用途で使われるため削除せず、`serialize_request()` を別途追加。
- POST body に JSON 文字列を直接送信（`reqwest::Client::json()` ではなく `.body()` を使用）。理由: 公式サンプルと同じ挙動（`body=pi_prm` は文字列渡し）を再現するため。

**テスト**: 3件追加（`login_sends_post_request_with_json_body`, `fetch_market_prices_sends_post_request`, `fetch_daily_history_sends_post_request`）。mockito で `"POST"` + `Content-Type: application/json` ヘッダを検証。

---

### 2.2 ✅ p_no（リクエスト通番）がハードコード — 修正済み

**修正内容**: `AtomicU64` のグローバルカウンタ `REQUEST_COUNTER` と `next_p_no()` 関数を導入。`LoginRequest::new()`, `MarketPriceRequest::new()`, `DailyHistoryRequest::new()` の全3箇所を動的生成に変更。

```rust
// tachibana.rs
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn next_p_no() -> String {
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // CAS: カウンタが 0（未初期化）の場合のみ epoch_secs で初期化。
    let _ = REQUEST_COUNTER.compare_exchange(0, epoch_secs, Ordering::Relaxed, Ordering::Relaxed);
    let val = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    val.to_string()
}
```

**設計判断**:
- 初期値は `0`（未初期化の番兵）。初回呼び出し時に `compare_exchange` で Unix 秒に初期化し、前セッションの p_no を常に超える。
- `compare_exchange` で初期化を排他。複数スレッドが同時に呼んでも1つだけが CAS に成功し、残りは既に初期化済みのカウンタに対して安全に `fetch_add` する。
- `Ordering::Relaxed` で十分。p_no はユニーク性だけが必要で、他の変数との順序保証は不要。
- セッション永続化は keyring 方式で実装済み（セッション復元時もカウンタが前セッションの値を確実に超える）。

**テスト**: 5件。`next_p_no` 連続呼び出しのインクリメント検証、各リクエスト型で p_no が "1" ハードコードでないこと・連続生成で異なることの検証、並行呼び出しでの全ユニーク検証。

**Tips**: テストは並列実行されるため、`AtomicU64` の絶対値ではなく **相対的なインクリメント** (`b == a + 1`) や **不等** (`req1.p_no != req2.p_no`) で検証している。

---

### 2.3 ✅ パスワードの URL エンコード未実施 — 修正済み

**修正内容**: `login()` 関数内で `urlencoding::encode()` を適用してから `LoginRequest` に渡す。`exchange/Cargo.toml` に `urlencoding = "2"` を追加。

```rust
// tachibana.rs — login() 内
let encoded_password = urlencoding::encode(&password).into_owned();
let req = LoginRequest::new(user_id, encoded_password);
```

**設計判断**:
- エンコードは `LoginRequest::new()` 内ではなく `login()` 関数側で行う。理由: `LoginRequest` は純粋なデータ構造体として保ち、エンコードの責務は呼び出し側に持たせる。これにより、テストで `LoginRequest` を直接生成する際にエンコード有無を制御しやすい。
- `urlencoding::encode()` は `urllib.parse.quote()` と同等のRFC 3986 準拠エンコード。公式 `e_api_login_tel.py` の独自 `func_replace_urlecnode()` は対象文字を限定しているが、RFC 3986 準拠の方が安全側に倒している。
- `into_owned()` で `Cow<str>` から `String` に変換（エンコード不要な文字列の場合はゼロコピーだが、`LoginRequest` が `String` を要求するため変換が必要）。

**テスト**: 1件追加。`"pass{word}"` → body に `"pass%7Bword%7D"` が含まれることを mockito の `Matcher::Regex` で検証。

**Tips**: POST に切り替えた後もパスワードの URL エンコードは必要。公式サンプルが POST モードでもエンコードしているため、サーバー側でデコード処理が入っていると推測される。

---

### 2.4 ✅ 業務 API レスポンスのエラーチェック欠落 — 修正済み

**修正内容**: ジェネリックな `ApiResponse<T>` ラッパー型を導入。`#[serde(flatten)]` で既存のレスポンス型 (`MarketPriceResponse`, `DailyHistoryResponse`) をそのまま内包し、`.check()` メソッドで `p_errno` → `sResultCode` の順にエラー検査する。

```rust
// tachibana.rs — 新設
#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    #[serde(default)]
    pub p_errno: String,
    #[serde(default)]
    pub p_err: String,
    #[serde(rename = "sResultCode", default)]
    pub result_code: String,
    #[serde(rename = "sResultText", default)]
    pub result_text: String,
    #[serde(flatten)]
    pub data: T,
}
```

**設計判断**:
- `#[serde(flatten)]` を使うことで、既存の `MarketPriceResponse` / `DailyHistoryResponse` 型を一切変更せずに共通エラーフィールドを追加できた。既存テストへの影響を最小化。
- `LoginResponse` にはすでに `p_errno` / `sResultCode` が直接含まれ、`TryFrom` で独自チェックしているため、`ApiResponse<T>` のラップ対象外。ログインのエラーハンドリングは元のまま。
- `.check()` メソッドは `self` を消費（`self`、非`&self`）する設計。理由: エラー時にデータを使うことはなく、成功時はデータの所有権を返すため。
- エラーコードは既存の `TachibanaError::ApiError { code, message }` variant を活用。新しい variant の追加なし。
- `p_errno` の優先チェック: 空文字列 **かつ** `"0"` 以外の場合のみエラー。公式サンプルの `ans_check()` と同等のロジック。

**テスト**: 5件追加。
- ユニット: `ApiResponse<T>` の正常系・`p_errno` エラー・`sResultCode` エラーの3パターン
- 統合: `fetch_market_prices` / `fetch_daily_history` が mockito サーバーからエラーレスポンスを受け取った場合に `ApiError` を返すことを検証

**Tips**: 既存の mockito テスト（`fetch_market_prices_returns_records`, `fetch_daily_history_returns_candles`）のレスポンスに `p_errno`/`sResultCode` フィールドを追加する必要があった。`ApiResponse` が `#[serde(default)]` を使っているため省略しても動作するが、明示的に `"0"` を入れてテストの意図を明確にした。

**既知のエラーコード（公式サンプルより）**:
| `p_errno` | 意味 |
|-----------|------|
| `0` | 正常 |
| `2` | セッション切断 |
| `-62` | 稼働時間外 |

---

## 3. 新知見：EVENT I/F プロトコル

公式サンプルから EVENT I/F の実装詳細が判明した。これは移行プランの「未確定事項」を解消する重要な情報。

### 3.1 EVENT I/F パラメータ形式

**重要発見**: EVENT I/F は JSON 形式ではなく、**通常の URL クエリストリング形式**を使用する。

```python
# e_api_sample_v4r8.py:509
p_prm = "p_rid=22&p_board_no=1000&p_gyou_no=1,2,3&p_issue_code=6501,7203,8411&p_mkt_code=00,00,00&p_eno=0&p_evt_cmd=ST,KP,FD"
```

パラメータの意味:
| パラメータ | 値 | 意味 |
|-----------|-----|------|
| `p_rid` | 22 | リクエストID |
| `p_board_no` | 1000 | ボード番号 |
| `p_gyou_no` | 1,2,3 | 行番号（銘柄のスロット） |
| `p_issue_code` | 6501,7203,8411 | 銘柄コード（カンマ区切り） |
| `p_mkt_code` | 00,00,00 | 市場コード（各銘柄に対応） |
| `p_eno` | 0 | イベント番号（開始位置） |
| `p_evt_cmd` | ST,KP,FD | イベントコマンド |

`p_evt_cmd` の種類:
- `ST` = 歩み値（ティック）
- `KP` = 現在値
- `FD` = 板情報 (Full Depth)

### 3.2 EVENT I/F データフォーマット

**重要発見**: 受信データは JSON ではなく、**バイナリ区切り文字**を使用するカスタムプロトコル。

```python
# e_api_sample_v4r8.py:460-468
def proc_print_event_if_data(pi_data):
    pa_rec = pi_data.split('\x01')      # SOH (0x01) = レコード区切り
    for p_rec in pa_rec:
        if (p_rec):
            pa_colval = p_rec.split('\x02')  # STX (0x02) = カラム:値 区切り
            print("col:[" + pa_colval[0] + "], val:[" + pa_colval[1] + "]")
```

- `\x01` (SOH) = レコード（項目値）区切り
- `\x02` (STX) = カラム名と値の区切り
- `\x03` (ETX) = 値と値のサブ区切り（複数値を持つフィールド内の区切り）
- テキストエンコーディングは ASCII

補足: `e_api_event_receive_tel.py` の実装ではこの3種の区切り子を使用:
```python
# e_api_event_receive_tel.py:563-568
# 通知データは「^A」「^B」「^C」を区切り子とし...
# 区切り子「^A」は1項目値、「^B」は項目と値、「^C」は値と値の各区切り。
```

### 3.3 WebSocket 接続パラメータ

```python
# e_api_sample_v4r8.py:439
websockets.connect(pi_url, ping_interval=86400, ping_timeout=10)
```

- `ping_interval=86400`（24時間）: サーバー側から ping が来る設計、クライアント ping は実質無効
- `ping_timeout=10`: pong の待ち時間は10秒
- **注意**: `e_api_websocket_receive_tel.py:754` では `ping_interval=None`（無効）で接続しており、サンプル間で設定が異なる。実動作検証で要確認

### 3.4 HTTP Long-polling の実装

```python
# e_api_sample_v4r8.py:412-415
p_ss = requests.session()
p_res = p_ss.get(p_url, stream=True)
for p_rec in p_res.iter_lines():
    pi_cbp(p_rec.decode('ascii'))
```

- HTTP GET のストリーミングレスポンス（Server-Sent Events 的）
- 1行ごとにイベントデータが送られる
- エンコーディングは ASCII（Shift-JIS ではない）

### 3.5 WebSocket vs HTTP の接続先

- **HTTP**: `sUrlEvent + "?" + params`
- **WebSocket**: `sUrlEventWebSocket + "?" + params`
- 同一パラメータで両方使用可能（切り替え可能な設計）

---

## 4. 中程度の差異（改善推奨）

### 4.1 ✅ セッションの永続化（keyring 保存）— 実装済み (2026-04-10)

**修正内容**: Windows Credential Manager (keyring) にセッション JSON を保存。起動時に `try_restore_session()` で復元・検証し、有効ならログイン画面をスキップしてダッシュボードに直行する。

- `data/src/config/tachibana.rs`: keyring への save/load/delete
- `src/connector/auth.rs`: `persist_session()`, `try_restore_session()`
- `src/main.rs`: `SessionRestoreResult` メッセージによる起動フロー分岐

詳細は `✅tachibana_session_restore.md` 参照。

### 4.2 ✅ MarketPriceResponse の p_errno チェック — 2.4 で解決済み

`ApiResponse<T>` ラッパーにより、全業務 API で `p_errno` / `sResultCode` を検証するようになった。
セッション切れ（`p_errno: 2`）を検出して自動再ログインにつなげる仕組みは、上位層（`auth.rs` / `fetcher.rs`）で `TachibanaError::ApiError` をハンドリングして実装する想定。

### 4.3 ✅ LoginRequest の p_no が固定 "1" — 2.2 で解決済み

全リクエスト型（ログイン含む）で `next_p_no()` を使用するように統一済み。
`REQUEST_COUNTER` は `compare_exchange(0, epoch_secs)` で初期化するため、セッション復元時もタイムスタンプベースで前回の p_no を常に超える。ファイル永続化は不要。

---

## 5. 良好な実装ポイント

### 5.1 型安全性
- `TachibanaError` の thiserror 活用が適切
- `TryFrom<LoginResponse> for TachibanaSession` による安全な変換
- `daily_record_to_kline` の `Option` 返却（`"*"` 未取得データのハンドリング）

### 5.2 テスト品質
- 50テスト（ユニット + mockito 統合テスト）で主要パスをカバー（元21 → 50）
- 境界ケース（`"*"` フィールド、未読書面フラグ、認証エラー）もテスト済み
- 追加分: HTTP メソッド検証、p_no 動的生成検証+並行ユニーク検証、パスワードエンコード検証、API エラーハンドリング検証、validate_session 未知p_errno検証

### 5.3 Shift-JIS デコード
- `encoding_rs::SHIFT_JIS` の使用は公式サンプルの `decode('shift-jis')` と一致
- `decode_response_body()` として共通化されている
- lossy flag (`had_errors`) を検査し、文字化け時に `log::warn!` を出力（`decode_response_body` とマスタストリームの2箇所）

### 5.4 sJsonOfmt: "5" の設定
- 公式サンプルと同一。これがないと応答が数値キー形式になり解析困難

### 5.5 アーキテクチャ統合
- `Exchange::Tachibana` / `Venue::Tachibana` の追加が既存アーキテクチャに自然に統合
- `connect.rs` のスタブが将来の EVENT I/F 実装への足場として適切
- `auth.rs` のセッション管理が Iced の非同期パターンに適合

---

## 6. 修正優先度まとめ

### ✅ 高（本番接続前に必須）— 全件修正済み (2026-04-09)

| # | 項目 | 状態 | テスト |
|---|------|------|--------|
| 1 | HTTP POST 対応 | ✅ 完了 | +3件 |
| 2 | p_no インクリメンタルカウンタ | ✅ 完了 | +4件 |
| 3 | パスワード URL エンコード | ✅ 完了 | +1件 |
| 4 | 業務API エラーチェック（p_errno） | ✅ 完了 | +5件 |

**変更ファイル**: `exchange/src/adapter/tachibana.rs`, `exchange/Cargo.toml` (+`urlencoding = "2"`)
**テスト結果**: 54テスト全パス（exchange 50 + data 3 + 統合テスト 1）、アプリ層 20テスト全パス

### 🟡 中（品質・UX 改善）

| # | 項目 | 影響 | 対象ファイル | 状態 |
|---|------|------|-------------|------|
| 5 | セッション永続化 | 再起動時に再ログイン必要 | auth.rs, data/config/tachibana.rs | ✅ keyring 方式で実装済み |
| 6 | セッション切れ検出→自動再ログイン | 日中の断続エラー | fetcher.rs, auth.rs | 未実装 |

**Tips (5, 6 に着手する人向け)**:
- `ApiResponse<T>.check()` が返す `TachibanaError::ApiError { code: "2", .. }` がセッション切れのシグナル。上位層でこのコードをマッチして再ログインフローを起動する設計が自然。
- `REQUEST_COUNTER` は `compare_exchange(0, epoch_secs)` で初期化するため、セッション復元時もタイムスタンプベースで前回の p_no を常に超える。ファイル永続化は不要。

### 🟢 低（将来対応）

| # | 項目 | 影響 | 対象ファイル |
|---|------|------|-------------|
| 7 | EVENT I/F 実装（本報告の新知見を活用） | リアルタイムデータ未対応 | connect.rs, 新規 |
| 8 | 専用アイコン（現在 Icon::Star） | UI 見た目 | style.rs |

---

## 7. EVENT I/F 実装ガイド（新知見に基づく設計メモ）

公式サンプルから判明した仕様に基づき、Phase 3 WebSocket 実装の設計指針を記載する。

### 接続フロー
```
1. ログインで sUrlEventWebSocket を取得
2. パラメータ構築:
   "p_rid={rid}&p_board_no={board}&p_gyou_no={行番号列}&p_issue_code={銘柄列}&p_mkt_code={市場列}&p_eno=0&p_evt_cmd=ST,KP,FD"
3. WebSocket 接続: {sUrlEventWebSocket}?{params}
4. 受信データのパース: SOH(0x01) でレコード分割 → STX(0x02) でカラム:値分割
```

### Rust 実装の方針
```rust
// EVENT I/F のパラメータ構築（JSON ではなく URL クエリストリング）
fn build_event_params(issues: &[(&str, &str)]) -> String {
    let gyou_nos: Vec<String> = (1..=issues.len()).map(|i| i.to_string()).collect();
    let issue_codes: Vec<&str> = issues.iter().map(|(code, _)| *code).collect();
    let mkt_codes: Vec<&str> = issues.iter().map(|(_, mkt)| *mkt).collect();
    
    format!(
        "p_rid=1&p_board_no=1000&p_gyou_no={}&p_issue_code={}&p_mkt_code={}&p_eno=0&p_evt_cmd=ST,KP,FD",
        gyou_nos.join(","),
        issue_codes.join(","),
        mkt_codes.join(","),
    )
}

// 受信データのパース
fn parse_event_data(data: &str) -> Vec<(String, String)> {
    data.split('\x01')
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let parts: Vec<&str> = record.split('\x02').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect()
}
```

### WebSocket 接続設定
- ping_interval は大きな値（86400秒）に設定（サーバー側 ping に依存）
- ping_timeout は10秒
- fastwebsockets の設定で対応可能

---

## 8. 結論

~~現在の実装は構造設計・テスト品質ともに良好だが、公式サンプル v4r8 との比較で **HTTP POST 対応**、**p_no 通番管理**、**パスワード URL エンコード**、**業務API エラーチェック** の4点が本番接続前に修正必須。~~

**2026-04-09 更新**: セクション2の重大な差異4件はすべて修正済み。TDD（テスト駆動開発）アプローチで実装し、各修正に対して先にテストを書いて失敗を確認してから実装した。

**2026-04-10 更新**: レビューレポート（`implementation_review_report.md`）の Warning 5件（W1-W4, W6）をすべて修正。テスト数は 25 → 54 に増加（exchange 50 + data 3 + 統合 1）。アプリ層テスト 20件も全パス。

**現状**: 本番接続テストに進める状態。残る作業は中優先度の UX 改善（自動再ログイン）と低優先度の EVENT I/F 実装。セッション永続化は keyring 方式で実装済み。

EVENT I/F の実装に必要なプロトコル詳細（SOH/STX 区切りのカスタムフォーマット、URL クエリストリングパラメータ形式）が公式サンプルから判明したため、Phase 3 の WebSocket 実装に着手可能な状態となった。

---

## 9. 修正作業の技術メモ（他の作業者向け）

### 9.1 変更されたファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `exchange/src/adapter/tachibana.rs` | 本体修正（POST化、p_noカウンタ、URLエンコード、ApiResponse<T>）+ テスト13件追加 |
| `exchange/Cargo.toml` | `urlencoding = "2"` 追加 |

### 9.2 新規導入した主要シンボル

| シンボル | 種類 | 用途 |
|---------|------|------|
| `REQUEST_COUNTER` | `static AtomicU64` | グローバルリクエスト通番カウンタ（`new(0)` + CAS初期化） |
| `next_p_no()` | `pub fn` | 次の通番を文字列で返す（`compare_exchange` でスレッドセーフ初期化） |
| `serialize_request<T>()` | `fn` | リクエスト構造体 → JSON 文字列 |
| `post_request()` | `async fn` | POST 送信 + Shift-JIS デコードの共通処理 |
| `ApiResponse<T>` | `pub struct` | 業務API 共通レスポンスラッパー（p_errno/sResultCode + flatten data） |

### 9.3 既存 mockito テストへの影響

既存の mockito テスト（`login_returns_session_on_success` 等）は以下の変更が必要だった:
- `"GET"` → `"POST"` に変更（exchange 層 + アプリ層の auth.rs / fetcher.rs の両方で修正済み）
- `fetch_*` 系テストのレスポンスに `p_errno`, `p_err`, `sResultCode`, `sResultText` フィールドを追加
- fetcher.rs のフィールド名 `aCLMMfdsGetMarketPriceHistory` → `aCLMMfdsMarketPriceHistory` に修正

### 9.4 本番接続テストの手順

1. 電話認証を完了（ユーザー手動）
2. `BASE_URL_DEMO` または `BASE_URL_PROD` を選択
3. `login()` を呼び出し → `TachibanaSession` 取得を確認
4. `fetch_market_prices()` で銘柄取得テスト
5. エラーケース確認: 稼働時間外 (`p_errno: -62`) の挙動

### 9.5 注意事項

- **`build_api_url` / `build_api_url_from` は残存**: GET 用の URL 構築ヘルパー。POST 移行後は直接使われていないが、テスト (`build_api_url_appends_json_after_question_mark` 等) が存在する。将来 GET が必要になる場面（EVENT I/F の HTTP Long-polling など）で使用可能。
- **`LoginRequest` にはエンコード済みパスワードが入る**: `LoginRequest::new()` に渡す前に `login()` 側で URL エンコードしている。`LoginRequest` を直接テストする既存テストはエンコード前の値を渡している（単体テストとしてはこれで正しい）。