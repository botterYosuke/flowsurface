# Fix: StepBackward — 過去バー（コンテキスト）が表示されない

## 概要

⏮ ボタン押下後、replay start 以前のコンテキストバーが表示されない問題の調査・修正。

## 状況

- **ブランチ**: `sasa/step`
- **日付**: 2026-04-13

---

## 根本原因分析

### Bug 2 修正前（`KlineChart::new()` 呼び出し時）の動作

1. `KlineChart::new()` → 空チャート作成
2. 次の `tick()` で `fetch_missing_data()` が呼ばれる
3. `timeseries.datapoints.is_empty()` = true → 「現在から450本」のフェッチを発行
4. 取引所から歴史的データ（replay start 以前も含む）が返ってくる
5. `insert_hist_klines(req_id, klines)` でデータ挿入（チャート再構築なし）

### Bug 2 修正後（`reset_for_seek()` 呼び出し時）の問題

1. `reset_for_seek()` → `timeseries.datapoints` を空にする（`request_handler` は**未リセット**）
2. **同一イベントハンドラ内で** `ingest_replay_klines(0..new_time+1)` → 5本のリプレイバーを注入
3. 次の `tick()` で `fetch_missing_data()` が呼ばれる
4. `timeseries.datapoints.is_empty()` = **false**（5本ある） → 初期450本フェッチが発火しない
5. `visible_earliest < kline_earliest` チェック: viewport が replay start より前を表示していなければフェッチしない
6. **結果**: コンテキストバーが取得されず、5本のみ表示

### 副次的問題: `request_handler` 未リセット

仮に `visible_earliest < kline_earliest` が成立しても、前回フェッチが30秒以内に完了していれば
`request_handler.add_request()` が `Ok(None)` を返してフェッチを抑制する。

---

## 修正方針: 最終確定（`request_handler` リセット + 自然なコンテキストフェッチ）

`reset_for_seek()` で `request_handler` をリセットし、
`fetch_missing_data()` の既存 `visible_earliest < kline_earliest` 機構に任せる。

### 変更ファイル

| ファイル | 変更内容 |
|---|---|
| `src/chart/kline.rs` | `reset_for_seek()` で `self.request_handler = RequestHandler::default()` 追加 |

---

## 失敗した試み（学習記録）

### Cycle 1-3: `needs_initial_fetch` フラグアプローチ（廃棄）

- `needs_initial_fetch` フラグで `fetch_missing_data()` に「強制フェッチ」を追加
- **問題**: `chrono::Utc::now()`（テスト当日 = April 13）のバーをフェッチするため、`latest_x` が April 13 に更新
- `latest_x` が April 13 になると `visible_timerange()` が April 13 周辺を指す
- ビューポートが April 9 から April 13 に「ジャンプ」する → バーが見えなくなる
- **廃棄理由**: フェッチ時刻が `now` = replay とは無関係の現実時刻

## TDD 作業リスト

### ✅ Cycle: `reset_for_seek` が `request_handler` をリセットする
- ✅ RED: `reset_for_seek_resets_request_handler` テスト作成
- ✅ GREEN: `reset_for_seek()` で `self.request_handler = RequestHandler::default()`

---

## 検証シナリオ

1. アプリ起動 → replay start `2026-04-09 00:00` → 自動再生 5分
2. ⏮ ボタン押下
3. **期待**: replay start 以前のコンテキストバーが表示される（チラつきなし）

---

## 進捗ログ

### 2026-04-13

- 根本原因を特定:
  - `reset_for_seek()` は `request_handler` をリセットしない
  - `ingest_replay_klines` が同一フレームで呼ばれるため、`tick()` 時には既にデータが5本ある
  - `fetch_missing_data()` の `datapoints.is_empty()` ガードを通過できずフェッチ不発
- 修正方針決定: `needs_initial_fetch` フラグ方式（Case A）
- ✅ 計画書作成
- ✅ ログ調査で根本原因特定（`needs_initial_fetch` が `latest_x` を April 13 に更新していた）
- ✅ `needs_initial_fetch` アプローチ廃棄
- ✅ 最終修正: `request_handler = RequestHandler::default()` のみ
- ✅ TDD テスト1本（`reset_for_seek_resets_request_handler`）残存
- ✅ `cargo test`: 150 PASS（1 pre-existing failure）
- ✅ `cargo build --release` 成功

## 最終実装サマリー

変更ファイル: `src/chart/kline.rs` のみ

`reset_for_seek()` に1行追加:
```rust
self.request_handler = RequestHandler::default();
```

## Tips（次の作業者向け）

### なぜ `request_handler` リセットで動くのか

1. `reset_for_seek()` はデータを空にし `latest_x` を保持する
2. `ingest_replay_klines()` でリプレイバーが注入される（`latest_x` は replay 範囲内に留まる）
3. 次 tick の `fetch_missing_data()` が `visible_earliest < kline_earliest` を検出:
   - `visible_earliest` ≈ `latest_x - 90min` (replay start前)
   - `kline_earliest` = replay start (April 9 00:00)
   - → April 8 コンテキストバーをフェッチ
4. `request_handler` リセットにより直前の completed エントリがクリアされ、同一範囲の再フェッチが通る

### なぜ `needs_initial_fetch` は NG だったか

`chrono::Utc::now()` (= April 13) から450本フェッチすると `latest_x` が April 13 に更新される。
`visible_timerange()` は `latest_x` を基準に計算するため、ビューポートが April 13 に移動してしまう。
April 9 のリプレイバーが画面外（4日分左）に吹き飛ぶ。
