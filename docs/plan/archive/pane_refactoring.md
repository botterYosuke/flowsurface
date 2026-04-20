# pane.rs リファクタリング計画

## ゴール・スコープ

`src/screen/dashboard/pane.rs`（2804行）を責務ごとのサブモジュールに分割し、  
単一ファイルの行数を大幅に削減する。既存の動作・テスト・API は変更しない。

## 分割方針と各ファイルの責務

```
src/screen/dashboard/
├── pane/
│   ├── mod.rs      ← State / Message / Event / Status / Action + impl State + tests
│   ├── effect.rs   ← Effect enum（注文・ストリーム副作用の型）
│   ├── content.rs  ← Content enum と impl Content 全メソッド（+Display/PartialEq）
│   └── modal.rs    ← ビューヘルパー関数（link_group_modal / ticksize_modifier / basis_modifier）
└── （既存ファイルはそのまま）
```

### 各ファイルの判断根拠

| ファイル | 移動対象 | 根拠 |
|---------|---------|------|
| `effect.rs` | `Effect` enum（51〜79行） | 副作用の型定義のみ。依存が少なく移動コストが最小 |
| `content.rs` | `Content` enum + `impl Content`（2118〜2599行）| チャート/パネルコンテンツの独立したロジック |
| `modal.rs` | `link_group_modal` / `ticksize_modifier` / `basis_modifier`（2601〜2692行）| モーダル UI レンダリングヘルパー |
| `mod.rs` | 上記以外のすべて | `State` / `Message` / `Event` などの中核型 |

### 残留判断

- `by_basis_default`: `State::set_content_and_streams` で使用 → `mod.rs` に残す
- `virtual_order_from_new_order_request`: `State::update` で使用 → `mod.rs` に残す

## 実装ステップ

- ✅ Step 0: `pane.rs` → `pane/mod.rs` に移動（`pane.rs` 削除）＆ `cargo check` 確認
- ✅ Step 1: `effect.rs` の抽出 → `cargo check`
- ✅ Step 2: `content.rs` の抽出 → `cargo test`
- ✅ Step 3: `controls.rs` の抽出（`modal.rs` は名前衝突のため `controls.rs` に変更）→ `cargo test`
- ✅ Step 4: `mod.rs` 不要 `use` 整理・`cargo clippy -- -D warnings` & `cargo fmt`

## コードレビュー対応（2026-04-20）

- ✅ `PartialEq` 実装に欠落していた `Comparison`・`ShaderHeatmap`・`OrderEntry`・`OrderList`・`BuyingPower` を追加（バグ修正）
- ✅ `ShaderHeatmap` の未初期化表示で `ContentKind::HeatmapChart` → `ContentKind::ShaderHeatmap` に修正
- ✅ `rebuild_content` 内の二重束縛 `let mut new_chart = new_chart` を削除
- ✅ `if !(self.content.kind() == kind)` → `if self.content.kind() != kind` に修正
- ✅ `new_kline` 内の二重ガード（外側で `is_empty()` チェック済みなのに内側で `> 0` チェック）を削除
- ✅ `by_basis_default` をUI非関連のためストリーム選択ロジックとして `mod.rs` に移動

## コードレビュー対応（Round 2 / 2026-04-20）

- ✅ `update_studies()` 内の 3x `expect()` → `let Some(c) = ... else { return; }` に変更（#1）
- ✅ `insert_hist_oi` / `insert_hist_klines` 内の 3x `panic!()` → `log::warn!` + `return` に変更（#2）
- ✅ `set_content_and_streams`: `tickers[0]` を `tickers.first()` ガードに変更、冗長な内側 `base_ticker` バインディングを削除（#3）
- ✅ `toggle_indicator` / `reorder_indicators` 内の `panic!` → `log::warn!` + return（#5）
- ✅ `view()` 内の `self.modal.clone()` → `self.modal.as_ref()` + `*modifier`（Copy）（#6）
- ✅ `MiniTickersListInteraction` 内の冗長 `self.modal = Some(Modal::MiniTickersList(mini_panel.clone()))` を削除（#7）
- ✅ `new_kline` の `unreachable!` → `log::warn!` + Candles フォールバック（#8）
- ✅ `placed_time_ms: 0` → `SystemTime::now()` から取得（#9）
- ✅ `FetchOrderDetail(String, String)` → `FetchOrderDetail { order_num, eig_day }` 名前付きフィールドに変更（#11）

## 作業中の知見・設計判断

- Rust では `impl State` を複数ファイルに分割できないため、modal 制御メソッド
  (`show_modal_with_focus`, `compose_stack_view`) は `mod.rs` に残し、
  `modal.rs` にはそれらから呼ばれる **フリー関数** のみを置く
- `content.rs` の `impl Content` 内では `Effect`, `Action`, `Message` を
  `use super::{Effect, Action, Message, Event};` でインポートする

## 完了条件

- ✅ `cargo build` が通る
- ✅ `cargo test` が通る（356 passed）
- ✅ `cargo clippy -- -D warnings` が通る
- ✅ `cargo fmt` 適用済み
- ✅ 元の `pane.rs` が `pane/mod.rs` に置き換わり、単一ファイルの行数が 2804 → 2186 に削減
- ✅ 計画書に完了の記録がある

## 最終ファイル構成

| ファイル | 行数 | 内容 |
|---------|-----|------|
| `pane/mod.rs` | 2186 | State / Message / Event / Status / Action / impl State / tests |
| `pane/effect.rs` | 32 | Effect enum |
| `pane/content.rs` | 497 | Content enum + impl Content + Display + PartialEq |
| `pane/controls.rs` | 118 | link_group_modal / ticksize_modifier / basis_modifier / by_basis_default |
