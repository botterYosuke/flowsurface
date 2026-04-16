# バグ修正依頼: リプレイモードでのペイン銘柄変更が kline を再ロードしない

## プロジェクト概要
flowsurface は Rust 製デスクトップアプリです（iced GUI フレームワーク使用）。
ペインを複数表示できるダッシュボードで、暗号資産の kline チャート等をリプレイ再生できます。

---

## バグの概要

**再現手順**:
1. リプレイモードに入る
2. kline チャートを表示しているペインのヘッダー左上にある銘柄名をクリック
3. 開いた「MiniTickersList」パネルから別の銘柄を選択する

**期待動作**: 新しい銘柄の kline データがリロードされ、チャートが更新される
**実際の動作**: kline が再ロードされず、チャートが古い銘柄のままになる（または空になる）

---

## 原因の特定

銘柄変更には 2 つのパスがあり、動作が異なります。

### ✅ 正常に動作するパス（サイドバー虫眼鏡）
`src/main.rs` の `Message::Sidebar` ハンドラ（約 808 行目）:

```rust
Some(dashboard::sidebar::Action::TickerSelected(ticker_info, content)) => {
    // リプレイ中の旧 kline stream を取得
    let old_kline_streams: Vec<exchange::adapter::StreamKind> =
        if self.replay.is_replay() {
            self.replay.active_kline_streams()
        } else {
            vec![]
        };

    let task = self.active_dashboard_mut()
        .switch_tickers_in_group(main_window_id, ticker_info);

    // kline stream がある場合: ReloadKlineStream（pause → seek → 再ロード）
    // ない場合: SyncReplayBuffers にフォールバック
    let reload_tasks: Vec<Task<Message>> = old_kline_streams
        .into_iter()
        .filter_map(|old| {
            old.as_kline_stream().map(|(_, tf)| {
                Task::done(Message::Replay(ReplayMessage::System(
                    ReplaySystemEvent::ReloadKlineStream {
                        old_stream: Some(old),
                        new_stream: exchange::adapter::StreamKind::Kline {
                            ticker_info,
                            timeframe: tf,
                        },
                    },
                )))
            })
        })
        .collect();

    let replay_task = if reload_tasks.is_empty() {
        Task::done(Message::Replay(ReplayMessage::System(
            ReplaySystemEvent::SyncReplayBuffers,
        )))
    } else {
        Task::batch(reload_tasks)
    };

    return Task::batch([
        task.map(move |msg| Message::Dashboard { layout_id: None, event: msg }),
        replay_task,  // ← kline を pause+seek+再ロード
    ]);
}
```

### ❌ バグがあるパス（ペイン左上クリック → MiniTickersList）
`src/screen/dashboard/pane.rs` 約 1682 行目:
```rust
crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti) => {
    return Some(Effect::SwitchTickersInGroup(ti));
}
```

`src/screen/dashboard.rs` 約 438 行目:
```rust
pane::Effect::SwitchTickersInGroup(ticker_info) => {
    self.switch_tickers_in_group(main_window.id, ticker_info)
    // ← ReloadKlineStream / SyncReplayBuffers が発火しない
}
```

この `(task, None)` が `main.rs` の `Message::Dashboard` ハンドラに渡り、`SyncReplayBuffers` は chain されますが **`ReloadKlineStream` は発火しません**。
`SyncReplayBuffers` だけでは clock の pause・seek・kline 再ロードが行われないため、リプレイ中の kline チャートは更新されません。

---

## 修正方針

`dashboard::Event` に新しいバリアントを追加し、pane の `SwitchTickersInGroup` effect を `main.rs` に伝搬させて、サイドバーパスと同じリプレイ同期ロジックを実行させます。

### Step 1: `dashboard::Event` にバリアント追加
`src/screen/dashboard.rs` の `pub enum Event`（約 116 行目）に追加:
```rust
pub enum Event {
    // ... 既存バリアント ...
    SwitchTickersInGroup {
        ticker_info: TickerInfo,
    },
}
```

### Step 2: dashboard.rs の Effect ハンドラを変更
`src/screen/dashboard.rs` 約 438 行目を変更:
```rust
// 変更前
pane::Effect::SwitchTickersInGroup(ticker_info) => {
    self.switch_tickers_in_group(main_window.id, ticker_info)
}

// 変更後
pane::Effect::SwitchTickersInGroup(ticker_info) => {
    return (
        Task::none(),
        Some(Event::SwitchTickersInGroup { ticker_info }),
    );
}
```

### Step 3: main.rs の Message::Dashboard ハンドラに処理を追加
`src/main.rs` 約 603 行目付近（`Some(dashboard::Event::ReloadReplayKlines {...})` の隣）に追加:
```rust
Some(dashboard::Event::SwitchTickersInGroup { ticker_info }) => {
    let main_window_id = self.main_window.id;

    let old_kline_streams: Vec<exchange::adapter::StreamKind> =
        if self.replay.is_replay() {
            self.replay.active_kline_streams()
        } else {
            vec![]
        };

    let switch_task = self
        .active_dashboard_mut()
        .switch_tickers_in_group(main_window_id, ticker_info)
        .map(move |msg| Message::Dashboard {
            layout_id: Some(layout_id),
            event: msg,
        });

    let reload_tasks: Vec<Task<Message>> = old_kline_streams
        .into_iter()
        .filter_map(|old| {
            old.as_kline_stream().map(|(_, tf)| {
                Task::done(Message::Replay(ReplayMessage::System(
                    ReplaySystemEvent::ReloadKlineStream {
                        old_stream: Some(old),
                        new_stream: exchange::adapter::StreamKind::Kline {
                            ticker_info,
                            timeframe: tf,
                        },
                    },
                )))
            })
        })
        .collect();

    let replay_task = if reload_tasks.is_empty() {
        Task::done(Message::Replay(ReplayMessage::System(
            ReplaySystemEvent::SyncReplayBuffers,
        )))
    } else {
        Task::batch(reload_tasks)
    };

    // switch_task と replay_task は並列実行（.chain() だと
    // 認証待ちタスクが clock.seek をブロックする既知の不具合があるため）
    return Task::batch([switch_task, replay_task]);
}
```

---

## 注意事項
- `Task::batch` を使う理由: `.chain()` だと認証待ち（Tachibana 等）のタスクが長期ブロックした場合に `clock.seek(start)` が実行されない既知の不具合があるため（既存コードのコメント参照）
- `TickerInfo` 型は `exchange::TickerInfo`
- `ReplayMessage`と `ReplaySystemEvent` は既存の import を確認してください
- 修正後は `cargo check` → `cargo clippy -- -D warnings` → `cargo test` で確認してください

---

## 関連ファイル
- `src/main.rs` — Sidebar/Dashboard メッセージハンドラ
- `src/screen/dashboard.rs` — `Event` 定義、`pane::Effect` ハンドラ、`switch_tickers_in_group()`
- `src/screen/dashboard/pane.rs` — `Effect::SwitchTickersInGroup` の発火元
- `src/modal/pane/mini_tickers_list.rs` — `RowSelection::Switch`
```