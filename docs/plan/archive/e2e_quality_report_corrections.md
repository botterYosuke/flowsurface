# E2E 品質レポート 修正計画

> 作成日: 2026-04-17  
> 対象: `docs/plan/e2e_quality_report.md`  
> 起因: `src/replay_api.rs` 精読による誤認・見落とし 4 件の特定

---

## 修正サマリー

| # | 箇所 | 種別 | 優先度 |
|---|---|---|---|
| C1 | Section 3-2「Race condition」行 | 根本前提の誤り → 記述を刷新 | 高 |
| C2 | Section 2-1 テーブル（全エンドポイント） | 見落とし → 500 パスを追記 | 中 |
| C3 | Section 4-3「s17_error_boundary.sh:40-46」行 | 誤認 → 訂正 | 中 |
| C4 | Section 4-3 末尾 | 見落とし → `kind: null` 暗黙等価性を追記 | 低 |

---

## C1 — Section 3-2「Race condition」行の刷新

### 問題

`replay_api.rs:203-289` の `run_server` は `listener.accept()` → `read_full_request().await` → `reply_rx.await` を **シリアルに実行**する。次の `accept()` は前リクエストの応答が完了するまで呼ばれない。

```rust
loop {
    let (mut stream, _peer) = match listener.accept().await { ... };
    // ↑ この時点で次の accept には戻らない
    let request_string = match read_full_request(&mut stream).await { ... };
    ...
    match reply_rx.await { ... }  // iced 応答待ち完了後に次のループへ
}
```

これにより、`s41_race_toggle_play.sh` で提案している「toggle → sleep 0 → play × 10 の連続発行」はサーバー側でシリアライズされ、**HTTP 層での Race condition を再現できない**。

### 修正内容（Section 3-2 の該当行を置き換え）

**変更前:**

| バグパターン | 現 TC の限界 | 追加すべき TC |
|---|---|---|
| toggle → play 高速切替時のバッファ競合（Race condition） | 各操作間に 1 秒以上の sleep が入っている | `s41_race_toggle_play.sh`: toggle → sleep 0 → play × 10 を連続実行し、最終的に Paused/Playing が安定するか検証 |

**変更後:**

| バグパターン | 現 TC の限界 | 追加すべき TC |
|---|---|---|
| toggle → play 高速切替時のバッファ競合（Race condition） | 各操作間に 1 秒以上の sleep が入っている | **E2E テストでの再現不可**。`replay_api.rs` の HTTP サーバーはシリアル処理のため、HTTP 層で同時リクエストを発行しても連続実行に変換される。Race condition は `iced` の `update()` Message キュー内に存在し、unit test / integration test で検証すること（E2E では検出不能と明記）。`s41` は "連続高速発行でクラッシュしないか" の **安定性テスト** として再定義する。 |

### 優先度マトリクスへの影響

Section 5「優先度マトリクス」の `s41_race_toggle_play.sh` 行を以下に修正:

**変更前:**
> `s41_race_toggle_play.sh` 追加（toggle/play 競合の Race condition 検出）

**変更後:**
> `s41_stability_burst.sh` 追加（toggle/play 連続発行のクラッシュ安定性検証。Race condition 検出は iced unit test で対応）

---

## C2 — Section 2-1 テーブルへの 500 パス追記

### 問題

`replay_api.rs:270-288` に 2 つの HTTP 500 応答パスが存在するが、現レポートの Section 2-1 テーブルに記載がない。

```rust
// パス A: iced channel が詰まった場合
if sender.send((command, ReplySender::new(reply_tx))).await.is_err() {
    let _ = write_response(&mut stream, 500, r#"{"error":"App channel closed"}"#).await;
    continue;
}

// パス B: ReplySender が drop され oneshot がキャンセルされた場合
Err(_) => {
    write_response(&mut stream, 500, r#"{"error":"No response from app"}"#).await;
}
```

どちらも現行 E2E テストで検証されておらず、運用上の重大障害を意味する。

### 修正内容（Section 2-1 テーブルへ行を追加）

テーブル末尾（`GET /api/app/screenshot` 行の後）に以下を追記:

| エンドポイント | 正常系 | 4xx/5xx | 評価 | 主なギャップ |
|---|---|---|---|---|
| **全エンドポイント共通（HTTP 500 パス）** | — | **なし** | **要改善** | ① `{"error":"App channel closed"}`: iced channel 輻輳時に返る。② `{"error":"No response from app"}`: ReplySender drop 時に返る。どちらも E2E で未検証。運用上の重大障害パスであり、fault injection での検証を検討する |

---

## C3 — Section 4-3「s17_error_boundary.sh:40-46」行の訂正

### 問題

現行の記述:

> `s17_error_boundary.sh:40-46` | 不正 pane_id への `pane/split` が HTTP 200 を返す TC で、正常応答か error 応答か区別せず PASS

`replay_api.rs:500-509` の `parse_split_command` を見ると、**不正 UUID（UUID フォーマット違反）は `RouteError::BadRequest` → 400** を返す。現行記述の "HTTP 200 を返す" は誤り。

```rust
// parse_split_command の UUID パース失敗 → BadRequest(400) が返る
```

実際の問題は「UUID フォーマットとしては有効だが存在しない `pane_id`」を渡した場合、`route()` は 200 を返し、**app 層での "not found" エラーがテストで区別されていない**点である。

### 修正内容（Section 4-3 の該当行を置き換え）

**変更前:**

| スクリプト:行番号 | 問題 | 推奨修正 |
|---|---|---|
| `s17_error_boundary.sh:40-46` | 不正 pane_id への `pane/split` が HTTP 200 を返す TC で、正常応答か error 応答か区別せず PASS | レスポンス body に `"error"` キーが存在するかを検証 |

**変更後:**

| スクリプト:行番号 | 問題 | 推奨修正 |
|---|---|---|
| `s17_error_boundary.sh:40-46` | UUID フォーマット違反の `pane_id` → `parse_split_command` が `BadRequest(400)` を返す（HTTP ステータスは正しい）。問題は「UUID フォーマットは有効だが存在しない `pane_id`」を渡した場合に `route()` が 200 を返し、**app 層の "not found" エラーが正常応答と区別されない**こと | 存在しない UUID を `pane_id` に指定して `pane/split` を実行し、レスポンス body に `"error"` キーが存在することを確認する TC を追加 |

---

## C4 — Section 4-3 末尾への `kind: null` 暗黙等価性の追記

### 問題

`replay_api.rs:534-541` の `body_opt_str_field`:

```rust
Ok(parsed.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()))
```

JSON で `"kind": null` を送ると `as_str()` が `None` を返し、**フィールド省略と同じ `kind: None` として処理される**。これは仕様書に記載がなく、必須フィールドを扱う `body_str_field` との一貫性も崩れている。

`jqn` の null 問題（`"null"` 文字列と JSON null の区別なし）と構造が類似しており、どちらも「null を特別扱いしない」実装の two-sided problem である。

### 修正内容（Section 4-3 末尾に追記）

`common_helpers.sh:23` の `jqn` 行の後に以下を追加:

| スクリプト:行番号 | 問題 | 推奨修正 |
|---|---|---|
| `replay_api.rs:534-541`（`body_opt_str_field`）が影響する TC | `"kind": null` を送ると `body_opt_str_field` がフィールド省略と同一視し `None` を返す。`parse_sidebar_select_ticker` のみが現在の影響先 | **仕様決定済み**: null ≡ omission を正式仕様として採用。docstring に明記し、ユニットテスト `opt_str_field_null_equals_omission` で仕様を固定済み。E2E テスト追加は不要（unit test で十分） |

---

## C5 — ソースコード修正: `handle_pane_api` が app 層エラーを常に HTTP 200 で返す

### 問題

C3 の精査で判明した**ソースコード上のバグ**。

`main.rs:1212-1213` で `handle_pane_api` の戻り値を `reply_tx.send(body)` で送信しているが、
`ReplySender::send()` は **固定で HTTP 200** を返す（`replay_api.rs:110`）。

```rust
// main.rs:1212-1213
let (body, task) = self.handle_pane_api(cmd);
reply_tx.send(body);  // → tx.send((200, body)) — 常に 200
```

一方、`pane_api_split` 等の app 層関数は存在しない `pane_id` に対して
`{"error":"pane not found: ..."}` を body に詰めて返す（`main.rs:1847-1851`）。
**body にエラーがあっても HTTP ステータスは 200 のまま**。

影響を受ける全関数:
- `pane_api_split` — 存在しない pane_id → `{"error":"pane not found"}`（200 で返る）
- `pane_api_close` — 同上
- `pane_api_set_ticker` — 同上
- `pane_api_set_timeframe` — 同上
- `pane_api_split` — 不正 axis → `{"error":"invalid axis"}`（200 で返る）

### 修正内容

**Step 1**: `handle_pane_api` の戻り値を `(String, Task)` → `(u16, String, Task)` に変更

```rust
// 変更前
fn handle_pane_api(&mut self, cmd: PaneCommand) -> (String, Task<Message>)

// 変更後
fn handle_pane_api(&mut self, cmd: PaneCommand) -> (u16, String, Task<Message>)
```

**Step 2**: 各 `pane_api_*` 関数の戻り値にステータスコードを追加

```rust
// pane_api_split — 正常時
(200, ok.to_string(), task)

// pane_api_split — pane not found
(404, format!(r#"{{"error":"pane not found: {pane_id}"}}"#), Task::none())

// pane_api_split — invalid axis
(400, format!(r#"{{"error":"invalid axis: {axis_str}"}}"#), Task::none())
```

**Step 3**: 呼び出し側で `reply_tx.send_status()` を使う

```rust
// main.rs — 変更前
let (body, task) = self.handle_pane_api(cmd);
reply_tx.send(body);

// main.rs — 変更後
let (status, body, task) = self.handle_pane_api(cmd);
reply_tx.send_status(status, body);
```

**Step 4**: 同様に `handle_replay_api`, `handle_auth_api` も精査し、
エラーを body に詰めている箇所があれば同じパターンで修正する。

### 既存テストへの影響

- `s17_error_boundary.sh` が「存在しない pane_id で 200」を期待している場合 → 404 に変わるため修正が必要
- `route_post_pane_split_*` 等の unit test → 戻り値の型変更に伴う修正

---

## C6 — ✅ 仕様決定済み: `body_opt_str_field` の `null` ≡ omission を正式仕様とする

### 決定

**`null` はフィールド省略と等価（`None`）として扱う。現行動作を維持する。**

### 根拠

- JSON の慣例として、オプショナルフィールドの `null` は「未指定」と同義
- `body_str_field`（必須）が null を 400 で拒否するのは正しい — 必須フィールドに値がないのはエラー
- `body_opt_str_field`（省略可）が null を `None` として扱うのも正しい — 「値なし」の表現が2通り（省略・null）あるだけ
- 唯一の呼び出し元 `parse_sidebar_select_ticker` の `kind` は「省略時はデフォルト」のセマンティクスであり、null も同じ扱いが自然
- **一貫性は崩れていない**: 必須 vs 省略可で null の扱いが異なるのは意図的な設計

### 実施済み修正

1. `replay_api.rs:533-535`: `body_opt_str_field` の docstring に null 等価性を明記
2. `replay_api.rs` テストモジュール: `opt_str_field_null_equals_omission` 他 4 件のユニットテストを追加し仕様を固定

---

## 作業手順

```
C3（レポート訂正）→ C5（ソースコード修正）→ C1（レポート刷新）→ C2（レポート追記）→ C4（レポート追記）→ C6（仕様決定後）
```

C5 を C1 より前にする理由: C3 の訂正内容を正しく理解した上でソースコードを修正し、
修正後の挙動を踏まえてレポートを更新する。

### チェックリスト

- [x] C3: Section 4-3「s17_error_boundary.sh:40-46」行を訂正
- [x] C5: `handle_pane_api` の戻り値に HTTP ステータスコードを追加（`main.rs`）
- [x] C5: `pane_api_split`, `pane_api_close`, `pane_api_set_ticker`, `pane_api_set_timeframe`, `pane_api_open_order_pane`, `pane_api_sidebar_select_ticker` の戻り値修正（エラー→400/404、成功→200）
- [x] C5: 呼び出し側を `reply_tx.send()` → `reply_tx.send_status()` に変更
- [x] C5: `handle_auth_api`（変更不要）、`handle_test_api`（変更不要）の精査完了
- [x] C5: E2E テスト `s17_error_boundary.sh`・`s21_tachibana_error_boundary.sh` の HTTP 200 期待値を 404 に更新。unit test 293 件 PASS
- [x] C1: Section 3-2「Race condition」行を刷新
- [x] C1: Section 5「優先度マトリクス」の `s41` 行を修正
- [x] C2: Section 2-1 テーブルに HTTP 500 パスを全エンドポイント共通 gap として追記
- [x] C4: Section 4-3 末尾に `body_opt_str_field` の null 暗黙等価性を追記
- [x] C6: `kind: null` の仕様を決定（null ≡ omission を正式採用）、docstring 追記 + ユニットテスト 4 件追加
