# kline.rs リファクタリング計画書

作成日: 2026-04-20
対象: `src/chart/kline.rs`（2,115 行）

---

## 現状分析

### ファイル構造（行番号ベース）

| 行範囲 | 責務 |
|---|---|
| L1–27 | `use` インポート |
| L28–121 | `impl Chart for KlineChart` トレイト実装 |
| L123–151 | `impl PlotConstants for KlineChart` トレイト実装 |
| L153–165 | `pub struct KlineChart` 定義 |
| L167–935 | `impl KlineChart` — 状態管理・データ投入・フェッチ制御・リプレイ制御 |
| L937–1142 | `impl canvas::Program<Message> for KlineChart` — draw / update / mouse |
| L1144–1996 | フリー関数・ローカル struct — 描画ロジック |
| L1998–2114 | `#[cfg(test)] mod tests` |

### 混在している責務

1. **KlineChart 状態管理** (`impl KlineChart`): データ挿入・更新・フェッチ制御・リプレイ制御
2. **canvas 描画** (`impl canvas::Program`): draw メソッド → Footprint / Candle の 2 分岐
3. **Footprint 描画** (フリー関数群): `draw_clusters`, `draw_footprint_kline`, `draw_imbalance_markers`, `draw_all_npocs`, `effective_cluster_qty`, `ContentGaps`, `ProfileArea`, `BidAskArea`
4. **共有描画** (フリー関数群): `render_data_source`, `draw_cluster_text`, `draw_crosshair_tooltip`, `should_show_text`
5. **Candle 描画** (フリー関数群): `draw_candle_dp`

---

## 既存テスト状況

現在の `#[cfg(test)] mod tests` に実装済みのテスト：
- ✅ `ingest_historical_klines_inserts_into_timeseries`
- ✅ `reset_for_seek_clears_timeseries`
- ✅ `reset_for_seek_resets_request_handler`

追加依頼されているが未実装のテスト：
- ❌ `replay_mode_suppresses_fetch` — `replay_mode = true` 時に `fetch_missing_data` が `None` を返すこと
- ❌ `bar_count_returns_correct_count` — kline 投入後の `bar_count()` 一致確認

> **注意**: `reset_for_seek_clears_data` は `reset_for_seek_clears_timeseries` として既に実装済みのため、新規追加は実質 2 テスト。

---

## 設計方針

### モジュール分割戦略

```
Rust モジュールシステムのルール：
  src/chart/kline.rs     → kline モジュール（現状）
  src/chart/kline/       → kline サブモジュール群（分割後）
```

Rust では `src/chart/kline.rs` と `src/chart/kline/` を同時に持つことはできない。
**分割後は `kline.rs` → `kline/mod.rs` に移し、サブモジュールを `kline/` 配下に置く。**

### 推奨モジュール構成

```
src/chart/
├── kline/
│   ├── mod.rs        # KlineChart struct + impl 公開 API（元の kline.rs から移行）
│   │                 # impl Chart, impl PlotConstants, impl KlineChart（状態管理）
│   ├── draw.rs       # impl canvas::Program<Message> for KlineChart
│   │                 # + render_data_source（共通），should_show_text，draw_cluster_text，draw_crosshair_tooltip
│   ├── footprint.rs  # Footprint 専用描画: draw_clusters, draw_footprint_kline,
│   │                 # draw_all_npocs, draw_imbalance_markers, effective_cluster_qty
│   │                 # ContentGaps, ProfileArea, BidAskArea（ローカル struct）
│   └── candle.rs     # Candle 専用描画: draw_candle_dp
```

### visibility ルール

- 公開 API（`KlineChart`, `new()` 等）は `pub`（変更なし）
- フリー描画関数は `pub(super)` — 同 `kline` モジュール内でのみ参照
- `ContentGaps`, `ProfileArea`, `BidAskArea` は用途が描画内部に限定されるため `pub(super)` or プライベート
- テストは `kline/mod.rs` 末尾の `#[cfg(test)] mod tests` に残す

---

## 実装フェーズ

### Phase 0: 前提確認（実施前）
- [x] `cargo test` が全通過することを確認
- [x] 既存テストを確認（3テスト実装済み）

### Phase 1: TDD — 新規テスト追加（先にテストを書いてコンパイルを確認）
- [ ] `replay_mode_suppresses_fetch` を追加（現状の `kline.rs` に）
- [ ] `bar_count_returns_correct_count` を追加（現状の `kline.rs` に）
- [ ] `cargo test` でこれらのテストが `PASS` することを確認

### Phase 2: ディレクトリ構造の作成
- [ ] `src/chart/kline/` ディレクトリを作成
- [ ] `src/chart/kline.rs` を `src/chart/kline/mod.rs` にコピー（まず同内容で）
- [ ] `src/chart/kline.rs` を削除（Rust は両方あるとエラー）
- [ ] `cargo build` が通ることを確認

### Phase 3: candle.rs への分割
- [ ] `src/chart/kline/candle.rs` を作成
- [ ] `draw_candle_dp` を `candle.rs` に移動
- [ ] `mod.rs` から `mod candle;` を宣言し `use candle::draw_candle_dp;`
- [ ] `cargo build` + `cargo test` 確認

### Phase 4: footprint.rs への分割
- [ ] `src/chart/kline/footprint.rs` を作成
- [ ] 以下を移動：
  - `draw_footprint_kline`
- [ ] 各ファイルの行数確認（目標: 500 行以下/ファイル）
- [ ] public API シグネチャの不変確認

---

## 注意事項・設計上のTips

### Rust モジュール移動時の `use super::` の書き換え

現在の `kline.rs` は `super::` で親チャートモジュールのシンボルを参照している。
`kline/mod.rs` に移動しても `super::` の参照先は変わらないため修正不要。

しかし `kline/footprint.rs` や `draw.rs` からは親の Chart モジュールのシンボルを参照するために
`use super::super::*` や `use crate::chart::*` が必要になる場合がある。
→ **回避策**: `mod.rs` で必要なシンボルを `pub(super) use` で再エクスポートする。

### canvas::Program impl の分割について

`impl canvas::Program<Message> for KlineChart` を `draw.rs` に移す場合：
- `KlineChart` の定義は `mod.rs` にある
- `impl` ブロックはどのファイルに置いても `use` で型を参照できれば OK（Rust の孤立ルール: 型と trait の少なくとも一方が同一クレート内であれば OK）
- `draw.rs` では `use super::KlineChart;` で参照可能

### テスト配置

テストは `mod.rs`（旧 `kline.rs`）の末尾 `#[cfg(test)] mod tests` に集約する。
サブモジュールごとに `#[cfg(test)]` を作っても良いが、ヘルパー関数（`make_kline`, `build_test_kline_chart`）の重複を避けるため一箇所にまとめる方が保守しやすい。

---

## 完了定義

- [ ] 計画書 `docs/plan/kline-refactor.md` 作成済み ✅（本書）
- [ ] `cargo build` が通る
- [ ] `cargo test` が全て通る（既存 3 + 新規 2 = 5 テスト）
- [ ] `cargo clippy -- -D warnings` がエラーなし
- [ ] `cargo fmt --check` がエラーなし
- [ ] public API のシグネチャに変更なし
- [ ] 1 ファイルあたり 500 行以下（目安）
- [ ] 各モジュールの責務が 1 つに絞られている

---

## 変更ログ

| 日付 | 変更内容 |
|---|---|
| 2026-04-20 | 初版作成・現状分析・モジュール設計 |
