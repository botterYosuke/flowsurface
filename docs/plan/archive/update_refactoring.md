# update() メッセージハンドラ分割リファクタリング計画

## 目的

`src/main.rs` の `update()` 関数（行 332〜1631、約 1300 行）を薄いルーターに分割する。
動作は一切変更しない。

## 現状分析

- `update()` が 33 の `Message` バリアントを直接処理
- `Message::Dashboard`（約 200 行）と `Message::Replay`（約 100 行）が特に肥大化
- API 結果 6 本（`BuyingPowerApiResult` 〜 `FetchHoldingsApiResult`）が同一パターンを繰り返す

## 分割方針

### 1. 共通ヘルパー

```rust
// API result の Ok/Err を JSON 文字列化して reply.send する共通関数
fn send_api_reply(reply: ReplySender, result: Result<serde_json::Value, String>)
```

API 結果 6 アームはそれぞれ `handle_api_*` メソッドにしつつ、
エラーブランチの重複は `serde_json::json!({ "error": e }).to_string()` の1行で統一。

### 2. 抽出するメソッド一覧

| メソッド名 | 対象 Message | 行数 (概算) |
|---|---|---|
| `handle_login` | `Login` | 18 |
| `handle_login_completed` | `LoginCompleted` | 16 |
| `handle_session_restore_result` | `SessionRestoreResult` | 44 |
| `handle_api_buying_power` | `BuyingPowerApiResult` | 14 |
| `handle_api_tachibana_order` | `TachibanaOrderApiResult` | 14 |
| `handle_api_fetch_orders` | `FetchOrdersApiResult` | 22 |
| `handle_api_fetch_order_detail` | `FetchOrderDetailApiResult` | 16 |
| `handle_api_modify_order` | `ModifyOrderApiResult` | 12 |
| `handle_api_fetch_holdings` | `FetchHoldingsApiResult` | 6 |
| `handle_market_ws_event` | `MarketWsEvent` | 45 |
| `handle_tick` | `Tick` | 65 |
| `handle_window_event` | `WindowEvent` | 24 |
| `handle_dashboard_message` | `Dashboard` | 200 |
| `handle_layouts` | `Layouts` | 73 |
| `handle_audio_stream` | `AudioStream` | 17 |
| `handle_theme_editor` | `ThemeEditor` | 16 |
| `handle_network_manager` | `NetworkManager` | 29 |
| `handle_sidebar` | `Sidebar` | 132 |
| `handle_replay` | `Replay` | 98 |
| `handle_replay_api` | `ReplayApi` | 328 |

### 3. update() 完成形イメージ

```rust
fn update(&mut self, message: Message) -> Task<Message> {
    match message {
        Message::Login(msg)                         => self.handle_login(msg),
        Message::LoginCompleted(result)             => self.handle_login_completed(result),
        Message::SessionRestoreResult(result)       => self.handle_session_restore_result(result),
        Message::BuyingPowerApiResult { reply, result }     => self.handle_api_buying_power(reply, result),
        Message::TachibanaOrderApiResult { reply, result }  => self.handle_api_tachibana_order(reply, result),
        Message::FetchOrdersApiResult { reply, result }     => self.handle_api_fetch_orders(reply, result),
        Message::FetchOrderDetailApiResult { reply, result }=> self.handle_api_fetch_order_detail(reply, result),
        Message::ModifyOrderApiResult { reply, result }     => self.handle_api_modify_order(reply, result),
        Message::FetchHoldingsApiResult { reply, result }   => self.handle_api_fetch_holdings(reply, result),
        Message::MarketWsEvent(event)               => self.handle_market_ws_event(event),
        Message::Tick(now)                          => self.handle_tick(now),
        Message::WindowEvent(event)                 => self.handle_window_event(event),
        Message::ExitRequested(windows)             => { self.save_state_to_disk(&windows); iced::exit() }
        Message::SaveStateRequested(windows)        => { self.save_state_to_disk(&windows); Task::none() }
        Message::RestartRequested(windows)          => { self.save_state_to_disk(&windows); self.restart() }
        Message::GoBack                             => { self.handle_go_back(); Task::none() }
        Message::ThemeSelected(theme)               => { self.handle_theme_selected(theme); Task::none() }
        Message::Dashboard { layout_id, event }     => self.handle_dashboard_message(layout_id, event),
        Message::RemoveNotification(index)          => { self.notifications.remove(index); Task::none() }
        Message::SetTimezone(tz)                    => { self.timezone = tz; Task::none() }
        Message::ScaleFactorChanged(value)          => { self.ui_scale_factor = value; Task::none() }
        Message::ToggleTradeFetch(checked)          => { self.handle_toggle_trade_fetch(checked); Task::none() }
        Message::ToggleDialogModal(dialog)          => { self.confirm_dialog = dialog; Task::none() }
        Message::Layouts(message)                   => self.handle_layouts(message),
        Message::AudioStream(message)               => { self.handle_audio_stream(message); Task::none() }
        Message::DataFolderRequested                => { self.handle_data_folder(); Task::none() }
        Message::OpenUrlRequested(url)              => { self.handle_open_url(url); Task::none() }
        Message::ThemeEditor(msg)                   => { self.handle_theme_editor(msg); Task::none() }
        Message::NetworkManager(msg)                => self.handle_network_manager(msg),
        Message::Sidebar(message)                   => self.handle_sidebar(message),
        Message::ApplyVolumeSizeUnit(pref)          => self.handle_apply_volume_size_unit(pref),
        Message::Replay(msg)                        => self.handle_replay(msg),
        Message::ReplayApi((command, reply_tx))     => self.handle_replay_api(command, reply_tx),
    }
}
```

## 実装手順

各ステップで `cargo check` を実行してコンパイルを確認する。
全ステップ完了後に `cargo test` → `cargo clippy -- -D warnings` → `cargo fmt` を実行。

1. ✅ 計画書作成
2. ✅ API result 6 アームの抽出 → `cargo check` ✓
3. ✅ Login / SessionRestore ハンドラ抽出 → `cargo check` ✓
4. ✅ Tick ハンドラ抽出 → `cargo check` ✓
5. ✅ MarketWsEvent ハンドラ抽出 → `cargo check` ✓
6. ✅ **[借用注意]** Dashboard ハンドラ抽出 → `cargo check` ✓（NLL で借用衝突なし）
7. ✅ **[借用注意]** Sidebar ハンドラ抽出 → `cargo check` ✓（借用衝突なし）
8. ✅ Replay ハンドラ抽出 → `cargo check` ✓
9. ✅ ReplayApi ハンドラ抽出（`self.update()` 再帰 → `self.handle_replay()` 直接呼び出しに変更） → `cargo check` ✓
10. ✅ 残りの小アーム（Layouts, ThemeEditor, NetworkManager, GoBack 等）抽出 → `cargo check` ✓
11. ✅ 最終検証 → `cargo test` 356 passed / `cargo clippy` 警告 0 / `cargo fmt` clean

## 完了サマリ

- `update()` 行数: 1300 行 → **93 行**（行 332〜424）
- 抽出メソッド数: 21 メソッド
- 全テスト通過・警告 0・フォーマット clean

## 設計メモ

- `iced` の Elm-like アーキテクチャのため `update()` のシグネチャは変更不可
- `#[cfg(debug_assertions)]` 付きコードは条件コンパイルを維持する
- `Message::Dashboard` ハンドラでは `let Some(active_layout)` の早期 return パターンを維持
- API result ハンドラは `Task<Message>` を返さなくてよい（`reply.send()` のみで完結）が、
  シグネチャを統一するため全ハンドラ `Task<Message>` を返す
- `ReplySender` の実際の型:
  `replay_api::ReplySender` = `Arc<Mutex<Option<oneshot::Sender<(u16, String)>>>>`
  （`replay_api.rs:126`）— ハンドラシグネチャで `replay_api::ReplySender` と参照する

### 借用の注意事項

`handle_dashboard_message` / `handle_sidebar` / `handle_replay` はいずれも
`self.layout_manager` と `self.replay` の両方に触れる。  
これらは**別フィールド**なので分割借用自体は成立するが、`&mut self` を一つのメソッドに
渡した後にさらに別フィールドを借用するコードは Rust コンパイラが弾く場合がある。  
パターン: `let active_id = self.layout_manager.active_layout_id()...` で先に
`layout_id` を `Copy` な値に取り出してから `self.replay.*` へアクセスすれば回避できる。  
各ステップで `cargo check` を回してコンパイルを確認すること。

### `handle_replay_api` の内部構造について

`handle_replay_api` は抽出後も約 328 行と大きい。本リファクタリングの範囲では
「update() から切り出す」に留め、さらなる内部分割は行わない。  
ただし将来の技術的負債になる可能性があるため、以下の内部ヘルパー候補を
設計メモとして記録しておく:
- `handle_replay_api_replay_cmd` — `ApiCommand::Replay(cmd)` アーム
- `handle_virtual_exchange_cmd` — `ApiCommand::VirtualExchange(cmd)` アーム

## Tips

- `update()` 内で `return` を多用しているが、抽出後は各メソッドの最終式で返せばよい
- `Message::ReplayApi` の `reply_replay_status` ローカルクロージャは
  `&self` を借用するので、メソッド内クロージャとして移動可能
- `handle_replay_api` は内部で `self.update()` を再帰呼び出しするアームがあるため、
  `handle_replay` を先に抽出しておくこと（`Toggle`, `Pause`, `Resume`, `StepForward`, `StepBackward`, `CycleSpeed`）
