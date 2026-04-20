# src/main.rs リファクタリング計画

## 目標
3453行の main.rs を責務単位でモジュール分割し、各ファイルを 500行以下にする。

## 分割方針

Rust の同一クレート内での `impl Flowsurface` 複数ファイル分散を利用する。
- `struct Flowsurface`・`enum Message` は `main.rs` に残す
- 各サブモジュールで `use crate::Flowsurface;` して `impl Flowsurface { ... }` を記述

## ファイル構成と責務・移動するメソッド一覧

| ファイル | 責務 | 主なメソッド | 目標行数 |
| :--- | :--- | :--- | :--- |
| `src/main.rs` | struct/enum/main/iced メソッド | `struct Flowsurface`, `enum Message`, `fn main`, `update`, `view`, `subscription`, `theme`, `title`, `scale_factor` | ~440行 |
| `src/app/mod.rs` | 構築・アクセサ | `new`, `active_dashboard`, `active_dashboard_mut` | ~150行 |
| `src/app/view.rs` | UI ヘルパー | `view_replay_header`, `view_with_modal`（settings は modal.rs に委譲） | ~340行 |
| `src/app/modal.rs` | モーダル UI | `build_settings_modal_content`, `build_layout_modal_content` | ~380行 |
| `src/app/handlers.rs` | 汎用メッセージハンドラー | `handle_go_back`, `handle_theme_selected`, `handle_toggle_trade_fetch`, `handle_layouts`, `handle_audio_stream`, `handle_theme_editor`, `handle_network_manager`, `handle_market_ws_event`, `handle_tick`, `handle_window_event`, `handle_login`, `handle_login_completed`, `handle_session_restore_result` | ~420行 |
| `src/app/dashboard.rs` | ダッシュボード系ハンドラー | `handle_dashboard_message`, `handle_sidebar`, `handle_replay` | ~460行 |
| `src/app/persistence.rs` | 永続化・ライフサイクル | `save_state_to_disk`, `load_layout`, `restart`, `transition_to_dashboard`, `start_master_download`, `make_disk_cache_task` | ~295行 |
| `src/app/api/mod.rs` | API ディスパッチ・結果処理 | `handle_replay_api`（薄いディスパッチ）, `handle_auth_api`, `handle_test_api`, `handle_api_buying_power`, `handle_api_tachibana_order`, `handle_api_fetch_orders`, `handle_api_fetch_order_detail`, `handle_api_modify_order`, `handle_api_fetch_holdings` | ~280行 |
| `src/app/api/replay.rs` | Replay/VirtualExchange API | `handle_replay_commands`, `handle_virtual_exchange_commands` | ~260行 |
| `src/app/api/pane.rs` | ペイン操作 API | `handle_pane_api`, `find_pane_handle`, `pane_api_split`, `pane_api_close`, `pane_api_open_order_pane`, `build_notification_list_json`, `build_pane_list_json`, `build_chart_snapshot_json` | ~260行 |
| `src/app/api/pane_ticker.rs` | ペイン ticker/timeframe 操作 | `pane_api_set_ticker`, `pane_api_set_timeframe`, `pane_api_sidebar_select_ticker` | ~400行 |
| `src/app/api/helpers.rs` | 型定義・パースユーティリティ | `KlineStateItem`, `TradeStateItem`, `extract_pane_ticker_timeframe`, `parse_ser_ticker`, `parse_timeframe`, `parse_content_kind`, `resolve_ticker_info` | ~200行 |

## 作業順序

1. ✅ 計画書作成
2. `src/app/api/helpers.rs` — 型・純粋関数（依存最小）
3. `src/app/api/pane.rs` — ペイン操作 API
4. `src/app/api/pane_ticker.rs` — ticker/timeframe 操作
5. `src/app/api/replay.rs` — Replay/VirtualExchange API
6. `src/app/api/mod.rs` — API ディスパッチ
7. `src/app/persistence.rs` — 永続化
8. `src/app/dashboard.rs` — ダッシュボード系ハンドラー
9. `src/app/handlers.rs` — 汎用ハンドラー
10. `src/app/modal.rs` — モーダル UI
11. `src/app/view.rs` — UI ヘルパー
12. `src/app/mod.rs` — new() + アクセサ
13. `src/main.rs` 更新 — iced メソッド + struct + enum + fn main のみに

## 重要な注意点

### 可視性
- サブモジュールの `impl Flowsurface` メソッドのうち、`main.rs` や他モジュールから呼ばれるものは `pub(crate)` にする
- 同一モジュール内のみで使うものは private のまま

### 型の参照
- `Flowsurface`: `use crate::Flowsurface;`
- `Message`: `use crate::Message;`
- `Task<Message>`: `use iced::Task;` + `use crate::Message;`

### view_with_modal の分割
- Settings case の 220行を `build_settings_modal_content(&self)` メソッドとして `modal.rs` に切り出す
- Layout case の 110行を `build_layout_modal_content(&self, dashboard, main_window)` として同じく `modal.rs` に切り出す
- view_with_modal 自体は薄いディスパッチャーになる（~60行）

### handle_replay_api の分割
- ReplayCommand 処理 (~120行) → `api/replay.rs` の `handle_replay_commands`
- VirtualExchangeCommand 処理 (~130行) → `api/replay.rs` の `handle_virtual_exchange_commands`
- 外側ディスパッチ (~80行) → `api/mod.rs` の `handle_replay_api`

## 完了条件
- ✅ `src/main.rs` が 500行以下（421行）
- ✅ 分割後の各ファイルが 500行以下（最大 421行）
- ✅ `cargo build` が通る
- ✅ `cargo test` が通る（356 passed）
- ✅ `cargo clippy -- -D warnings` が通る
- ✅ `cargo fmt --check` が通る
