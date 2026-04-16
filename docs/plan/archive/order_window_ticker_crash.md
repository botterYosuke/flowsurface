# 注文ウィンドウ「Choose a ticker」削除計画

## 問題の概要

注文ウィンドウ（Order Entry / Order List / Buying Power ペイン）に表示される「Choose a ticker」ボタンをクリックして銘柄を選ぶと、アプリがクラッシュする。

**対応方針**: 注文ペインに「Choose a ticker」ボタンを表示しない。

## 根本原因（デバッグ済み）

```
src\screen\dashboard\pane.rs:427
not yet implemented: order panel content
```

### クラッシュフロー

```
注文ペインの「Choose a ticker」クリック
  → Modal::MiniTickersList を開く
  → ユーザーが銘柄をクリック
  → RowSelection::Switch(ticker_info)
  → Effect::SwitchTickersInGroup(ticker_info)
  → switch_tickers_in_group() → init_focused_pane()
  → state.set_content_and_streams(vec![ticker_info], ContentKind::OrderEntry)
  → match kind { ContentKind::OrderEntry => todo!() } ← PANIC
```

注文ペインはストリームを持たない設計のため、`set_content_and_streams` に渡す経路自体が誤り。

## 修正箇所

修正ファイルは `src/screen/dashboard/pane.rs` の **1ファイルのみ**。

---

### 修正1 — 「Choose a ticker」ボタンの表示条件から注文ペインを除外

**場所**: `pane.rs` の `view_controls` 関数内（L645付近）

現在の条件:
```rust
} else if !matches!(self.content, Content::Starter) && !self.has_stream() {
    // 「Choose a ticker」ボタンを表示
}
```

修正後:
```rust
} else if !matches!(
    self.content,
    Content::Starter | Content::OrderEntry(_) | Content::OrderList(_) | Content::BuyingPower(_)
) && !self.has_stream() {
    // 「Choose a ticker」ボタンを表示
}
```

---

### 修正2 — `ContentSelected` イベントで注文ペインを MiniTickersList に誘導しない

コンテンツ種別を変更した直後に MiniTickersList を自動で開くパスが存在する（L1246付近）。注文ペインへの切り替え時もここを通るため、同様に除外が必要。

現在:
```rust
Event::ContentSelected(kind) => {
    self.content = Content::placeholder(kind);

    if !matches!(kind, ContentKind::Starter) {
        self.streams = ResolvedStream::waiting(vec![]);
        let modal = Modal::MiniTickersList(MiniPanel::new());
        if let Some(effect) = self.show_modal_with_focus(modal) {
            return Some(effect);
        }
    }
}
```

修正後:
```rust
Event::ContentSelected(kind) => {
    self.content = Content::placeholder(kind);

    if !matches!(
        kind,
        ContentKind::Starter
            | ContentKind::OrderEntry
            | ContentKind::OrderList
            | ContentKind::BuyingPower
    ) {
        self.streams = ResolvedStream::waiting(vec![]);
        let modal = Modal::MiniTickersList(MiniPanel::new());
        if let Some(effect) = self.show_modal_with_focus(modal) {
            return Some(effect);
        }
    }
}
```

---

### 修正3（安全策） — `set_content_and_streams` の `todo!()` を `unreachable!()` に昇格

上記2箇所の修正で注文ペインへの経路は塞がれる。`todo!()` を `unreachable!()` に変えることで「ここに来たらコードのバグ」を明示する。

現在:
```rust
ContentKind::OrderEntry
| ContentKind::OrderList
| ContentKind::BuyingPower => {
    // 注文パネルは ticker_info / stream を必要としない
    todo!("order panel content")
}
```

修正後:
```rust
ContentKind::OrderEntry
| ContentKind::OrderList
| ContentKind::BuyingPower => {
    unreachable!("order panes do not use streams — caller must not reach here")
}
```

---

## 副産物: 既修正済みコンパイルエラー

`src/screen/dashboard/panel/order_entry.rs:529` の `text(&side_str)` → `text(side_str.clone())` を本デバッグセッション中に修正済み。

## 実装メモ（2026-04-16）

### TDD 結果

- RED → GREEN → PASS の順で進行。
- 4テストを `pane.rs` 末尾の `#[cfg(test)] mod tests` に追加:
  - `content_selected_order_entry_does_not_open_ticker_modal`
  - `content_selected_order_list_does_not_open_ticker_modal`
  - `content_selected_buying_power_does_not_open_ticker_modal`
  - `content_selected_kline_opens_ticker_modal`（リグレッション確認）

### 設計上の知見

- `Content::placeholder(ContentKind::OrderEntry)` は `todo!()` を含まず正常に動作する（`OrderEntryPanel::new()` を返す）。パニックは `set_content_and_streams` 側のみに存在した。
- 修正2で注文ペインを除外した後、`self.streams` の更新も不要（注文ペインはストリームを使わない設計）。`Content::placeholder` 呼び出しはそのまま残して正しく初期化される。
- `connector::auth::tests::get_session_returns_none_when_no_session_stored` の失敗は本修正と無関係な既存バグ。

### Tips

- 将来「ストリーム不要コンテンツ」が増えた場合、`view_controls` の matches! 条件と `ContentSelected` ハンドラの matches! 条件の両方に追加が必要。メンテナビリティのため `Content::needs_ticker_selector()` ヘルパーの導入を検討価値あり。

## 完了チェックリスト

- ✅ 修正1: `view_controls` の `else if` 条件に注文ペインを追加
- ✅ 修正2: `ContentSelected` ハンドラに注文ペインを追加
- ✅ 修正3: `todo!()` → `unreachable!()`
- ✅ `cargo check` 通過（clippy で確認）
- ✅ `cargo clippy -- -D warnings` 通過
- ✅ `cargo test` 通過（240 PASS、既存バグ 1 FAILED は無関係）
- [ ] E2E: 注文ペインに「Choose a ticker」が表示されないことを確認
- [ ] E2E: チャートペインに「Choose a ticker」が引き続き表示されることを確認（リグレッション確認）
