# ✅ 立花証券 日足データ取得デバッグ

## 背景

`CLMMfdsGetMarketPriceHistory` API で日足データを取得する際、3つの問題があった。

## 修正内容

### 1. p_no カウンターのリセット問題 ✅
- アプリ再起動時に `p_no` が 1 からリセットされ、サーバー側セッションの前回値を下回ると `p_errno: 6` で拒否される
- **修正**: `REQUEST_COUNTER` を `compare_exchange(0, epoch_secs, Relaxed, Relaxed)` で初期化し、常に前回値を超えるようにした。CAS で初期化を排他し、複数スレッドが同時に呼んでも安全
- **ファイル**: `exchange/src/adapter/tachibana.rs` の `next_p_no()`

### 2. serde デシリアライズ失敗 ✅
- エラー応答に `aCLMMfdsGetMarketPriceHistory` フィールドが含まれず、デシリアライズが先に失敗していた
- **修正**: `DailyHistoryResponse` と `MarketPriceResponse` の `records` フィールドに `#[serde(default)]` を追加
- **ファイル**: `exchange/src/adapter/tachibana.rs`

### 3. レスポンスフィールド名の不一致 ✅
- **根本原因**: `DailyHistoryResponse` の `#[serde(rename)]` が `aCLMMfdsGetMarketPriceHistory`（「Get」付き）だったが、実際のAPIレスポンスのフィールド名は `aCLMMfdsMarketPriceHistory`（「Get」なし）
- `#[serde(default)]` により不一致が静かに無視され、空の Vec が返されていた
- **修正**: `aCLMMfdsGetMarketPriceHistory` → `aCLMMfdsMarketPriceHistory` にリネーム
- **ファイル**: `exchange/src/adapter/tachibana.rs` の `DailyHistoryResponse` 構造体 + 全テストの mock レスポンス

## 確認結果

- `cargo test -p flowsurface-exchange`: 全50テスト合格（+2: p_no並行テスト、validate_session未知p_errnoテスト）
- `cargo test --bin flowsurface`: 全20テスト合格（GET→POST + フィールド名修正済み）
- `cargo run` で TOYOTA (7203) を選択: **`Tachibana: fetched 6189 daily klines for 7203`** — 2001年から約25年分の日足データが正常に取得された

## 知見・Tips

- 立花証券 API のレスポンスフィールド名の命名規則:
  - リクエストの `sCLMID`: `CLMMfdsGetMarketPriceHistory`（「Get」付き）
  - レスポンスの配列キー: `aCLMMfdsMarketPriceHistory`（「Get」なし）
  - 同様に時価情報も: リクエスト `CLMMfdsGetMarketPrice` → レスポンス `aCLMMfdsMarketPrice`
- `#[serde(default)]` はデシリアライズ失敗を防ぐために有用だが、フィールド名の不一致を隠蔽してしまうリスクがある。新しい API 型を追加する際は、実際のレスポンスでフィールド名を確認すること。

## 関連ファイル

- `exchange/src/adapter/tachibana.rs` — API アダプター
- `src/connector/fetcher.rs` — fetch_tachibana_daily_klines（テストの `mock("GET")` → `mock("POST")` + フィールド名修正済み）
