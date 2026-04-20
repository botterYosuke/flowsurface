# pane/mod.rs リファクタリング計画

## ゴール
`src/screen/dashboard/pane/mod.rs`（2198行）を責務単位に分割し、
単一関数・ファイルの行数を大幅削減する。動作・API・テストは変更しない。

## 分割方針

### Rust の制約
`impl State` は複数ファイルに分割できない。よって State のメソッドは mod.rs に残す。
ただし、メソッド本体を **子モジュールの pub(super) フリー関数** に委譲することで
mod.rs を薄い委譲ラッパーに変える。

### 新ファイル構成
```text
src/screen/dashboard/pane/
├── mod.rs            （State 定義 + impl State の薄いラッパー、~400行）
├── content.rs        （既存: Content enum + impl）
├── controls.rs       （既存: UI ヘルパー）
├── effect.rs         （既存: Effect enum）
├── view.rs           （新規: render_pane / compose_stack_view / view_controls + Content別レンダラー）
├── update.rs         （新規: dispatch / handle_stream_modifier / handle_panel_interaction 等）
└── init.rs           （新規: set_content_and_streams / rebuild_content / insert_hist_klines）
```

### 移動する関数
| 移動元（mod.rs） | 移動先 | 理由 |
| :--- | :--- | :--- |
| `view()` 本体（700行） | `view.rs::render_pane()` | Content別レンダリングを責務分離 |
| `view_controls()` | `view.rs` 内プライベート関数 | view() からのみ呼ばれる |
| `compose_stack_view()` | `view.rs` 内プライベート関数 | view() からのみ呼ばれる |
| `update()` 本体（469行） | `update.rs::dispatch()` | Event処理の責務分離 |
| `virtual_order_from_new_order_request()` | `update.rs` | update() からのみ使用 |
| `set_content_and_streams()` 本体（251行） | `init.rs::set_content_and_streams()` | コンテンツ初期化の責務分離 |
| `rebuild_content()` | `init.rs::rebuild_content()` | 同上 |
| `insert_hist_klines()` | `init.rs::insert_hist_klines()` | 同上 |
| `by_basis_default()` | `init.rs` | init からのみ使用 |

## 実装ステップ
- [x] Step 0: cargo test 全通過を確認（361 PASS）
- [x] Step 1: docs/plan/pane-mod-refactor.md 作成
- [x] Step 2: view.rs 作成（render_pane + compose_stack_view + view_controls + per-content helpers）
- [x] Step 3: update.rs 作成（dispatch + handle_stream_modifier + handle_panel_interaction + handle_mini_tickers_list + virtual_order_from_new_order_request）
- [x] Step 4: init.rs 作成（set_content_and_streams + rebuild_content + insert_hist_klines + by_basis_default）
- [x] Step 5: mod.rs をリファクタリング（薄い委譲ラッパーに変更、~347行）
- [x] Step 6: cargo check / cargo test（361 PASS）/ cargo clippy（警告なし）/ cargo fmt

## 作業中の知見・設計判断

### pub(super) の使い方
- view.rs / update.rs / init.rs は mod.rs の子モジュール（`mod view;` 等で宣言）
- `use super::State` で State にアクセス可能
- State の pub フィールド（modal, content, settings, notifications, streams, status, link_group, is_virtual_mode）は全て子モジュールからアクセス可
- State の `pub fn` メソッドも呼び出せる
- `show_modal_with_focus` は update.rs から呼ぶため `pub(super)` に変更が必要

### view.rs の設計
- `pub(super) fn render_pane(state: &State, id, panes, is_focused, maximized, window, main_window, timezone, tickers_table, is_replay, theme) -> pane_grid::Content`
- Content 別の大きな match アームを helper 関数に分割:
  - `fn render_heatmap_body(state, chart, indicators, id, timezone, modifier, compact_controls, tickers_table) -> (Element, Element)`
  - `fn render_kline_body(...)` 
  - `fn render_shader_heatmap_body(...)`
  - `fn render_ladder_body(...)`
- `compose_stack_view` と `view_controls` はプライベートフリー関数として view.rs に残す

### update.rs の設計
- `pub(super) fn dispatch(state: &mut State, msg: Event) -> Option<Effect>`
- `fn handle_stream_modifier_changed(state: &mut State, message) -> Option<Effect>` (~100行の巨大ハンドラー)
- `fn handle_basis_selected(state: &mut State, modifier, new_basis) -> (Modifier, Option<Effect>)` (BasisSelected アームの分割)
- `fn handle_ticksize_selected(state: &mut State, modifier, tm) -> (Modifier, Option<Effect>)` (TicksizeSelected アームの分割)
- `fn handle_panel_interaction(state: &mut State, msg) -> Option<Effect>`
- `fn handle_mini_tickers_list(state: &mut State, message) -> Option<Effect>`

### init.rs の設計
- `pub(super) fn set_content_and_streams(state: &mut State, tickers, kind) -> Vec<StreamKind>`
- `pub(super) fn rebuild_content(state: &mut State, replay_mode: bool)`
- `pub(super) fn insert_hist_klines(state: &mut State, req_id, timeframe, ticker_info, klines)`
- `fn by_basis_default(basis, default_tf, on_time, on_tick) -> T` (プライベートヘルパー)

## 完了条件
- [x] `docs/plan/pane-mod-refactor.md` が更新されている
- [x] `cargo build` が通る
- [x] `cargo test` が 361 PASS（またはそれ以上）
- [x] `cargo clippy -- -D warnings` が通る
- [x] `cargo fmt` 適用済み
- [x] `mod.rs` が 500 行以下（~347行）
- [x] 各関数が原則 50 行以下（大きな match の dispatch 関数は例外として 100 行まで許容）
