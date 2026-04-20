# dashboard.rs リファクタリング計画書

## 概要

`src/screen/dashboard.rs`（2,878行）を機能ドメイン別に分割し、保守性・可読性を向上させる。
外部から見た`Dashboard`の pub API・`Message`・`Event`の型シグネチャを変更しない。

---

## ベースライン確認

- **テスト**: `cargo test` → 371件 PASS（2026-04-20 確認）
- **対象ファイル**: `src/screen/dashboard.rs` 2,878行、104,496バイト
- **既存サブモジュール**: `dashboard/pane/`（7ファイル）、`dashboard/panel/`（5ファイル）

---

## コードマップ（行番号付き）

| 行範囲 | 内容 | 抽出先候補 |
|--------|------|-----------|
| 1–40 | use/pub宣言 | dashboard.rs に残す |
| 42–92 | `Message` enum（注文APIバリアント含む） | dashboard.rs に残す |
| 94–198 | `Dashboard` struct + Default/from_config | dashboard.rs に残す |
| 121–146 | `Event` enum（Replay/Order追加バリアント含む） | dashboard.rs に残す |
| 148–232 | `load_layout`, `update` | dashboard.rs に残す |
| 233–323 | `update()` match本体 | dashboard.rs に残す |
| 325–379 | `handle_pane_message` | `pane_ops.rs` |
| 381–452 | `handle_visual_config_changed`, `visual_config_should_apply` | `pane_ops.rs` |
| 454–566 | `handle_switch_link_group`, `handle_pane_event` | `pane_ops.rs` |
| 569–594 | `handle_request_fetch` | `pane_ops.rs` |
| 597–611 | `order_effect_task` | `effect.rs` |
| 613–688 | `submit_effect_task`, `fetch_effect_task` | `effect.rs` |
| 690–823 | `handle_order_new_result`〜`handle_holdings_result` | `order_handler.rs` |
| 825–878 | `handle_virtual_order_filled`, `eig_day_or_today`, `sync_issue_to_order_entry`, `sync_virtual_mode` | `replay.rs` + `order_handler.rs` |
| 880–1082 | ペイン操作ヘルパー(`new_pane`, `focus_pane`, etc.) | `pane_ops.rs` |
| 1084–1227 | `view()`, `view_window()`, `go_back()`, `handle_error()`, `init_pane()` | dashboard.rs に残す |
| 1229–1460 | `init_focused_pane`, `auto_focus_single_pane`, `split_focused_and_init`, `split_focused_and_init_order`, `initial_buying_power_fetch`, `initial_order_list_fetch`, `switch_tickers_in_group` | `pane_ops.rs` |
| 1462–1945 | `toggle_trade_fetch`, `distribute_fetched_data`, `refresh_streams`, etc. | dashboard.rs に残す（データフロー中心） |
| 1715–1732 | `ingest_replay_klines` | `replay.rs` |
| 1947–1974 | `From<fetcher::FetchUpdate> for Message` | dashboard.rs に残す |
| 1976–2048 | `build_depth_subs`, `build_trade_subs`, `build_kline_subs` | `subscription.rs` |
| 2050–2877 | `#[cfg(test)] mod tests` | 各移行先モジュールに分散 |

---

## 分割方針

### 原則
1. **`pub(super)` で visibility を最小化** — `dashboard.rs` から呼ぶメソッドは `pub(super)`
2. **`impl Dashboard` を複数ファイルに分散** — Rust はファイル分割 `impl` を許可する
3. **段階的移行** — 1モジュールずつ抽出 → `cargo check` → `cargo test` でグリーン確認
4. **テストは移行先モジュールに追従** — 対応するロジックのテストを同ファイルへ

### 移行順序（リスク低い順）

| 順序 | モジュール | 含むメソッド | 理由 |
|------|-----------|------------|------|
| 1 | `subscription.rs` | `build_depth_subs`, `build_trade_subs`, `build_kline_subs` | `Dashboard` 非メンバー・純関数 |
| 2 | `effect.rs` | `order_effect_task`, `submit_effect_task`, `fetch_effect_task` | 外部依存が`pane::Effect`のみ |
| 3 | `order_handler.rs` | `handle_order_new_result`〜`handle_holdings_result`, `eig_day_or_today`, `sync_issue_to_order_entry` | 注文 API 応答処理の集約 |
| 4 | `replay.rs` | `handle_virtual_order_filled`, `sync_virtual_mode`, `ingest_replay_klines` | リプレイ統合ロジック |
| 5 | `pane_ops.rs` | `handle_pane_message`, `handle_visual_config_changed`, `handle_switch_link_group`, `handle_pane_event`, `handle_request_fetch`, ペイン操作ヘルパー全般 | 最も行数が多いがペイン操作に特化 |

---

## 各モジュールの責務と配置

### `src/screen/dashboard/subscription.rs` [NEW]
**責務**: マーケットデータ購読の構築ロジック

```rust
// dashboard.rs から移動（Dashboard 非メンバーの自由関数）
pub(super) fn build_depth_subs(...) -> Option<Subscription<exchange::Event>> { ... }
pub(super) fn build_trade_subs(...) -> Option<Subscription<exchange::Event>> { ... }
pub(super) fn build_kline_subs(...) -> Option<Subscription<exchange::Event>> { ... }
```

移動するテスト:
- `build_depth_subs_returns_none_when_specs_depth_is_empty`
- `build_depth_subs_returns_some_when_specs_depth_is_non_empty`
- `build_trade_subs_*` (3件)
- `build_kline_subs_*` (3件)
- `depth_subs_use_ticker_exchange_not_argument_exchange`
- `market_subscriptions_returns_batch_for_empty_dashboard`

---

### `src/screen/dashboard/effect.rs` [NEW]
**責務**: ペインエフェクト → `Task<Message>` へのマッピング

```rust
impl Dashboard {
    pub(super) fn order_effect_task(effect: pane::Effect, is_replay: bool, pane_id: uuid::Uuid, eig_day: String) -> Task<Message> { ... }
    fn submit_effect_task(effect: pane::Effect, is_replay: bool, pane_id: uuid::Uuid) -> Task<Message> { ... }
    fn fetch_effect_task(effect: pane::Effect, pane_id: uuid::Uuid, eig_day: String) -> Task<Message> { ... }
}
```

> **注意**: `pane/effect.rs`（既存）はペイン側の Effect 定義。本モジュールはダッシュボード側のタスク変換。

---

### `src/screen/dashboard/order_handler.rs` [NEW]
**責務**: 注文 API 応答処理・営業日管理・銘柄同期

```rust
impl Dashboard {
    pub(super) fn handle_order_new_result(...) { ... }
    pub(super) fn handle_order_modify_result(...) { ... }
    pub(super) fn handle_orders_list_result(...) { ... }
    pub(super) fn handle_order_detail_result(...) { ... }
    pub(super) fn handle_buying_power_result(...) { ... }
    pub(super) fn handle_holdings_result(...) { ... }
    pub(super) fn eig_day_or_today(&self) -> String { ... }
    pub(super) fn sync_issue_to_order_entry(...) -> Task<Message> { ... }
}
```

移動するテスト:
- `eig_day_or_today_returns_stored_value_when_set`
- `eig_day_or_today_returns_today_in_yyyymmdd_format_when_not_set`

---

### `src/screen/dashboard/replay.rs` [NEW]
**責務**: リプレイ統合ロジック

```rust
impl Dashboard {
    pub(super) fn handle_virtual_order_filled(fill: FillEvent) -> (Task<Message>, Option<Event>) { ... }
    pub fn sync_virtual_mode(&mut self, main_window: window::Id) { ... }  // pub のまま
    pub fn ingest_replay_klines(&mut self, ...) { ... }                   // pub のまま
}
```

移動するテスト:
- `update_virtual_order_filled_emits_notification`

---

### `src/screen/dashboard/pane_ops.rs` [NEW]
**責務**: ペイン操作ヘルパー全般

```rust
impl Dashboard {
    // handle 系（dashboard.rs の update から呼ばれる）
    pub(super) fn handle_pane_message(...) -> (Task<Message>, Option<Event>) { ... }
    fn handle_visual_config_changed(...) { ... }
    fn visual_config_should_apply(...) -> bool { ... }
    fn handle_switch_link_group(...) -> (Task<Message>, Option<Event>) { ... }
    pub(super) fn handle_pane_event(...) -> (Task<Message>, Option<Event>) { ... }
    fn handle_request_fetch(...) -> Task<Message> { ... }
    
    // ペイン操作
    fn new_pane(...) -> Task<Message> { ... }
    fn focus_pane(...) -> Task<Message> { ... }
    fn split_pane(...) -> Task<Message> { ... }
    fn popout_pane(...) -> Task<Message> { ... }
    fn merge_pane(...) -> Task<Message> { ... }
    
    // ペイン状態ヘルパー
    pub fn all_panes_have_ready_streams(...) -> bool { ... }
    pub fn has_tachibana_stream_pane(...) -> bool { ... }
    pub fn refresh_waiting_panes(...) { ... }
    fn auto_focus_single_pane(...) { ... }
    
    // ペイン初期化
    pub fn split_focused_and_init(...) -> Option<Task<Message>> { ... }
    pub fn split_focused_and_init_order(...) -> Task<Message> { ... }
    pub fn initial_buying_power_fetch(...) -> Task<Message> { ... }
    pub fn initial_order_list_fetch(...) -> Task<Message> { ... }
}
```

移動するテスト（多数）:
- `all_panes_have_ready_streams_*` (2件)
- `refresh_waiting_panes_*` (1件)
- `has_tachibana_stream_pane_*` (2件)
- `split_focused_and_init_*` (6件)
- `split_focused_and_init_order_*` (4件)
- `initial_order_list_fetch_*` (3件)
- `initial_buying_power_fetch_*` (2件)

---

## `dashboard.rs` に残るもの（リファクタ後）

- `pub mod` 宣言（既存 + 新規モジュール）
- `Message` enum
- `Dashboard` struct + `Default` + `from_config`
- `Event` enum
- `load_layout`
- `update()` — match本体（各ハンドラ呼び出しのみ）
- `get_pane`, `get_mut_pane`, `get_mut_pane_state_by_uuid`
- `iter_all_panes`, `iter_all_panes_mut`
- `view()`, `view_window()`
- `go_back()`, `handle_error()`
- `init_pane()`, `init_focused_pane()`
- `toggle_trade_fetch`, `distribute_fetched_data`, `insert_fetched_trades`
- `update_latest_klines`, `ingest_depth`, `ingest_trades`
- `invalidate_all_panes`, `park_for_inactive_layout`, `tick()`
- `resolve_streams`, `market_subscriptions()`, `refresh_streams()`
- `theme_updated`, `peek_kline_streams`, `prepare_replay`
- `clear_chart_for_replay`, `reset_charts_for_seek`, `rebuild_for_live`
- `collect_trade_streams`
- `From<fetcher::FetchUpdate> for Message`
- テスト: `update_*` 系（update 動作確認）・`mini_tickers_list_*`

---

## 各ステップの作業手順

各モジュール抽出時の手順：

1. `src/screen/dashboard/<module>.rs` を新規作成
2. 対象メソッドを `dashboard.rs` からコピー（`Dashboard` の `impl` ブロック内）
3. 必要な `use` 文を新ファイルに追加
4. `dashboard.rs` に `mod <module>;` を追加
5. `dashboard.rs` の元メソッドを削除
6. `cargo check` でコンパイル確認
7. 新ファイルにテストを移動
8. `cargo test` で全グリーン確認
9. `cargo fmt` でフォーマット
10. 計画書の該当ステップに ✅

---

## 進捗管理

### Phase 1: `subscription.rs` 抽出 ✅
- ✅ `subscription.rs` 新規作成（自由関数 3つ移動）
- ✅ `cargo check` グリーン
- ✅ テスト 9件を `subscription.rs` へ移動
- ✅ `cargo test` グリーン

### Phase 2: `effect.rs` 抽出 ✅
- ✅ `effect.rs` 新規作成（`order_effect_task`, `submit_effect_task`, `fetch_effect_task`）
- ✅ `cargo check` グリーン
- ✅ `cargo test` グリーン

### Phase 3: `order_handler.rs` 抽出 ✅
- ✅ `order_handler.rs` 新規作成（注文ハンドラ 6つ + `eig_day_or_today` + `sync_issue_to_order_entry`）
- ✅ `cargo check` グリーン
- ✅ テスト 2件を移動
- ✅ `cargo test` グリーン

### Phase 4: `replay.rs` 抽出 ✅
- ✅ `replay.rs` 新規作成（`handle_virtual_order_filled` + `sync_virtual_mode` + `ingest_replay_klines`）
- ✅ `cargo check` グリーン
- ✅ `cargo test` グリーン

### Phase 5: `pane_ops.rs` 抽出 ✅
- ✅ `pane_ops.rs` 新規作成（ペイン操作ヘルパー全般）
- ✅ `cargo check` グリーン
- ✅ テスト 22件を移動
- ✅ `cargo test` グリーン

### Phase 6: 最終確認 ✅
- ✅ `cargo fmt`
- ✅ `cargo clippy -- -D warnings`（警告・エラーなし）
- ✅ `cargo test` 全371件グリーン
- ✅ 行数確認：`dashboard.rs` 1,263行（目標 1,500行以下 達成）

---

## 制約・注意事項

### Rust のファイル分割 `impl` について
- 複数ファイルに分けた `impl Dashboard` は問題なく動作する
- 各ファイルに `use super::*` または必要な use 文を書く
- `pub(super)` — `dashboard.rs` から呼べるが、外部からは見えない

### visibility 設計
| メソッド | 可視性 | 理由 |
|---------|-------|------|
| `handle_pane_message` | `pub(super)` | `update()` から呼ぶ |
| `handle_order_*` | `pub(super)` | `update()` から呼ぶ |
| `order_effect_task` | `pub(super)` | `handle_pane_event` から呼ぶ |
| `sync_virtual_mode` | `pub` | `main.rs` から呼ぶ |
| `ingest_replay_klines` | `pub` | `main.rs` から呼ぶ |
| `build_*_subs` | `pub(super)` | `market_subscriptions()` から呼ぶ |

### 既存 `pane/effect.rs` との名前衝突回避
- `src/screen/dashboard/pane/effect.rs` = ペイン側 `Effect` 定義
- `src/screen/dashboard/effect.rs` [NEW] = Dashboard 側タスク変換
- `dashboard.rs` で `mod effect;` を宣言すると `pane` モジュール内の `effect.rs` とは異なるパスになるため衝突しない

---

## 知見・Tips（後続作業者向け）

### `impl Dashboard` のファイル分割パターン
```rust
// src/screen/dashboard/order_handler.rs
use super::{Dashboard, Message, panel};  // super = dashboard モジュール
use crate::...;

impl Dashboard {
    pub(super) fn handle_order_new_result(...) { ... }
}
```

### `pub(super)` の適用検討
- `dashboard.rs` の `handle_pane_message`（現在 `fn`）→ `pub(super)` に昇格必要
- `dashboard.rs` の各 private サブ関数も同様

### テスト内 `unwrap()` の扱い
- 既存テスト（371件）には `unwrap()` が23箇所存在するが、テスト内は許容（SKILL.md準拠）
- モジュール移動時はそのまま移動する（改善は別タスク）

### cargo check を常に通す
- 各ステップで `cargo check` を実施し、「コンパイルが通る状態」を維持する
- `cargo test` は時間がかかるため、コンパイル確認を先に行う
