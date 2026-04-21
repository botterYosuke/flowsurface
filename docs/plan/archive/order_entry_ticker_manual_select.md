# 注文入力 銘柄手動選択への移行計画

## 概要

チャートペインの銘柄切り替えに連動して注文入力の銘柄が自動同期される仕組みを廃止し、
注文入力パネルのタイトルバーに銘柄選択ボタンを設置してユーザーが明示的に選択する方式へ変更する。

---

## 背景・動機

- チャートペインを開いた直後や起動直後は同期が発火しないため「銘柄未選択」のままになる
- チャートペインの銘柄変更が意図せず注文入力に影響する
- 注文入力の銘柄はユーザーが能動的に選ぶべきであり、チャート操作と注文操作は独立させる

---

## 変更方針

| 変更種別 | 内容 |
|---|---|
| 廃止 | チャートペイン銘柄変更 → OrderEntry 自動同期 |
| 追加 | OrderEntry タイトルバーに銘柄選択ボタン（MiniTickersList を開く） |
| 維持 | チャートペイン側の銘柄選択 UI（既存の `mini_tickers_btn`）はそのまま |

---

## 呼び出し経路の全体図

自動同期の経路は 2 本ある。どちらも最終的に同じ関数を呼ぶ。

```
経路A: チャートペイン銘柄変更（MiniTickersList 以外）
  init_focused_pane() [dashboard.rs:571]
    └─ sync_issue_to_order_entry() [order_handler.rs:149]

経路B: MiniTickersList で銘柄選択 → リンクグループ全体を更新
  switch_tickers_in_group() [pane_ops.rs:540]
    └─ init_pane() [dashboard.rs:505]        ← 経路Aと共通の関数
         └─ sync_issue_to_order_entry()

経路C: Effect::SyncIssueToOrderEntry 経由
  handle_pane_event() [pane_ops.rs:239 付近]
    └─ sync_issue_to_order_entry()
```

**経路 B は init_pane() 内を修正するだけで対応できる（Step 1-4 が経路 A・B を両方カバー）。**
経路 C は Effect の生成元を含め別途除去が必要（Step 1-1 〜 1-3）。

---

## 実装ステップ

### Step 1: 自動同期メカニズムの除去

> コンパイルエラーを手掛かりに順番に進める。1-2 を削除するとコンパイルエラーが出るので
> 1-3・1-4 で参照を消してから 1-2 を消す順序でも可。

#### 1-1. `pane/effect.rs` — `SyncIssueToOrderEntry` バリアントを削除

**ファイル**: `src/screen/dashboard/pane/effect.rs:30-34`

```rust
// 削除対象
SyncIssueToOrderEntry {
    issue_code: String,
    issue_name: String,
    tick_size: Option<f64>,
},
```

#### 1-2. `order_handler.rs` — `sync_issue_to_order_entry` 関数を削除

**ファイル**: `src/screen/dashboard/order_handler.rs:149-176`

関数全体を削除する。

#### 1-3. `pane_ops.rs` — Effect ハンドラから `SyncIssueToOrderEntry` 分岐を削除（経路 C）

**ファイル**: `src/screen/dashboard/pane_ops.rs`（`handle_pane_event` 内、行 239 付近）

```rust
// 削除対象
Effect::SyncIssueToOrderEntry { issue_code, issue_name, tick_size } => {
    self.sync_issue_to_order_entry(main_window.id, issue_code, issue_name, tick_size)
}
```

#### 1-4. `dashboard.rs` — `init_pane` / `init_focused_pane` 内の呼び出しを削除（経路 A・B）

**ファイル**: `src/screen/dashboard.rs`

`init_pane`（行 505）の `ticker_changed` ブロックは **sync_task のためだけに存在する**。
削除後は単純に kline_fetch_task を返すだけになる。

```rust
// 削除前（行 522-549）
let ticker_changed = previous_ticker != Some(ticker_info);
if ticker_changed {
    let sync_task = self.sync_issue_to_order_entry(...);
    if !skip_kline_fetch {
        for stream in &streams {
            if let StreamKind::Kline { .. } = stream {
                return Task::batch(vec![kline_fetch_task(...), sync_task]);
            }
        }
    }
    return sync_task;
}
// 削除後（ticker_changed ブロックを丸ごと除去、以降の kline_fetch のみ残す）
```

`init_focused_pane`（行 571）の `ticker_changed` ブロック（行 600-627）も同様に除去する。
ただし `ticker_changed` 時の `state.link_group = None;`（行 591）は **リンクグループ解除の別ロジック** なので残す。

#### 1-5. `panel/order_entry.rs` — `SyncIssue` メッセージと処理を削除

**ファイル**: `src/screen/dashboard/panel/order_entry.rs`

- `Message::SyncIssue { issue_code, issue_name, tick_size }` バリアントを削除（行 210-215）
- `update` 内の `Message::SyncIssue` アームを削除（行 378-395）

---

### Step 2: OrderEntry タイトルバーへ銘柄選択ボタンを追加

#### 2-0. 前提確認と設計決定（実装前に決める）

実コード調査で判明した事項と、それに伴う設計決定：

| 項目 | 実態 | 決定 |
|---|---|---|
| `TickerInfo` の `issue_name` | **フィールド無し**（`ticker`, `min_ticksize`, `min_qty`, `contract_size` のみ） | — |
| 旧 `SyncIssue` の `issue_name` | `init_pane` で `issue_code.clone()` を渡しており **issue_code と同じ文字列**（[dashboard.rs:526-530](src/screen/dashboard.rs#L526)） | **`OrderEntryPanel::issue_name` フィールドを廃止**。UI 表示は `issue_code` のみ使う |
| `OrderEntryPanel::view` のシグネチャ | `view(&self, theme: &Theme, is_replay: bool)`（[order_entry.rs:402](src/screen/dashboard/panel/order_entry.rs#L402)） | **`tickers_table: &'a TickersTable` を引数に追加**。呼び出し元 `pane/view.rs` も修正 |
| `MiniPanel::new()` | 引数なし（[mini_tickers_list.rs:38](src/modal/pane/mini_tickers_list.rs#L38)） | そのまま使用 |
| `tick_size` 型変換 | `MinTicksize` → `f32::from(...)` → `f64` | 旧 `SyncIssue` と同じ変換を移植 |
| MiniPanel の `RowSelection::Add / Remove` | お気に入り追加/削除用で OrderEntry 不要 | 無視 |
| Escape キーでモーダルを閉じる | 他パネルと挙動を揃える必要あり | 既存の MiniTickersList モーダル処理パターンを踏襲（`pane/update.rs` 参照） |

**複数 OrderEntry ペインの挙動変化（破壊的変更）**
旧来は `sync_issue_to_order_entry` が `iter_all_panes_mut` で全 OrderEntry を一斉更新していた。
新方式では **各 OrderEntry パネルが独立して銘柄を持つ**ため、複数パネル配置時は個別に選択する必要がある。

#### 2-1. OrderEntry パネルにモーダル状態を追加

**ファイル**: `src/screen/dashboard/panel/order_entry.rs`

`OrderEntryPanel` 構造体にモーダル表示状態を追加する。

```rust
pub modal: Option<MiniPanel>,  // MiniPanel は既存の MiniTickersList モーダル
```

#### 2-2. `Message` に銘柄選択トリガーと MiniPanel メッセージを追加

```rust
OpenTickerSearch,
MiniTickers(mini_tickers_list::Message),  // MiniPanel への委譲
```

`TickerSelected` は Message には追加しない。MiniPanel の update が返す
`Action::RowSelected(RowSelection::Switch(ticker_info))` を update 内で直接処理する。

#### 2-3. OrderEntry の `view` でタイトルバーに銘柄選択ボタンを追加

**ファイル**: `src/screen/dashboard/panel/order_entry.rs`（`view` メソッド）

`view` のシグネチャを変更する（2-0 の決定事項）:

```rust
// 変更前
pub fn view(&self, theme: &Theme, is_replay: bool) -> Element<'_, Message>
// 変更後
pub fn view<'a>(
    &'a self,
    theme: &'a Theme,
    is_replay: bool,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message>
```

呼び出し元（`src/screen/dashboard/pane/view.rs` 内で OrderEntry の `view` を呼ぶ箇所）にも
`tickers_table` を渡すよう修正する。`tickers_table` は既にチャートペインと同じ経路で
`pane::view` まで伝搬されているので、そこから OrderEntry へ渡すだけでよい。

タイトルバーの左側に「銘柄未選択」または `issue_code` を表示するボタンを配置し、
クリックで `OpenTickerSearch` を発火させる。モーダルが開いている場合は
`modal.view(...)` を overlay として重ねる。

```rust
let issue_label = if self.issue_code.is_empty() {
    text("銘柄未選択").size(13)
} else {
    text(&self.issue_code).size(13)
};
let ticker_btn = button(issue_label)
    .on_press(Message::OpenTickerSearch)
    .style(...);
// MiniPanel の view は Message::MiniTickers にマップ
let overlay = self.modal.as_ref().map(|m| {
    m.view(tickers_table, None, None).map(Message::MiniTickers)
});
```

#### 2-4. MiniPanel のメッセージ処理と銘柄選択結果の反映

`update` 内での処理フロー：

```
Message::OpenTickerSearch
  → self.modal = Some(MiniPanel::new())

Message::MiniTickers(msg)
  → self.modal.as_mut()?.update(msg)
    ├─ None              → 何もしない
    └─ Some(Action::RowSelected(RowSelection::Switch(ticker_info)))
         → issue_code = ticker_info.ticker.symbol_and_exchange_string()
         → tick_size   = f32::from(ticker_info.min_ticksize) as f64
         → holdings リセット、売りモードなら FetchHoldings Action 返却
         → self.modal = None  // モーダルを閉じる
    └─ Some(Action::RowSelected(RowSelection::Add | Remove))
         → OrderEntry では無視
```

**備考：** `issue_name` フィールドは 2-0 の決定により廃止。UI 表示は `issue_code` のみ使う。

---

### Step 3: コンパイル確認・テスト

```bash
cargo check
cargo clippy -- -D warnings
cargo fmt
cargo test
```

#### E2E スモークテスト（HTTP API ポート 9876 経由）

**目的**: 自動同期廃止と手動選択への切替を検証する。

想定シナリオ（`tests/` に追加、`.claude/skills/e2e-testing/SKILL.md` 参照）:

1. `saved-state.json` に TOYOTA チャートがある状態でアプリを起動
2. OrderEntry パネルのタイトルバーが「銘柄未選択」であることを確認
3. チャートペインで銘柄を別銘柄（例: HONDA）に切り替えても OrderEntry は変化しないことを確認
4. OrderEntry の銘柄選択ボタンをクリック → MiniTickersList が開く
5. TOYOTA を選択 → OrderEntry のタイトルバーに TOYOTA が表示される

HTTP API で MiniTickersList の開閉・行選択が操作できるか確認が必要。
未対応なら E2E は手動確認に留める。

---

## 影響範囲

| ファイル | 変更種別 |
|---|---|
| `src/screen/dashboard/pane/effect.rs` | `SyncIssueToOrderEntry` バリアント削除 |
| `src/screen/dashboard/order_handler.rs` | `sync_issue_to_order_entry` 関数削除 |
| `src/screen/dashboard/pane_ops.rs` | Effect ハンドラの分岐削除（経路 C） |
| `src/screen/dashboard.rs` | `init_pane` / `init_focused_pane` の sync ブロック削除（経路 A・B） |
| `src/screen/dashboard/panel/order_entry.rs` | `SyncIssue` 削除 + 銘柄選択ボタン・MiniPanel 管理追加、`view` シグネチャ変更、`issue_name` 廃止 |
| `src/screen/dashboard/pane/view.rs` | OrderEntry `view` 呼び出しに `tickers_table` を渡すよう修正 |

## 注意事項

- `init_focused_pane` の `state.link_group = None;`（行 591）は sync とは無関係なので **残す**
- リンクグループに OrderEntry が含まれていても、銘柄同期は行われなくなる（意図的）
- 旧 `SyncIssue` が設定していた `tick_size` は、新 MiniPanel 選択時に引き続き設定する
  （呼値単位による価格ステップ検証に使用しているため）
- **複数 OrderEntry ペインの独立化（破壊的変更）**: 旧来は全 OrderEntry が一斉同期していたが、
  新方式では各パネルが独立して銘柄を持つ。複数配置時はそれぞれ個別に銘柄選択が必要
- `issue_name` フィールドは廃止。旧コードでは `init_pane` が `issue_code.clone()` を
  両方に渡していたため、実質的な情報損失はない

---

## 進捗

- ✅ Step 1-1: `SyncIssueToOrderEntry` バリアント削除
- ✅ Step 1-2: `sync_issue_to_order_entry` 関数削除
- ✅ Step 1-3: `pane_ops.rs` のハンドラ分岐削除（経路 C）
- ✅ Step 1-4: `dashboard.rs` の sync ブロック削除（経路 A・B、`link_group = None` は残す）
- ✅ Step 1-5: `SyncIssue` メッセージ・処理削除
- ✅ Step 2-0: 前提確認（`issue_name` 廃止・`view` シグネチャ変更を決定）
- ✅ Step 2-1: OrderEntry にモーダル状態追加（`modal: Option<MiniPanel>`、`Default` 実装追加）
- ✅ Step 2-2: `OpenTickerSearch` / `MiniTickers` メッセージ追加
- ✅ Step 2-3: タイトルバーへ銘柄選択ボタン配置・`render_order_entry` で MiniPanel overlay 合成
- ✅ Step 2-4: MiniPanel update 処理・銘柄選択結果の反映（`tick_size` 変換・`holdings` リセット・`FetchHoldings` 含む）
- ✅ Step 3: コンパイル・Clippy（既存 pre-existing 以外ゼロ）・fmt・テスト 383 件全通過
