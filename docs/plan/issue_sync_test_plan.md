# 銘柄同期テスト計画

## 問題

チャートペインで銘柄を切り替えても注文入力パネルに反映されない。

## 特定されたバグ

`order_handler.rs:sync_issue_to_order_entry()` が `panel.update()` の戻り値（`Option<Action>`）を破棄している。

```rust
// BEFORE (バグ): アクションが捨てられる
panel.update(panel::order_entry::Message::SyncIssue { ... });
```

売りモードで新銘柄に切り替えた場合、`Action::FetchHoldings` が返されるべきだが無視される。
その結果、保有株数が取得されずパネルに表示されない。

## テスト戦略

### tests/rust/issue_sync.rs（統合テスト）
- `OrderEntryPanel::update()` の正しい動作を統合テストレベルで検証
- 複数メッセージのシナリオをカバー

### order_handler.rs（ユニットテスト追加）
- `sync_issue_to_order_entry()` のバグ修正を検証

## 修正方針

```rust
// AFTER (修正): アクションを処理してタスクを返す
let pane_id = state.unique_id();
if let Some(panel::order_entry::Action::FetchHoldings { issue_code: code }) =
    panel.update(panel::order_entry::Message::SyncIssue { ... })
{
    tasks.push(Task::perform(
        order_connector::fetch_holdings(code),
        move |result| Message::HoldingsResult { pane_id, result },
    ));
}
```

## 進捗

- [x] バグ特定
- [x] `src/lib.rs` 追加（統合テスト用）
- ✅ `tests/rust/issue_sync.rs` 作成（10テスト）
- ✅ `order_handler.rs` バグ修正（`Task::batch` で FetchHoldings タスクを返す）
- ✅ `cargo test` 確認（10 + 380 = 390テスト全通過）
