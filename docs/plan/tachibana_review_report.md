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
| テスト (21件) | ✅ 良好 |
| Shift-JIS デコード | ✅ 公式準拠 |
| sJsonOfmt: "5" | ✅ 公式準拠 |
| p_errno / sResultCode 二重チェック | ✅ 公式準拠 (ログインのみ) |
| HTTP メソッド (POST/GET) | ⚠️ 要修正 |
| p_no リクエスト通番 | ⚠️ 要修正 |
| パスワード URL エンコード | ⚠️ 要修正 |
| 業務API のエラーチェック | ⚠️ 欠落 |
| EVENT I/F プロトコル理解 | 🔍 新知見あり |
| セッション永続化 | 💡 改善提案 |

---

## 2. 重大な差異（要修正）

### 2.1 HTTP POST 未対応（v4r8 の主要変更）

**現状**: すべてのリクエストを `GET` で送信している。

```rust
// tachibana.rs:352
let resp = client.get(&url).send().await?;
```

**公式サンプル**: v4r8 では **POST がデフォルト** になっている。

```python
# e_api_sample_v4r8.py:92
self._p_debug_http_post = 1  # request to http-post or http-get.

# e_api_sample_v4r8.py:195-206
if self._p_debug_http_post == 1:
    p_resp = p_http.request(
        'POST', pi_url,
        body=pi_prm,
        headers={'Content-Type': 'application/json'}
    )
else:
    p_resp = p_http.request('GET', (pi_url + '?' + pi_prm))
```

**影響**: 
- GET の場合、JSON パラメータが URL に含まれるため URL 長制限に引っかかる可能性（特に複数銘柄指定時）
- v4r8 で POST が追加された理由が URL 長制限の回避であると推測される
- GET でも動作はするが、POST が推奨

**修正案**:
```rust
// POST モードに変更
let resp = client
    .post(&base_url)
    .header("Content-Type", "application/json")
    .body(json_body)
    .send()
    .await?;
```

**優先度**: 🔴 高

---

### 2.2 p_no（リクエスト通番）がハードコード

**現状**: すべてのリクエストで `p_no: "1"` を固定で送信。

```rust
// tachibana.rs:71, 198, 265
pub p_no: "1".to_string(),
```

**公式サンプル**: `p_no` はインクリメンタルカウンタで、リクエスト毎に +1 し、ファイルに永続化する。

```python
# e_api_sample_v4r8.py:142-149
def _gen_no(self, pi_ary):
    self._p_no += 1
    pi_ary = self._gen_col_val(pi_ary, "p_no", str(self._p_no))
    self._file_save()
    return (pi_ary)
```

**影響**:
- サーバー側でリクエストの重複検出やシーケンス管理をしている場合、同一 `p_no` のリクエストが拒否される可能性がある
- 公式サンプルではログインも含めて全リクエストで通番をインクリメントしている

**修正案**:
```rust
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_p_no() -> String {
    REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}
```

**優先度**: 🔴 高

---

### 2.3 パスワードの URL エンコード未実施

**現状**: パスワードをそのまま JSON に含めている。

```rust
// tachibana.rs:71
pub fn new(user_id: String, password: String) -> Self {
    Self { ..., password, ... }
}
```

**公式サンプル**: パスワードに含まれる記号を URL エンコードしてから JSON に含めている。

```python
# e_api_sample_v4r8.py:483
gp_pswd = urllib.parse.quote(gp_pswd)
```

**影響**:
- パスワードに `"`, `{`, `}`, `&` 等の特殊文字が含まれる場合、JSON/URL が壊れてログイン失敗する
- GET モードでは URL に直接含まれるため特に危険

**修正案**:
```rust
use urlencoding::encode;

let encoded_password = encode(&password).to_string();
let req = LoginRequest::new(user_id, encoded_password);
```

**補足**: 公式 `e_api_login_tel.py` では `urllib.parse.quote` ではなく独自の `func_replace_urlecnode()` 関数で手動エンコードしている（対象文字が限定的）。`urllib.parse.quote` で十分かは要確認。

**優先度**: 🔴 高（GET モード使用中は特に）

---

### 2.4 業務 API レスポンスのエラーチェック欠落

**現状**: `MarketPriceResponse` と `DailyHistoryResponse` には `p_errno`/`sResultCode` フィールドがなく、エラーチェックが行われていない。

```rust
// tachibana.rs:236-239 — エラーフィールドなし
pub struct MarketPriceResponse {
    #[serde(rename = "aCLMMfdsMarketPrice")]
    pub records: Vec<MarketPriceRecord>,
}
```

**公式サンプル**: すべてのレスポンスに対して `ans_check()` でエラーチェックを実施。

```python
# e_api_sample_v4r8.py:236-259
def ans_check(self, pi_ans):
    p_errno      = self.ans_get_val(pi_ans, 'p_errno',      "unknown")
    p_err        = self.ans_get_val(pi_ans, 'p_err',        "unknown")
    p_sResultCode = self.ans_get_val(pi_ans, 'sResultCode', "0")
    p_sResultText = self.ans_get_val(pi_ans, 'sResultText', "")
```

**影響**:
- セッション切れ（`p_errno: 2`）時にエラーが検出されず、空データとして処理される
- 稼働時間外（`p_errno: -62`）のエラーが無視される
- `serde_json` のデシリアライズ自体が失敗するか、空の配列として解釈される可能性

**補足**: `TachibanaError::ApiError` (tachibana.rs:29) が定義済みだが未使用。このラッパー導入で活用すべき。

**修正案**: 共通のレスポンスラッパー型を導入
```rust
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

impl<T> ApiResponse<T> {
    pub fn check(self) -> Result<T, TachibanaError> {
        if self.p_errno != "0" && !self.p_errno.is_empty() {
            return Err(TachibanaError::ApiError {
                code: self.p_errno,
                message: self.p_err,
            });
        }
        // ... sResultCode check
        Ok(self.data)
    }
}
```

**優先度**: 🔴 高

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

### 4.1 セッションの永続化（日次ファイル保存）

**現状**: セッションはメモリ上の `static RwLock` のみで、アプリ再起動時に再ログインが必要。

**公式サンプル**: `YYYYMMDD_e_api_sample.txt` にセッション情報を保存し、同日中は再利用。

```python
# e_api_sample_v4r8.py:351-383
def req_or_file_login(self, pi_url, pi_usid, pi_pswd):
    if self._file_load() != 0:
        # ファイルがなければログイン
        p_ans = self.req_login(pi_url, pi_usid, pi_pswd)
        ...
    # ファイルがあれば再利用
```

**修正案**: 設定ディレクトリに日付付きセッションファイルを保存し、起動時に有効なセッションがあれば再利用する。電話認証が必要な API なので、再ログイン回数を最小化することはユーザー体験上重要。

**優先度**: 🟡 中（UX 改善）

### 4.2 MarketPriceResponse の p_errno チェック

`fetch_market_prices` と `fetch_daily_history` で `p_errno` のチェックがない（2.4 と関連）。
特にセッション切れ時のエラーコード（`p_errno: 2`, `p_err: "セッションが切断されました。"`）を検出して自動再ログインにつなげる必要がある。

**優先度**: 🟡 中

### 4.3 LoginRequest の p_no が固定 "1"

ログインリクエスト自体は `p_no: "1"` でも動作する可能性があるが、公式サンプルではログインも含めてインクリメンタルカウンタを使用している。

セッション永続化を実装する場合、p_no も永続化して連続させる必要がある。

**優先度**: 🟡 中

---

## 5. 良好な実装ポイント

### 5.1 型安全性
- `TachibanaError` の thiserror 活用が適切
- `TryFrom<LoginResponse> for TachibanaSession` による安全な変換
- `daily_record_to_kline` の `Option` 返却（`"*"` 未取得データのハンドリング）

### 5.2 テスト品質
- 21テスト（ユニット + mockito 統合テスト）で主要パスをカバー
- 境界ケース（`"*"` フィールド、未読書面フラグ、認証エラー）もテスト済み

### 5.3 Shift-JIS デコード
- `encoding_rs::SHIFT_JIS` の使用は公式サンプルの `decode('shift-jis')` と一致
- `decode_response_body()` として共通化されている

### 5.4 sJsonOfmt: "5" の設定
- 公式サンプルと同一。これがないと応答が数値キー形式になり解析困難

### 5.5 アーキテクチャ統合
- `Exchange::Tachibana` / `Venue::Tachibana` の追加が既存アーキテクチャに自然に統合
- `connect.rs` のスタブが将来の EVENT I/F 実装への足場として適切
- `auth.rs` のセッション管理が Iced の非同期パターンに適合

---

## 6. 修正優先度まとめ

### 🔴 高（本番接続前に必須）

| # | 項目 | 影響 | 対象ファイル |
|---|------|------|-------------|
| 1 | HTTP POST 対応 | v4r8 推奨メソッド未対応 | tachibana.rs |
| 2 | p_no インクリメンタルカウンタ | リクエスト拒否の可能性 | tachibana.rs |
| 3 | パスワード URL エンコード | 特殊文字でログイン失敗 | tachibana.rs |
| 4 | 業務API エラーチェック（p_errno） | セッション切れ・時間外エラー未検出 | tachibana.rs |

### 🟡 中（品質・UX 改善）

| # | 項目 | 影響 | 対象ファイル |
|---|------|------|-------------|
| 5 | セッション永続化（日次ファイル） | 再起動時に再ログイン必要 | auth.rs |
| 6 | セッション切れ検出→自動再ログイン | 日中の断続エラー | fetcher.rs, auth.rs |

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

現在の実装は構造設計・テスト品質ともに良好だが、公式サンプル v4r8 との比較で **HTTP POST 対応**、**p_no 通番管理**、**パスワード URL エンコード**、**業務API エラーチェック** の4点が本番接続前に修正必須。

また、EVENT I/F の実装に必要なプロトコル詳細（SOH/STX 区切りのカスタムフォーマット、URL クエリストリングパラメータ形式）が公式サンプルから判明したため、Phase 3 の WebSocket 実装に着手可能な状態となった。