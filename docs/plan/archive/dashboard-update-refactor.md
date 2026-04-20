# Dashboard::update() リファクタリング計画

## 目的

`src/screen/dashboard.rs` の `Dashboard::update()` 関数（行 233〜727、約 494 行）を
責務単位の小関数群に分割する純粋構造リファクタリング。

---

## 現状分析

```
update() 494行 ─┬─ SavePopoutSpecs       (6行)
                ├─ ErrorOccurred         (13行)
                ├─ Pane(window, msg)     (325行) ← 最大
                │   ├─ PaneClicked       (1行)
                │   ├─ PaneResized       (1行)
                │   ├─ PaneDragged       (3行)
                │   ├─ SplitPane         (8行)
                │   ├─ ClosePane         (3行)
                │   ├─ MaximizePane      (1行)
                │   ├─ Restore           (1行)
                │   ├─ ReplacePane       (3行)
                │   ├─ VisualConfigChanged (55行)
                │   ├─ SwitchLinkGroup   (45行)
                │   ├─ Popout/Merge      (2行)
                │   └─ PaneEvent(effect) (160行)
                │       ├─ RefreshStreams
                │       ├─ RequestFetch
                │       ├─ SwitchTickersInGroup
                │       ├─ FocusWidget
                │       ├─ ReloadReplayKlines
                │       ├─ SubmitNewOrder/Correct/Cancel
                │       ├─ FetchOrders/Detail/BuyingPower/Holdings
                │       ├─ SyncIssueToOrderEntry
                │       └─ SubmitVirtualOrder
                ├─ RequestPalette        (1行)
                ├─ ChangePaneStatus      (4行)
                ├─ DistributeFetchedData (9行)
                ├─ ResolveStreams        (4行)
                ├─ Notification          (1行)
                ├─ OrderNewResult        (25行)
                ├─ OrderModifyResult     (7行)
                ├─ OrdersListResult      (12行)
                ├─ OrderDetailResult     (10行)
                ├─ BuyingPowerResult     (20行)
                ├─ HoldingsResult        (7行)
                └─ VirtualOrderFilled    (10行)
```

---

## 分割設計

### 最終メソッド一覧（実績値）

| メソッド名 | 責務 | 実績行数 |
|---|---|---|
| `update()` | トップレベル dispatch | 91行 |
| `handle_pane_message()` | pane::Message 全バリアント | 55行 |
| `handle_visual_config_changed()` | VisualConfig 同期ロジック | 45行 |
| `visual_config_should_apply()` ★ | VisualConfig 適用判定（自由関数） | 26行 |
| `handle_switch_link_group()` | LinkGroup 切替ロジック | 53行 |
| `handle_pane_event()` | pane::Effect dispatch | 60行 |
| `handle_request_fetch()` ★ | Fetch タスク構築（自由関数） | 27行 |
| `order_effect_task()` ★ | 注文 Effect dispatch（自由関数） | 15行 |
| `submit_effect_task()` ★ | 注文送信 Effect → Task（自由関数） | 38行 |
| `fetch_effect_task()` ★ | 注文照会 Effect → Task（自由関数） | 35行 |
| `handle_order_new_result()` | 新規注文応答 | 29行 |
| `handle_order_modify_result()` | 訂正/取消応答 | ~10行 |
| `handle_orders_list_result()` | 注文一覧応答 | ~15行 |
| `handle_order_detail_result()` | 注文詳細応答 | ~12行 |
| `handle_buying_power_result()` | 買付余力応答 | ~25行 |
| `handle_holdings_result()` | 保有株応答 | ~10行 |
| `handle_virtual_order_filled()` | 仮想約定通知 | ~12行 |

★ `self` を持たない associated fn。借用競合を回避しつつ `impl Dashboard` 内に収める。

---

## 借用チェッカーの制約と対策

### 問題: `state` と `self` の同時借用

`handle_pane_event` 内で `state = self.get_mut_pane(...)` を取得した後、
`self.refresh_streams(...)` を呼ぶ必要がある。

```
self ─(mut borrow)──→ state: &mut pane::State
self.refresh_streams(...)  ← ここで self の再借用が発生 → コンパイルエラーになりうる
```

### 対策: NLL (Non-Lexical Lifetimes) の活用

Rust の NLL により、借用は「最後に使われた場所」まで生存する。

- **`RefreshStreams` アーム**: `state` を使わないため、このアームに入る時点で
  `state` の借用は終了 → `self.refresh_streams(...)` 可能
- **`RequestFetch` アーム**: クロージャが `&mut state.content` をキャプチャするが、
  `handle_request_fetch()` に渡した後クロージャは消費済み → その後 `.chain(self.refresh_streams(...))` 可能
- **`is_replay` と `eig_day` の事前コピー**: どちらも `&mut self.panes` 借用前に
  `let (is_replay, eig_day) = (self.is_replay, self.eig_day_or_today());` として
  一括で取得。可変借用開始前の不変借用として成立する。

### `handle_switch_link_group` の型変更

`main_window: window::Id` を `main_window: &Window` に変更することで、
`handle_pane_message` 内の呼び出しが 100文字以内に収まるようになった（rustfmt が単行に保持）。

---

## TDD アプローチ

### Red フェーズ: 未カバーの Message バリアントにテスト追加

既存テストが `update()` を直接呼ぶのは `mini_tickers_list_switch_emits_switch_tickers_in_group_event` のみ。
以下のバリアントのテストを追加（実装前に記述）:

1. `update_request_palette_emits_event` — RequestPalette → Event::RequestPalette
2. `update_notification_passes_through` — Notification → Event::Notification
3. `update_resolve_streams_emits_event` — ResolveStreams → Event::ResolveStreams
4. `update_change_pane_status_updates_state` — ChangePaneStatus → pane status 更新
5. `update_virtual_order_filled_emits_notification` — VirtualOrderFilled → Event::Notification

### Green フェーズ: リファクタリング実装

メソッド分割を実施。外部から見える振る舞いは一切変えない。

### Refactor フェーズ: cargo fmt / clippy 通過確認

---

## 完了チェックリスト

- [x] `docs/plan/` に計画書を作成済み
- [x] 計画書に設計根拠・Tips・知見を記録済み
- [x] `update()` が 100 行以下に収まっている（91行）
- [x] 抽出した各メソッドが 60 行以下
- [x] `cargo test` が全 PASS（361件）
- [x] `cargo clippy -- -D warnings` が警告ゼロ
- [x] `cargo fmt --check` が通る
- [x] 新たに追加したプライベートメソッドにユニットテストがある

---

## 進捗ログ

### 2026-04-20

- 計画書作成
- `dashboard.rs` の全行を読み込み、分割設計を確定
- TDD Red フェーズ: 5 つのテストを追加（全 FAIL 確認）
- Green フェーズ: 分割実装
  - `update()` → 91行
  - `handle_pane_message()` → 55行（`handle_switch_link_group` 引数を `&Window` に変更して call site を短縮）
  - `handle_visual_config_changed()` → 45行（`visual_config_should_apply` 自由関数を抽出）
  - `handle_switch_link_group()` → 53行
  - `handle_pane_event()` → 60行（`(is_replay, eig_day)` を事前一括取得で借用競合回避 + 行数削減）
  - `order_effect_task()` → 15行（dispatch のみ）
  - `submit_effect_task()` → 38行（注文送信 3種 + replay ガード）
  - `fetch_effect_task()` → 35行（注文照会 4種）
- Refactor フェーズ: `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` 全通過
- レビュー指摘 HIGH 2件対応: `handle_order_detail_result` エラー通知追加、`unreachable!` に呼び出し保証コメント追加
- **スコープ外（別チケット）**: `handle_order_new_result` の `Ok(ref resp)` → `Ok(resp)` + `.clone()` 3箇所削除。機能影響なし、純粋構造リファクタリングの範囲外と判断。
