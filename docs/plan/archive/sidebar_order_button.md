# サイドバー「注文」ボタン追加 計画書

## 概要

注文ウィンドウの表示方法を変更する。

**現状（廃止）**:
1. サイドバーの「Split」でウィンドウ新規作成 → Starter ペイン
2. Starter ペインの pick_list から「OrderEntry」を選択

**新仕様**:
- サイドバーに専用「注文」ボタンを追加
- クリックするとインラインパネルが展開し、3 種類から選択できる
  - **Order Entry** → `ContentKind::OrderEntry`
  - **Order List** → `ContentKind::OrderList`
  - **Buying Power** → `ContentKind::BuyingPower`
- 選択するとフォーカスペインを Horizontal Split して新ペインに選択した種類を直接表示
- **注文パネルとティッカーテーブルは相互排他**（同時に開けない）

---

## サイドバーのボタン配置（変更後）

```
虫眼鏡 (Search)  ← 既存：クリックでティッカーテーブル展開
注文   (Order)   ← 新規：クリックで注文パネル選択リスト展開
レイアウト       ← 既存
音量
───（スペーサー）───
設定
```

注文ボタンを押すと虫眼鏡と同様のトグル動作で以下のインラインパネルが展開する：

```
[ Order Entry  ]
[ Order List   ]
[ Buying Power ]
```

---

## 変更ファイル一覧

| ファイル | 変更種別 | 内容 |
|---|---|---|
| `data/src/config/sidebar.rs` | 修正 | `Menu` enum に `Order` バリアント追加 |
| `data/src/layout/pane.rs` | 修正 | `ContentKind::ALL` から OrderEntry/OrderList/BuyingPower を除外 |
| `src/screen/dashboard/pane.rs` | 修正 | `fn placeholder` を `pub fn placeholder` に変更（中身の追加不要、OrderEntry/OrderList/BuyingPower は処理済み） |
| `src/screen/dashboard/sidebar.rs` | 修正 | `Message::OrderPaneSelected`・`Action::OpenOrderPane` 追加、注文ボタン＋インラインパネル追加、相互排他ロジック追加 |
| `src/screen/dashboard.rs` | 修正 | `fn auto_focus_single_pane` 切り出し・`split_focused_and_init` リファクタ・`split_focused_and_init_order` 追加 |
| `src/main.rs` | 修正 | `sidebar::Action::OpenOrderPane` ハンドラ追加 |

---

## 詳細設計

### 1. `data/src/config/sidebar.rs` — Menu::Order 追加

```rust
// Before
pub enum Menu {
    Layout,
    Settings,
    Audio,
    ThemeEditor,
    Network,
}

// After
pub enum Menu {
    Layout,
    Settings,
    Audio,
    ThemeEditor,
    Network,
    Order,  // ← 新規
}
```

`active_menu` は `#[serde(skip)]` のため永続化に影響なし。

---

### 2. `data/src/layout/pane.rs` — ContentKind::ALL 縮小

#### ContentKind::ALL 変更

```rust
// Before: 11 要素
pub const ALL: [ContentKind; 11] = [
    ContentKind::Starter,
    ContentKind::HeatmapChart,
    ContentKind::ShaderHeatmap,
    ContentKind::FootprintChart,
    ContentKind::CandlestickChart,
    ContentKind::ComparisonChart,
    ContentKind::TimeAndSales,
    ContentKind::Ladder,
    ContentKind::OrderEntry,   // ← 削除（注文ボタン経由のみに限定）
    ContentKind::OrderList,    // ← 削除
    ContentKind::BuyingPower,  // ← 削除
];

// After: 8 要素
pub const ALL: [ContentKind; 8] = [
    ContentKind::Starter,
    ContentKind::HeatmapChart,
    ContentKind::ShaderHeatmap,
    ContentKind::FootprintChart,
    ContentKind::CandlestickChart,
    ContentKind::ComparisonChart,
    ContentKind::TimeAndSales,
    ContentKind::Ladder,
];
```

#### テスト更新

- `content_kind_all_includes_order_variants` テストを削除
- `content_kind_display_covers_all_variants` は引き続き動作（ALL のみ変更）

---

### 3. `src/screen/dashboard/pane.rs` — placeholder 公開

```rust
// Before
fn placeholder(kind: ContentKind) -> Self { ... }

// After
pub fn placeholder(kind: ContentKind) -> Self { ... }
```

`dashboard.rs` から `pane::Content::placeholder(content_kind)` を呼び出せるようにする。
中身（OrderEntry/OrderList/BuyingPower の各アーム）はすでに実装済みのため追加不要。
OrderEntry/OrderList/BuyingPower は TickerInfo が不要なため `set_content_and_streams`
（内部で `tickers[0]` に無条件アクセス）を bypass するためこの経路を使う。

---

### 4. `src/screen/dashboard/sidebar.rs` — 注文ボタン＋インラインパネル追加

#### Message に追加

```rust
pub enum Message {
    ToggleSidebarMenu(Option<sidebar::Menu>),
    SetSidebarPosition(sidebar::Position),
    TickersTable(super::tickers_table::Message),
    OrderPaneSelected(data::layout::pane::ContentKind),  // ← 新規
}
```

#### Action に追加

```rust
pub enum Action {
    TickerSelected(exchange::TickerInfo, Option<data::layout::pane::ContentKind>),
    ErrorOccurred(data::InternalError),
    OpenOrderPane(data::layout::pane::ContentKind),  // ← 新規
}
```

#### update() — 相互排他ロジック追加

注文パネルとティッカーテーブルは同時に開かない。それぞれを開く操作で相手を閉じる。

```rust
Message::ToggleSidebarMenu(menu) => {
    let new_menu = menu.filter(|&m| !self.is_menu_active(m));
    self.set_menu(new_menu);
    // 注文パネルが開いたらティッカーテーブルを閉じる
    if new_menu == Some(sidebar::Menu::Order) {
        self.tickers_table.is_shown = false;
    }
}
Message::TickersTable(msg) => {
    let action = self.tickers_table.update(msg);
    // ティッカーテーブルが開いたら注文パネルを閉じる
    if self.tickers_table.is_shown && self.is_menu_active(sidebar::Menu::Order) {
        self.set_menu(None);
    }
    // ... 既存の action ハンドリング（変更なし）
}
Message::OrderPaneSelected(kind) => {
    self.set_menu(None);  // 注文パネルを閉じる
    return (Task::none(), Some(Action::OpenOrderPane(kind)));
}
```

#### nav_buttons() にボタン追加

```rust
let order_pane_button = {
    let is_active = self.is_menu_active(sidebar::Menu::Order);

    button_with_tooltip(
        icon_text(Icon::Edit, 14)
            .width(24)
            .align_x(Alignment::Center),
        Message::ToggleSidebarMenu(Some(sidebar::Menu::Order)),
        None,
        tooltip_position,
        move |theme, status| crate::style::button::transparent(theme, status, is_active),
    )
};

column![
    ticker_search_button,
    order_pane_button,   // ← 追加
    layout_modal_button,
    audio_btn,
    space::vertical(),
    settings_modal_button,
]
```

#### view() にインラインパネル追加

相互排他が保証されるため、`is_table_open` と `is_order_open` は同時に `true` にならない。

```rust
let is_order_open = self.is_menu_active(sidebar::Menu::Order);

let order_panel = if is_order_open {
    use data::layout::pane::ContentKind;
    column![
        button(text("Order Entry"))
            .on_press(Message::OrderPaneSelected(ContentKind::OrderEntry)),
        button(text("Order List"))
            .on_press(Message::OrderPaneSelected(ContentKind::OrderList)),
        button(text("Buying Power"))
            .on_press(Message::OrderPaneSelected(ContentKind::BuyingPower)),
    ]
    .width(140)
    .spacing(4)
} else {
    column![]
};

match state.position {
    sidebar::Position::Left  => row![nav_buttons, tickers_table, order_panel],
    sidebar::Position::Right => row![order_panel, tickers_table, nav_buttons],
}
.spacing(if is_table_open || is_order_open { 8 } else { 4 })
.into()
```

---

### 5. `src/screen/dashboard.rs` — auto_focus_single_pane 切り出し・split_focused_and_init_order 追加

#### 5-1. `fn auto_focus_single_pane` — プライベートメソッド切り出し

`split_focused_and_init` と `switch_tickers_in_group` に重複していた「フォーカスなし＋単一ペイン時の自動フォーカス」ロジックを切り出す。

```rust
/// フォーカスが無く唯一ペインがある場合、そのペインを自動的にフォーカスする。
fn auto_focus_single_pane(&mut self, main_window: window::Id) {
    if self.focus.is_none()
        && self.panes.len() == 1
        && let Some((pane_id, _)) = self.panes.iter().next()
    {
        self.focus = Some((main_window, *pane_id));
    }
}
```

#### 5-2. `split_focused_and_init` リファクタ（既存メソッド）

インライン展開されていた自動フォーカスロジックをメソッド呼び出しに置き換える。

```rust
pub fn split_focused_and_init(
    &mut self,
    main_window: window::Id,
    ticker_info: TickerInfo,
    content_kind: ContentKind,
) -> Option<Task<Message>> {
    self.auto_focus_single_pane(main_window);  // ← インラインロジックを置き換え

    let (window, focused_pane) = self.focus?;
    // ... 以降は変更なし
}
```

`switch_tickers_in_group` にも同じインラインロジックがあるため、同様に置き換える。

#### 5-3. `split_focused_and_init_order` — 新規メソッド

```rust
/// フォーカスペインを Horizontal Split し、新ペインを指定の注文パネルで初期化する。
/// TickerInfo 不要（SyncIssueToOrderEntry で自動連動）。
/// set_content_and_streams は tickers[0] を必須アクセスするため使用しない。
pub fn split_focused_and_init_order(
    &mut self,
    main_window: window::Id,
    content_kind: data::layout::pane::ContentKind,
) -> Task<Message> {
    self.auto_focus_single_pane(main_window);  // ← 共通メソッド使用

    let Some((window, focused_pane)) = self.focus else {
        return Task::done(Message::Notification(Toast::warn(
            "No focused pane found".to_string(),
        )));
    };

    let Some((new_pane, _)) = self.panes.split(
        pane_grid::Axis::Horizontal,
        focused_pane,
        pane::State::new(),
    ) else {
        return Task::none();
    };

    self.focus = Some((window, new_pane));

    if let Some(state) = self.get_mut_pane(main_window, window, new_pane) {
        state.content = pane::Content::placeholder(content_kind);
    }

    Task::none()
}
```

---

### 6. `src/main.rs` — OpenOrderPane ハンドラ

```rust
Some(dashboard::sidebar::Action::OpenOrderPane(content_kind)) => {
    let main_window_id = self.main_window.id;
    self.active_dashboard_mut()
        .split_focused_and_init_order(main_window_id, content_kind)
        .map(move |msg| Message::Dashboard { layout_id: None, event: msg })
}
```

`SyncReplayBuffers` は発火しない（注文パネルはリプレイバッファを消費しないため意図的省略）。

---

## 影響範囲

- **既存保存レイアウト**: OrderEntry/OrderList/BuyingPower は `Pane` enum に残るため、保存済みレイアウトのデシリアライズに影響なし
- **Starter ペイン**: OrderEntry/OrderList/BuyingPower は `ContentKind::ALL` から除外されるため pick_list に表示されなくなる（注文ボタンが唯一の開き方になる）
- **`Event::ContentSelected(ContentKind::OrderEntry)`**: pane.rs のハンドラは残る（`Content::placeholder()` 経由は引き続き動作）
- **テスト**: `data/src/layout/pane.rs` の `content_kind_all_includes_order_variants` テストを削除

---

## 進捗

- ✅ `data/src/config/sidebar.rs` — `Menu::Order` バリアント追加
- ✅ `data/src/layout/pane.rs` — ContentKind::ALL 変更・テスト削除
- ✅ `src/screen/dashboard/pane.rs` — `fn placeholder` を `pub` に変更
- ✅ `src/screen/dashboard/sidebar.rs` — Message/Action/ボタン/インラインパネル/相互排他ロジック追加
- ✅ `src/screen/dashboard.rs` — `auto_focus_single_pane` 切り出し・`split_focused_and_init` リファクタ・`split_focused_and_init_order` 追加
- ✅ `src/main.rs` — OpenOrderPane ハンドラ追加・`Menu::Order` アーム追加（modal 不要、base をそのまま返す）
- ✅ `cargo check` でコンパイル確認
- ✅ `cargo test -p flowsurface` — 263 件全パス（4 件新規追加）
- ✅ `cargo clippy -- -D warnings` — 警告なし（`replay_api.rs` の既存 clippy 警告も修正）

## コードレビュー対応（2026-04-16）

### Medium 修正済み

**1. `split_focused_and_init_order` のスプリット失敗をサイレントから Toast 通知に変更**
`panes.split()` 失敗時に `Task::none()` ではなく `Toast::warn("Could not split pane")` を返すよう修正。

**2. `virtual_order_from_new_order_request` の side マッピングを排他的に**
`if "3" { Long } else { Short }` → `match` に変更し、未知コードは `log::warn!` + `None` で明示破棄。

---

## 実装メモ（次の作業者への引き継ぎ）

### `Menu::Order` は modal を出さない
`main.rs` の `view_with_menu` は `match menu { ... }` で全バリアントを網羅する。
`Order` は注文パネルをサイドバーのインラインに展開するため、`dashboard_modal` を呼ばず `base` を返す。

### 相互排他の実装場所
`sidebar.rs` の `update()` 内で行う。
- `ToggleSidebarMenu(Some(Order))` → `tickers_table.is_shown = false`
- `TickersTable` メッセージ処理後 → `tickers_table.is_shown && is_menu_active(Order)` なら `set_menu(None)`

### `split_focused_and_init_order` は `Option<Task>` でなく `Task` を返す
`split_focused_and_init` は「フォーカスなし複数ペイン」時に `None` を返し呼び出し側で分岐するが、
`split_focused_and_init_order` は Toast 警告を含む `Task` を返すため `Option` 不要。

### clippy 修正: `replay_api.rs`
`type_complexity`（`ReplySenderInner` type alias 追加）・`collapsible_if`・`manual_split_once`
の 3 件を合わせて修正。これらは本タスクとは無関係の既存問題だったが clippy pass のため対応。
