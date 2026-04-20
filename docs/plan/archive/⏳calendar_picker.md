# カレンダーピッカー導入計画

**作成日**: 2026-04-16
**ブランチ**: sasa/develop
**ステータス**: 計画中

---

## 目的

リプレイ範囲の Start / End 日時入力を、テキスト直打ちから**カレンダーピッカー UI** に置き換える。
ユーザーはボタンをクリック → ポップアップカレンダーで日付と時刻を選択 → 確定する操作フローになる。

---

## 前提調査まとめ

| 項目 | 結論 |
|---|---|
| iced バージョン | 0.14.0 — date_picker 標準なし |
| iced_aw | 未導入（追加しない方針）— 最新版は iced 0.12 系対応止まりで 0.14.0 と非互換 |
| popup パターン | `src/modal.rs` の `dashboard_modal()` (`stack!` + `mouse_area`) |
| カスタムウィジェット | `src/widget/` に前例あり（`color_picker.rs`）|
| 日時フォーマット | `"YYYY-MM-DD HH:MM"` — `parse_replay_range` の仕様は変更しない |

---

## UX フロー

```
Replay モード ヘッダー:
  🕐 仮想時刻  [REPLAY]  [2026-04-10 04:49 ▾]  ~  [2026-04-15 06:49 ▾]  ⏮ ▶⏸ ⏭ 1x

  ↓ "2026-04-10 04:49 ▾" をクリック

  ┌────────────────────────────┐
  │  ‹  April 2026  ›          │   ← 月ナビゲーション
  │  Mo Tu We Th Fr Sa Su     │
  │   -  -  1  2  3  4  5    │
  │   6  7  8  9 10 11 12    │   ← 10 が選択中（ハイライト）
  │  13 14 15 16 17 18 19    │
  │  20 21 22 23 24 25 26    │
  │  27 28 29 30  -  -  -    │
  │                            │
  │  時刻: [04] : [49]         │   ← HH / MM テキスト入力
  │              [確定]         │
  └────────────────────────────┘

  画面外クリック → キャンセル（range_input は変更しない）
```

---

## 設計

### ステート設計

現在 `Flowsurface` は `confirm_dialog: Option<ConfirmDialog>` などを直接フィールドで持つ。
カレンダーピッカーも同様に `date_picker: Option<DatePickerState>` をフィールドに追加する。

```rust
// src/main.rs Flowsurface struct に追加
date_picker: Option<DatePickerState>,

// 新規 struct（src/main.rs 内または src/screen/date_picker_state.rs に分離）
struct DatePickerState {
    /// Start を編集中か End を編集中か
    target: PickerTarget,
    /// カレンダーで現在表示している月の1日
    viewing_month: chrono::NaiveDate,
    /// 選択済みの日（確定前の一時値）
    selected_day: Option<chrono::NaiveDate>,
    /// 時刻入力（テキスト、バリデーション前）
    hour_input: String,
    minute_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PickerTarget {
    Start,
    End,
}
```

`DatePickerState` は永続化しない（ポップアップの開閉状態は transient）。

---

### メッセージ設計

`ReplayUserMessage` に追加:

```rust
pub enum ReplayUserMessage {
    // ... 既存 ...

    /// ピッカーを開く（Start or End）
    OpenPicker(PickerTarget),
    /// ピッカーを閉じる（キャンセル）
    ClosePicker,
    /// カレンダーで月をN進める（負は戻る）
    PickerNavMonth(i32),
    /// カレンダーで日付を仮選択
    PickerSelectDay(chrono::NaiveDate),
    /// 時テキスト変更
    PickerHourChanged(String),
    /// 分テキスト変更
    PickerMinuteChanged(String),
    /// 確定: range_input に反映して閉じる
    PickerConfirm,
}
```

`PickerTarget` は `ReplayUserMessage` と同居する `src/replay/mod.rs` に定義する
（メッセージ参照先と型定義を一致させるため）。
`DatePickerState` は transient な UI state なので `src/main.rs` に定義する。

---

### view 設計

#### view_replay_header() の変更点

```rust
// Before: text_input × 2
let start_input = text_input("Start", self.replay.range_input_start())
    .size(11).on_input(...);

// After: button × 2
let start_btn = button(
    row![
        text(self.replay.range_input_start().to_string()).size(11),
        text("▾").size(9),
    ]
    .spacing(4)
)
.on_press(Message::Replay(ReplayMessage::User(ReplayUserMessage::OpenPicker(PickerTarget::Start))))
.padding(padding::all(2).left(6).right(6));
```

`range_input_start()` が空文字列の場合は `"Start"` をプレースホルダーとして表示する。

#### view() でのポップアップ合成

現在の `view()` の flow:

```
base (column![header, replay_header, ...])
  → view_with_modal(base)   ← sidebar メニューが active の場合だけ呼ばれる
    → toast::Manager::new(content)   ← 最終ラップ
```

date_picker ポップアップは `toast::Manager` の直前（= `view_with_modal` と同じ階層）に
挿入する。sidebar メニューと同時に date_picker が開いた場合は **date_picker を優先して閉じる**
（`OpenPicker` 受信時に sidebar メニューを閉じる処理は行わない — 実用上同時操作は起きない）。

```rust
// view() の末尾（toast::Manager の直前）に挿入するイメージ
let content = if let Some(menu) = self.sidebar.active_menu() {
    self.view_with_modal(base.into(), dashboard, menu)
} else {
    base.into()
};

// date_picker ポップアップをここでラップ
let content = if let Some(picker) = &self.date_picker {
    modal::dashboard_modal(
        content,
        widget::date_picker::calendar_popup(picker, ...callbacks...),
        Message::Replay(ReplayMessage::User(ReplayUserMessage::ClosePicker)),
        padding::all(60).top(40),   // NOTE: 目視確認 (Step 6) で要調整
        Alignment::Start,
        Alignment::Start,
    )
} else {
    content
};

toast::Manager::new(content, ...).into()
```

---

### カレンダーウィジェット設計

新規ファイル: `src/widget/date_picker.rs`

iced の基本ウィジェット（`column!`, `row!`, `button`, `text`, `text_input`）のみで構成する。
`decorate` / canvas は使わない（基本グリッドで十分）。

```rust
/// カレンダーポップアップ本体を返す pure view 関数
pub fn calendar_popup<'a, Message: Clone + 'a>(
    viewing_month: chrono::NaiveDate,  // 表示月の1日
    selected_day: Option<chrono::NaiveDate>,
    hour_input: &str,
    minute_input: &str,
    on_nav: impl Fn(i32) -> Message + 'a,      // 月ナビ
    on_select: impl Fn(chrono::NaiveDate) -> Message + 'a,  // 日選択
    on_hour: impl Fn(String) -> Message + 'a,
    on_minute: impl Fn(String) -> Message + 'a,
    on_confirm: Message,
) -> Element<'a, Message>
```

内部構造:
```
container(
  column![
    // 月ナビ行
    row![ button("‹"), text("April 2026"), button("›") ]
    // 曜日ヘッダー行
    row![ text("Mo"), text("Tu"), ... text("Su") ]
    // 日付行 × 最大6週
    row![ day_btn(1), day_btn(2), ... day_btn(7) ]
    ...
    // 時刻入力行
    row![ text_input("HH", hour_input), text(":"), text_input("MM", minute_input) ]
    // 確定ボタン
    button("確定").on_press(on_confirm)
  ]
)
```

#### 日付ボタンのスタイル

- 通常日: `style::button::text` 相当（背景なし）
- 選択中日: `style::button::bordered_toggle(..., true)` 相当（既存の REPLAY ボタンと同パターン）
- 当月外の日（前月末・翌月頭）: 表示しないか薄色

#### 月グリッドの生成ロジック

```rust
fn month_grid(viewing_month: NaiveDate) -> Vec<Vec<Option<u32>>> {
    // viewing_month.weekday() で月初の曜日を計算
    // 各週を [Option<day>; 7] で構成、当月外は None
}
```

`chrono` はすでに依存クレートに含まれているため追加不要。

---

### 確定処理フロー

```
[PickerConfirm メッセージ]
  ├─ picker.hour_input / minute_input を parse (u8)
  │     失敗 or 範囲外 → Toast 通知で中断
  ├─ picker.selected_day が None → Toast 通知で中断
  ├─ NaiveDate + hour + minute → "%Y-%m-%d %H:%M" でフォーマット
  ├─ target == Start → StartTimeChanged(formatted) を dispatch
  │   target == End   → EndTimeChanged(formatted) を dispatch
  └─ date_picker = None（ピッカーを閉じる）
```

バリデーション:
- hour: 0–23
- minute: 0–59
- Start > End になる場合は既存の `parse_replay_range` の `StartAfterEnd` エラーで弾かれる（確定時に Toast）

---

### ピッカー初期値の決め方

```
[OpenPicker(target) メッセージ]
  ├─ 現在の range_input_start/end を parse して NaiveDate を取り出す
  │     parse 成功 → その日付 / 時刻を初期値に
  │     parse 失敗 or 空 → 今日の日付 / 00:00 を初期値に
  └─ date_picker = Some(DatePickerState { target, viewing_month: initial_date, ... })
```

---

## 実装ファイルマップ

| ファイル | 変更種別 | 内容 |
|---|---|---|
| `src/widget/date_picker.rs` | **新規** | `calendar_popup()` 純関数 + `month_grid()` ヘルパー |
| `src/widget.rs` | 修正 | `pub mod date_picker;` を追加 |
| `src/replay/mod.rs` | 修正 | `ReplayUserMessage` に `OpenPicker` / `ClosePicker` / `PickerNav*` 等を追加 |
| `src/main.rs` | 修正 | `Flowsurface` に `date_picker: Option<DatePickerState>` 追加 / `view()` でポップアップ合成 / `view_replay_header()` でボタンに置換 / `update()` でピッカーメッセージ処理 |

---

## 実装ステップ

- [ ] **Step 1**: `src/widget/date_picker.rs` を新規作成
  - `month_grid()` ロジック（chrono で月初曜日を計算）
  - `calendar_popup()` view 関数（基本ウィジェットのみ）
  - `src/widget.rs` に `pub mod date_picker;` 追加
  - `month_grid()` のユニットテストを同ファイルに追加
    - 月初曜日が正しく計算されるか
    - 閏年 2 月（29 日）
    - 通常年 2 月（28 日）
    - 31 日の月（6 週になるケース）
- [ ] **Step 2**: `src/replay/mod.rs` のメッセージ拡張
  - `PickerTarget` enum 追加
  - `ReplayUserMessage` にバリアント追加
- [ ] **Step 3**: `src/main.rs` ステート・ハンドラ実装
  - `DatePickerState` struct 定義
  - `Flowsurface` に `date_picker` フィールド追加
  - `update()` でピッカーメッセージのハンドリング
- [ ] **Step 4**: `src/main.rs` view 変更
  - `view_replay_header()`: text_input → button に置換
  - `view()`: `view_with_modal` / `toast::Manager` の間に date_picker ポップアップを挿入（設計参照）
- [ ] **Step 5**: `cargo check` / `cargo clippy -- -D warnings` / `cargo test`
- [ ] **Step 6**: 目視確認
  - Start / End ボタンクリックでポップアップが開く
  - 日付選択 → 時刻入力 → 確定で range_input に反映される
  - 画面外クリックでキャンセル
  - Live モードではボタン自体が表示されない（前の改修済み）

---

## スコープ外（将来課題）

| 項目 | 理由 |
|---|---|
| キーボード（矢印キー）での日付移動 | 初期実装では不要 |
| 日時テキスト直打ち | ピッカーボタンへの置き換えで意図的に廃止 |
| 秒入力 | `parse_replay_range` が `HH:MM` 固定のため |
| 年跨ぎ直接入力欄 | 月ナビで充足 |
| ピッカーのアニメーション | iced の標準 transition なし |

---

## 完了条件

1. Replay モードのヘッダーに Start / End のピッカーボタンが表示される
2. ボタンクリックでカレンダーポップアップが開く
3. 日付 + 時刻を選択して確定すると range_input に `"YYYY-MM-DD HH:MM"` 形式で反映される
4. 画面外クリックで変更なしにポップアップが閉じる
5. Live モードではピッカーボタンが非表示（前の改修との整合）
6. `cargo clippy -- -D warnings` / `cargo test` が通る
