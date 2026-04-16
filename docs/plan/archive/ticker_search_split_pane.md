# 虫眼鏡: 銘柄+チャート種類選択で Split & 表示

## 目標

虫眼鏡（サイドバーの検索アイコン）からチャート種類付きで銘柄を選択したとき、
フォーカスペインを **上書き変更** するのではなく **Split して新ペインに表示** する。

### 新しいフロー

1. 虫眼鏡を開く（`TickersTable::ToggleTable`）
2. 銘柄をクリックして展開（`ExpandTickerCard`）
3. チャート種類ボタンを押す → `TickerSelected(ticker, Some(kind))`
4. フォーカスペインを Horizontal Split → 新ペインに銘柄+チャート種類を設定
5. フォーカスを新ペインに移動してチャートを表示
   - **フォーカスがない場合**: Split せず虫眼鏡テーブルを閉じる（非活性）

### 変更しないフロー

- 銘柄行を直接クリック（`content = None`）→ `switch_tickers_in_group()` のまま（銘柄だけ変更）

---

## 現状の動作（変更前）

```
TickerSelected(ticker, Some(kind))
  └─ main.rs:808 → init_focused_pane(main_window, ticker_info, kind)
       └─ フォーカスペインのチャート種類と銘柄を上書き
```

関連コード:
- [main.rs:808-874](../../src/main.rs#L808-L874) — `Action::TickerSelected` ハンドラ
- [dashboard.rs:1034-1072](../../src/screen/dashboard.rs#L1034-L1072) — `init_focused_pane()`
- [dashboard.rs:260-272](../../src/screen/dashboard.rs#L260-L272) — `SplitPane` 実装

---

## 変更計画

### Step 1: `dashboard.rs` に `split_focused_and_init()` を追加

`src/screen/dashboard.rs` に新メソッドを追加する。

```rust
/// フォーカスペインを Horizontal Split し、新ペインに ticker_info + content_kind を設定する。
/// Split 成功時は Some(Task)、フォーカスなし・Split 失敗時は None を返す。
pub fn split_focused_and_init(
    &mut self,
    main_window: window::Id,
    ticker_info: TickerInfo,
    content_kind: ContentKind,
) -> Option<Task<Message>> {
    // フォーカスが無ければ単一ペインを自動フォーカス（init_focused_pane と同じ）
    if self.focus.is_none()
        && self.panes.len() == 1
        && let Some((pane_id, _)) = self.panes.iter().next()
    {
        self.focus = Some((main_window, *pane_id));
    }

    let (window, focused_pane) = self.focus?;

    // Split（Horizontal 固定）
    let (new_pane, _) = self.panes.split(
        pane_grid::Axis::Horizontal,
        focused_pane,
        pane::State::new(),
    )?;

    // フォーカスを新ペインへ移動
    self.focus = Some((window, new_pane));

    // 新ペインに銘柄とチャート種類を設定
    let task = self.init_pane(main_window, window, new_pane, ticker_info, content_kind);
    Some(task)
}
```

### Step 2: `main.rs` の `Action::TickerSelected` ハンドラを変更

`src/main.rs` の `content = Some(kind)` 分岐（現在 `init_focused_pane` を呼んでいる部分）を
`split_focused_and_init` に差し替える。

**変更前** (main.rs:821-831):
```rust
let task = {
    if let Some(kind) = content {
        self.active_dashboard_mut().init_focused_pane(
            main_window_id,
            ticker_info,
            kind,
        )
    } else {
        self.active_dashboard_mut()
            .switch_tickers_in_group(main_window_id, ticker_info)
    }
};
```

**変更後**:
```rust
let task = {
    if let Some(kind) = content {
        // Split して新ペインに表示。フォーカスなし時は None → テーブルを閉じる。
        match self
            .active_dashboard_mut()
            .split_focused_and_init(main_window_id, ticker_info, kind)
        {
            Some(t) => t,
            None => {
                // フォーカスなし: 虫眼鏡テーブルを非活性にして終了
                self.sidebar.hide_tickers_table();
                return Task::none();
            }
        }
    } else {
        self.active_dashboard_mut()
            .switch_tickers_in_group(main_window_id, ticker_info)
    }
};
```

---

## 変更ファイル一覧

| ファイル | 変更内容 |
|---|---|
| [src/screen/dashboard.rs](../../src/screen/dashboard.rs) | `split_focused_and_init()` メソッドを追加 |
| [src/main.rs](../../src/main.rs) | `Action::TickerSelected` の `Some(kind)` 分岐を差し替え |

---

## エッジケース

| ケース | 期待動作 |
|---|---|
| フォーカスなし・ペイン数 = 1 | 唯一ペインを自動フォーカス → Split して表示 |
| フォーカスなし・ペイン数 > 1 | テーブルを閉じる（非活性） |
| Split 失敗（ペインが小さすぎ等） | テーブルを閉じる（非活性） |
| `content = None`（行クリック） | 変更なし → `switch_tickers_in_group()` |
| リプレイモード | 既存の `ReloadKlineStream` / `SyncReplayBuffers` chain はそのまま維持 |

---

## 進捗

- ✅ Step 1: `split_focused_and_init()` を追加
- ✅ Step 2: `main.rs` ハンドラを差し替え（Split 時のリプレイ修正も含む）
- ✅ `cargo test` 245 PASS / `cargo clippy` 警告なし
- [ ] 手動動作確認（Live モード）
- [ ] 手動動作確認（フォーカスなし時のテーブル閉じ）
