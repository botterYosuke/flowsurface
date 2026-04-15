# リプレイ機能 リファクタリング計画書 (Phase 2)

**作成日**: 2026-04-15
**対象ブランチ**: `sasa/develop`
**実装方針**: TDD (Red → Green → Refactor)
**関連仕様書**: [docs/replay_header.md](../replay_header.md)

---

## 目的

現在の `ReplayController` / `ReplayState` が抱える下記4つの設計問題を段階的に解消する。
各フェーズは独立してマージ可能な単位に分割し、既存テストを破壊しないまま進める。

| フェーズ | 調査番号 | タイトル | リスク | 優先度 |
|---|---|---|---|---|
| P1 | 調査3 | `seek_to` メソッドで重複パターンを統一 | 低 | 1 |
| P2 | 調査4 | `play_with_range` 追加・`pending_auto_play` 方針確定 | 中低 | 2 |
| P3 | 調査1 | `ReplaySession` State Machine 導入 | 高 | 3 |
| P4 | 調査2 | `ReplayMessage` 責務分割 | 高 | 4 |

---

## 現状分析

### コードの実際の状態（実装前に確認）

以下はコードリーディングで確認した現在の状態:

| 要素 | 状態 | 場所 |
|---|---|---|
| `handle_range_input_change` | **存在する** | `controller.rs:545-558` |
| `set_range_start` / `set_range_end` | **存在する** | `controller.rs:123-131` |
| `main.rs` の `ReplayCommand::Play` | セッター経由で呼んでいる | `main.rs:937-939` |

### 問題1: seek パターンの重複（P1 の根拠）

`controller.rs` に以下の 4 行セットが **4 箇所** 散在している:

```rust
clock.pause();
clock.seek(target_ms);
dashboard.reset_charts_for_seek(main_window_id);
self.inject_klines_up_to(target_ms, dashboard, main_window_id);
```

| 箇所 | 現状 |
|---|---|
| `StepForward` (Playing ブランチ) | 展開されたまま |
| `StepBackward` (Playing ブランチ) | 展開されたまま |
| `StepBackward` (Paused ブランチ) | 展開されたまま（順序が pause/seek 逆だが等価）|
| `handle_range_input_change` | 展開されたまま |

**例外**: `ReloadKlineStream` は `reset_charts` → ロード → 注入の順序が異なるため対象外。

**例外**: `StepForward` (Paused ブランチ) は pause も chart reset もしない（前進のみ）ため対象外。

現在の `handle_range_input_change` は二重ボローを回避するためにクロックを2回参照している:

```rust
// controller.rs:545-558 (現状)
fn handle_range_input_change(&mut self, ...) {
    let start_ms = self.state.clock.as_ref().map(|c| c.full_range().start); // 不変ボロー
    if let Some(start_ms) = start_ms {
        if let Some(clock) = &mut self.state.clock {                         // 可変ボロー
            clock.pause();
            clock.seek(start_ms);
        }
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(start_ms, dashboard, main_window_id);
    }
}
```

`seek_to` を導入するとこれは以下のようにシンプルになる:

```rust
fn handle_range_input_change(&mut self, ...) {
    if let Some(start_ms) = self.state.clock.as_ref().map(|c| c.full_range().start) {
        self.seek_to(start_ms, dashboard, main_window_id);
    }
}
```

### 問題2: `play_with_range` の欠落（P2 の根拠）

`ReplayCommand::Play` 受信時に `main.rs` が range 設定 + Play の2ステップを踏んでいる:

```rust
// main.rs:937-939 (現状)
self.replay.set_range_start(start);
self.replay.set_range_end(end);
let task = self.update(Message::Replay(ReplayMessage::Play));
```

これは `ReplayController` の単一メソッドに集約できる。

`pending_auto_play` の `StartupOrchestrator` への外部化は **本計画書全体を通じて非目標**とする。
理由: Dashboard API の設計に依存するため独立したタスクとして扱う。

### 問題3: 不正状態が型で防げない（P3 の根拠）

`clock=None` なのに `event_store`/`active_streams` が残留できる。
`DataLoadFailed` バグの根本原因。`reset_session()` で手動リセットしているが型の保証がない。

`try_resume_from_waiting` の全 stream スキャン (`event_store.is_loaded` × n) も
`pending_count` カウンタで O(1) に改善できる。

### 問題4: ReplayMessage の混在（P4 の根拠）

UI 操作・非同期応答・システムイベントが同一 enum に混在。
`handle_message` の肥大化の根本原因。分割により各ハンドラのシグネチャが単純になる。

---

## P1: `seek_to` メソッド統一

### 変更ファイル

- `src/replay/controller.rs`

### 実装する振る舞い

```rust
impl ReplayController {
    /// Pause → Seek → ChartReset → KlineInject を一括実行する。
    /// StepForward/StepBackward (Playing 時)、StepBackward (Paused 時)、
    /// および handle_range_input_change から呼ぶ。
    ///
    /// # 対象外
    /// - `ReloadKlineStream`: reset_charts → ロード → 注入の順序が異なる
    /// - `StepForward` (Paused 時): pause も chart reset も不要（前進のみ）
    fn seek_to(
        &mut self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) {
        if let Some(clock) = &mut self.state.clock {
            clock.pause();
            clock.seek(target_ms);
        }
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(target_ms, dashboard, main_window_id);
    }
}
```

### TDD サイクル

#### RED: 書くテスト

`seek_to` は private だが、同一ファイルの `#[cfg(test)] mod tests` からアクセスできる。

```rust
// controller.rs #[cfg(test)] mod tests に追加

/// seek_to を呼ぶと clock が Paused になること
#[test]
fn seek_to_pauses_clock() {
    let mut ctrl = make_playing_controller();
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    ctrl.seek_to(END_MS, &mut dashboard, win);

    assert_eq!(
        ctrl.state.clock.as_ref().unwrap().status(),
        ClockStatus::Paused,
        "seek_to must pause the clock"
    );
}

/// seek_to を呼ぶと now_ms が target_ms にスナップされること
#[test]
fn seek_to_positions_clock_at_target() {
    let mut ctrl = make_playing_controller();
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    ctrl.seek_to(END_MS, &mut dashboard, win);

    assert_eq!(ctrl.state.current_time(), END_MS);
}

/// seek_to で range.start を渡したとき now_ms が start になること
#[test]
fn seek_to_range_start_resets_position() {
    let mut ctrl = make_playing_controller();
    // clock を中間まで進める
    {
        let clock = ctrl.state.clock.as_mut().unwrap();
        clock.seek(START_MS + STEP_MS);
    }
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    ctrl.seek_to(START_MS, &mut dashboard, win);

    assert_eq!(ctrl.state.current_time(), START_MS);
}
```

**実行**:
```bash
cargo test -p flowsurface seek_to
# FAIL (メソッド未定義) を確認
```

#### GREEN: 最小実装

1. `seek_to` private メソッドを追加する
2. `StepForward` の Playing ブランチを `seek_to(end_ms, ...)` に置き換える
3. `StepBackward` の Playing ブランチを `seek_to(start_ms, ...)` に置き換える
4. `StepBackward` の Paused ブランチを `seek_to(new_time, ...)` に置き換える
   （現在は `seek → pause` の順なので `seek_to` の `pause → seek` に合わせて整理する）
5. `handle_range_input_change` を `seek_to` 呼び出しに書き換える

```bash
cargo test -p flowsurface
# 全テスト PASS を確認
```

#### REFACTOR

- `handle_range_input_change` の二重ボロー解消を確認
- `seek_to` の doc comment に対象外ケースを明記（`ReloadKlineStream`、`StepForward` Paused）

### 完了条件

- [ ] `seek_to` メソッドが追加され private に保たれている
- [ ] `StepForward` (Playing) / `StepBackward` (Playing + Paused) / `handle_range_input_change` が `seek_to` を利用している
- [ ] `ReloadKlineStream` と `StepForward` (Paused) は `seek_to` を使っていない（doc comment で理由を明記）
- [ ] 新規テスト 3 件 PASS
- [ ] 既存テスト全 PASS
- [ ] `cargo clippy -- -D warnings` エラーなし

---

## P2: `play_with_range` 追加・`pending_auto_play` 方針確定

### 変更ファイル

- `src/replay/controller.rs`（`play_with_range` 追加）
- `src/replay/mod.rs`（`pending_auto_play` コメント追加）
- `src/main.rs`（`ReplayCommand::Play` の処理のみ）

### 実装する振る舞い

#### P2-A: `play_with_range` メソッド追加

```rust
impl ReplayController {
    /// API コマンド `ReplayCommand::Play { start, end }` の処理を一括実行する。
    /// range_input を更新してから ReplayMessage::Play を処理する。
    /// `main.rs` の set_range_start + set_range_end + update の3ステップを1メソッドに集約。
    pub fn play_with_range(
        &mut self,
        start: String,
        end: String,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        self.state.range_input.start = start;
        self.state.range_input.end = end;
        self.handle_message(ReplayMessage::Play, dashboard, main_window_id)
    }
}
```

`main.rs` の `ReplayCommand::Play` ハンドラを `play_with_range` 1 行に書き直す:

```rust
// main.rs 変更後
ReplayCommand::Play { start, end } => {
    let task = self.replay.play_with_range(
        start, end, &mut dashboard, main_window_id,
    );
    reply_tx.send(reply_replay_status(self));
    return task;
}
```

#### P2-B: `pending_auto_play` の方針確定

`pending_auto_play` は `ReplayState` に残す。本計画書では外部化しない。

`ReplayState` のフィールド定義にコメントを追加する:

```rust
/// 起動フロー固有のフラグ。再生ロジック本体（clock / session）とは無関係。
/// saved-state.json から Replay 構成が復元されたとき、全ペイン Ready になった瞬間に
/// 自動 Play を発火するために使う。一度発火したら false に戻す（永続化しない）。
/// NOTE: StartupOrchestrator への外部化は Dashboard API 設計依存のため別タスクとする。
pending_auto_play: bool,
```

### TDD サイクル

#### RED: 書くテスト

```rust
// controller.rs #[cfg(test)] mod tests に追加

/// play_with_range を呼ぶと range_input が更新されること
#[test]
fn play_with_range_updates_range_input() {
    let mut ctrl = ReplayController::default();
    ctrl.state.mode = ReplayMode::Replay;
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    let _ = ctrl.play_with_range(
        "2025-01-01 00:00".to_string(),
        "2025-01-02 00:00".to_string(),
        &mut dashboard,
        win,
    );

    assert_eq!(ctrl.state.range_input.start, "2025-01-01 00:00");
    assert_eq!(ctrl.state.range_input.end, "2025-01-02 00:00");
}

/// play_with_range の結果が set_range_start + set_range_end + handle_message(Play) と等価であること
/// (main.rs の3ステップ削除による副作用なし)
#[test]
fn play_with_range_equivalent_to_set_then_play() {
    let make = || {
        let mut ctrl = ReplayController::default();
        ctrl.state.mode = ReplayMode::Replay;
        ctrl
    };

    let mut ctrl_combined = make();
    let mut ctrl_separate = make();
    let mut dash1 = Dashboard::default();
    let mut dash2 = Dashboard::default();
    let win = window::Id::unique();

    let _ = ctrl_combined.play_with_range(
        "2025-01-01 00:00".to_string(),
        "2025-01-02 00:00".to_string(),
        &mut dash1,
        win,
    );

    // 既存の set_range_start / set_range_end (controller.rs:123-131) を使う
    ctrl_separate.set_range_start("2025-01-01 00:00".to_string());
    ctrl_separate.set_range_end("2025-01-02 00:00".to_string());
    let _ = ctrl_separate.handle_message(ReplayMessage::Play, &mut dash2, win);

    assert_eq!(
        ctrl_combined.state.range_input.start,
        ctrl_separate.state.range_input.start,
    );
    assert_eq!(
        ctrl_combined.state.range_input.end,
        ctrl_separate.state.range_input.end,
    );
}
```

**実行**:
```bash
cargo test -p flowsurface play_with_range
# FAIL (play_with_range 未定義) を確認
```

#### GREEN: 最小実装

`play_with_range` を追加し、`main.rs` の `ReplayCommand::Play` ブロックを書き換える。

```bash
cargo test -p flowsurface
```

#### REFACTOR

`main.rs` の `ReplayCommand::Play` が1行になることを確認し、
`play_with_range` の doc comment に「`set_range_start` / `set_range_end` は引き続き
単独利用可能として公開したまま残す」を明記する。

### 完了条件

- [ ] `play_with_range` が `ReplayController` に追加されている
- [ ] `main.rs` の `ReplayCommand::Play` ブロックが `play_with_range` を使っている
- [ ] `pending_auto_play` に起動フロー専用・外部化は別タスクの旨のコメントが付いている
- [ ] 新規テスト 2 件 PASS
- [ ] 既存テスト全 PASS

---

## P3: `ReplaySession` State Machine 導入

### 変更ファイル

- `src/replay/mod.rs`（`ReplayState` に `session` フィールドを追加、`ReplaySession` enum 追加）
- `src/replay/controller.rs`（全フィールドアクセスと `tick()` を更新）

### 設計

```rust
/// リプレイセッションの状態機械。
/// 不正状態（clock が None なのにデータが残留する）を型で排除する。
pub enum ReplaySession {
    /// クロックなし・データなし（初期状態 / DataLoadFailed 後）
    Idle,
    /// 全 stream のロード完了待ち
    Loading {
        clock: StepClock,
        /// あと何本のロード完了を待つか（O(1) チェック。旧 try_resume_from_waiting の代替）
        pending_count: usize,
        store: EventStore,
        active_streams: HashSet<StreamKind>,
    },
    /// 再生可能状態（Paused / Playing / Waiting は ClockStatus で区別）
    Active {
        clock: StepClock,
        store: EventStore,
        active_streams: HashSet<StreamKind>,
    },
}
```

`ReplayState` の `clock: Option<StepClock>` + `event_store` + `active_streams` が
`session: ReplaySession` に置き換わる。
`mode` / `range_input` / `pending_auto_play` はそのまま残す。

#### 遷移表

| From | Event | To | 備考 |
|---|---|---|---|
| `Idle` | `Play` 押下 (kline あり) | `Loading { pending_count=n }` | n = kline_targets の数 |
| `Idle` | `Play` 押下 (kline なし) | `Active` | 即 Playing に移行 |
| `Loading` | `KlinesLoadCompleted` (pending_count → 0) | `Active` | `pending_count` がゼロで遷移 |
| `Loading` | `KlinesLoadCompleted` (pending_count > 0) | `Loading` (同 variant, カウンタ減算のみ) | |
| `Loading` | `DataLoadFailed` | `Idle` | |
| `Active` | `ReloadKlineStream` | `Loading { pending_count=1 }` | |
| `Active` / `Loading` | `ToggleMode` (→ Live) | `Idle` | |

#### `DataLoadFailed` 後の遅延 `KlinesLoadCompleted` の扱い

`pending_count=2` の `Loading` 状態で 1 本目が `DataLoadFailed` → `Idle` に遷移した後、
2 本目の非同期タスクが `KlinesLoadCompleted` を返した場合:
セッションが `Idle` なので `Loading { .. }` パターンにマッチせず、イベントは**無視される**。
これは意図的なサイレントドロップであり、問題ない（`Idle` 状態では古いデータを受け入れない）。

#### `std::mem::replace` を使った所有権移動パターン

`enum` の variant を別 variant に置き換える際、borrow checker を通すために
2ステップアプローチを取る:

```rust
// ── Loading → Active (KlinesLoadCompleted で pending_count がゼロになったとき) ──

// Step 1: ミュータブルボローで内部を更新し、遷移すべきかを bool で返す
let should_activate = if let ReplaySession::Loading {
    pending_count, store, clock, ..
} = &mut self.state.session
{
    store.ingest_loaded(stream, range, LoadedData { klines: klines.clone(), trades: vec![] });
    *pending_count = pending_count.saturating_sub(1);
    if *pending_count == 0 {
        clock.resume_from_waiting(Instant::now());
        true
    } else {
        false
    }
} else {
    false  // Idle なら DataLoadFailed 後の遅延到着 → 無視
};

// Step 2: ボローが解放されてから mem::replace で所有権を取り出して遷移
if should_activate {
    let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
    if let ReplaySession::Loading { clock, store, active_streams, .. } = old {
        self.state.session = ReplaySession::Active { clock, store, active_streams };
    }
}
```

```rust
// ── Active → Loading (ReloadKlineStream) ──

// Step 1: 必要な値を先に取り出し、ミュータブルボローで状態を変更する
let transition_data = if let ReplaySession::Active { clock, store: _, active_streams } =
    &mut self.state.session
{
    clock.pause();
    if let Some(old) = old_stream { active_streams.remove(&old); }
    active_streams.insert(new_stream);
    let step_ms = min_timeframe_ms(active_streams);
    clock.set_step_size(step_ms);
    let start_ms = clock.full_range().start;
    let end_ms = clock.full_range().end;
    clock.seek(start_ms);
    Some((start_ms, end_ms))
} else {
    None  // クロックなし → no-op
};

let Some((start_ms, end_ms)) = transition_data else {
    return (Task::none(), None);
};

// Step 2: 所有権を移動して Loading に遷移
let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
if let ReplaySession::Active { clock, store, active_streams } = old {
    self.state.session = ReplaySession::Loading {
        clock,
        store,
        active_streams,
        pending_count: 1,
    };
}

// Step 3: チャートリセット + ロードタスク発行（遷移後）
dashboard.reset_charts_for_seek(main_window_id);
let stream_step_ms = new_stream.as_kline_stream()
    .map(|(_, tf)| tf.to_milliseconds())
    .unwrap_or_else(|| min_timeframe_ms(&self.active_streams_ref()));
let range = super::compute_load_range(start_ms, end_ms, stream_step_ms);
let task = Task::perform(loader::load_klines(new_stream, range), |result| match result {
    Ok(r) => ReplayMessage::KlinesLoadCompleted(r.stream, r.range, r.klines),
    Err(e) => ReplayMessage::DataLoadFailed(e),
});
(task, None)
```

#### `tick()` の更新

現在の `controller.rs:tick()`:

```rust
// 変更前
let Some(clock) = &mut self.state.clock else {
    return TickOutcome { trade_events: vec![], reached_end: false };
};
let dispatch = dispatcher::dispatch_tick(
    clock, &self.state.event_store, &self.state.active_streams, now,
);
```

P3 後:

```rust
// 変更後: Loading / Active 両 variant から clock を取り出す
// Loading の clock は Waiting 状態 → tick しても空 range → reached_end=false → 問題なし
let (clock, store, active_streams) = match &mut self.state.session {
    ReplaySession::Loading { clock, store, active_streams, .. }
    | ReplaySession::Active { clock, store, active_streams } => (clock, store, active_streams),
    ReplaySession::Idle => {
        return TickOutcome { trade_events: vec![], reached_end: false };
    }
};
let dispatch = dispatcher::dispatch_tick(clock, store, active_streams, now);
```

#### その他のアクセス箇所の更新

`controller.rs` の以下のメソッドも session match に書き換える:

| メソッド | 変更内容 |
|---|---|
| `is_playing()` | `Active { clock, .. }` の `clock.status() == Playing` |
| `is_paused()` | `Active { clock, .. }` の `clock.status() == Paused` |
| `is_loading()` | `Loading { .. }` にマッチするか |
| `has_clock()` | `Idle` 以外なら `true` |
| `is_at_end()` | `Active { clock, .. }` の `clock.now_ms() >= clock.full_range().end` |
| `speed_label()` | `Active { clock, .. }` の `format_speed_label(clock.speed())` |
| `active_kline_streams()` | `Loading` / `Active` の `active_streams` を参照 |
| `active_stream_debug_labels()` | 同上 |
| `reset_session()` | `self.state.session = ReplaySession::Idle` の1行 |
| `try_resume_from_waiting` | `pending_count` カウンタに置き換え → **削除** |

### TDD サイクル

#### RED: 書くテスト

テストは `ReplayController::handle_message` を経由して書く（`ReplaySession` を直接構築しない）。

```rust
// controller.rs #[cfg(test)] mod tests に追加

/// Play を送ると session が Loading になること（kline_targets が空でない場合）
/// NOTE: Dashboard::default() では kline_targets が空になるため Loading に入らない。
/// この遷移は統合テストか、ヘルパーで kline_targets をセットした状態でテストする。
/// ここでは「kline なし → 即 Active（Paused）」のケースでセッション型を検証する。
#[test]
fn session_is_active_after_play_with_no_klines() {
    let mut ctrl = ReplayController::default();
    ctrl.state.mode = ReplayMode::Replay;
    ctrl.state.range_input.start = "2025-01-01 00:00".to_string();
    ctrl.state.range_input.end = "2025-01-02 00:00".to_string();
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    let _ = ctrl.handle_message(ReplayMessage::Play, &mut dashboard, win);

    assert!(
        matches!(ctrl.state.session, ReplaySession::Active { .. }),
        "kline なし → 即 Active に遷移するはず"
    );
}

/// DataLoadFailed を受けると session が Idle になること
#[test]
fn session_transitions_to_idle_on_data_load_failed() {
    let mut ctrl = ReplayController::default();
    // Loading 状態を手動で作る
    ctrl.state.session = ReplaySession::Loading {
        clock: StepClock::new(1_000_000, 4_000_000, 60_000),
        pending_count: 2,
        store: EventStore::new(),
        active_streams: HashSet::new(),
    };
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    let _ = ctrl.handle_message(
        ReplayMessage::DataLoadFailed("timeout".to_string()),
        &mut dashboard,
        win,
    );

    assert!(
        matches!(ctrl.state.session, ReplaySession::Idle),
        "DataLoadFailed → Idle に遷移するはず"
    );
}

/// Loading で pending_count=1 のとき KlinesLoadCompleted を受けると Active になること
#[test]
fn session_transitions_loading_to_active_when_last_stream_loaded() {
    use std::collections::HashSet;
    use exchange::adapter::{StreamKind, Timeframe};

    let stream = StreamKind::Kline {
        exchange: exchange::adapter::Exchange::BinanceFutures,
        symbol: "BTCUSDT".to_string(),
        timeframe: Timeframe::M1,
    };
    let mut active = HashSet::new();
    active.insert(stream);

    let mut ctrl = ReplayController::default();
    ctrl.state.session = ReplaySession::Loading {
        clock: StepClock::new(1_000_000, 4_000_000, 60_000),
        pending_count: 1,
        store: EventStore::new(),
        active_streams: active,
    };
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();

    let range = 1_000_000..4_000_000;
    let _ = ctrl.handle_message(
        ReplayMessage::KlinesLoadCompleted(stream, range, vec![]),
        &mut dashboard,
        win,
    );

    assert!(
        matches!(ctrl.state.session, ReplaySession::Active { .. }),
        "pending_count=1 → ロード完了で Active に遷移するはず"
    );
}

/// Loading で pending_count=2 のとき KlinesLoadCompleted を1本受けても Loading のままであること
#[test]
fn session_stays_loading_while_pending_count_above_zero() {
    // ... pending_count=2 → 1本完了 → Loading のまま
}

/// Idle 状態では is_loading / is_playing / is_paused がすべて false であること
#[test]
fn session_idle_all_status_false() {
    let ctrl = ReplayController::default();
    assert!(!ctrl.is_loading());
    assert!(!ctrl.is_playing());
    assert!(!ctrl.is_paused());
    assert!(!ctrl.has_clock());
}
```

**実行**:
```bash
cargo test -p flowsurface session_
# FAIL (ReplaySession 未定義) を確認
```

#### GREEN: 最小実装

1. `ReplaySession` enum を `mod.rs` に追加
2. `ReplayState` に `session: ReplaySession` フィールドを追加し、
   `clock` / `event_store` / `active_streams` フィールドを削除
3. `controller.rs` のすべての `self.state.clock` / `self.state.event_store` / `self.state.active_streams`
   アクセスを `match &self.state.session` / `match &mut self.state.session` に書き換え
4. `tick()` を新パターンに更新（上記 `tick()` の更新 を参照）
5. `KlinesLoadCompleted` / `DataLoadFailed` / `ReloadKlineStream` ハンドラを
   2ステップ `mem::replace` パターンに書き換え
6. `reset_session()` を `session = Idle` の1行に書き換え
7. `try_resume_from_waiting` を削除

```bash
cargo test -p flowsurface
```

#### REFACTOR

- `is_loading()` / `is_playing()` / `is_paused()` / `has_clock()` を exhaustive match に更新
- `active_kline_streams()` / `active_stream_debug_labels()` を session match に更新
- `from_saved` の `clock: None` 初期化を `session: ReplaySession::Idle` に変更

### 完了条件

- [ ] `ReplaySession` enum が `mod.rs` に定義されている
- [ ] `ReplayState` の `clock` / `event_store` / `active_streams` フィールドが削除され `session` に置き換わっている
- [ ] `reset_session()` が `session = Idle` の1行になっている
- [ ] `try_resume_from_waiting` が削除され `pending_count` カウンタに置き換わっている
- [ ] `tick()` が `Loading | Active` パターンで clock を取り出している
- [ ] DataLoadFailed → Idle 遷移が型レベルで保証されている（match の exhaustiveness による）
- [ ] DataLoadFailed 後の遅延 KlinesLoadCompleted がサイレントに無視されることをコメントで明記
- [ ] `Loading → Active` / `Active → Loading` が 2ステップ `mem::replace` パターンを使っている
- [ ] 新規テスト 5 件以上 PASS
- [ ] 既存テスト全 PASS（P1・P2 のテスト含む）
- [ ] `cargo clippy -- -D warnings` エラーなし

---

## P4: `ReplayMessage` 責務分割

> **前提**: P3 完了後に着手する。P3 の State Machine があることで分割後の境界が明確になる。

### 変更ファイル

- `src/replay/mod.rs`（enum 分割）
- `src/replay/controller.rs`（ハンドラ分割）
- `src/replay/loader.rs`（Task closure の戻り型変更）
- `src/main.rs`（全 `ReplayMessage::KlinesLoadCompleted` / `DataLoadFailed` 参照、
  および `Message::Replay(ReplayMessage::ToggleMode)` 等のパターン変更）

### 設計

```rust
/// UI 操作（ユーザーが発火）
pub enum ReplayUserMessage {
    ToggleMode,
    StartTimeChanged(String),
    EndTimeChanged(String),
    Play,
    Resume,
    Pause,
    StepForward,
    StepBackward,
    CycleSpeed,
}

/// 非同期タスク応答（load_klines Task が発火）
/// handle_load_event の戻り値が Option<Toast> になるため Task を返さない
pub enum ReplayLoadEvent {
    KlinesLoadCompleted(StreamKind, Range<u64>, Vec<Kline>),
    DataLoadFailed(String),
}

/// システムイベント（main.rs のシステムイベントが発火）
pub enum ReplaySystemEvent {
    SyncReplayBuffers,
    ReloadKlineStream { old_stream: Option<StreamKind>, new_stream: StreamKind },
}
```

#### ハンドラシグネチャ

```rust
impl ReplayController {
    /// UI 操作を処理する。非同期タスクを起動する可能性がある（Play 時に kline ロードタスクを発行）。
    pub fn handle_user_message(
        &mut self,
        msg: ReplayUserMessage,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) { ... }

    /// 非同期ロードイベントを処理する。
    /// KlinesLoadCompleted も DataLoadFailed もタスクを起動しないため、
    /// Task を返す必要がない。これを型で表現する。
    pub fn handle_load_event(
        &mut self,
        event: ReplayLoadEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Option<Toast> { ... }  // ← Task<ReplayMessage> を返さない

    /// システムイベントを処理する。
    pub fn handle_system_event(
        &mut self,
        event: ReplaySystemEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) { ... }
}
```

#### iced との整合性（変換レイヤー）

iced の `update(&mut self, msg: Message)` は単一型を受け取るため、
`ReplayLoadEvent` は `From<ReplayLoadEvent> for ReplayMessage` で変換する。

`loader.rs` の `Task::perform` クロージャも変更が必要:

```rust
// loader.rs 変更前
Task::perform(loader::load_klines(stream, range), |result| match result {
    Ok(r) => ReplayMessage::KlinesLoadCompleted(r.stream, r.range, r.klines),
    Err(e) => ReplayMessage::DataLoadFailed(e),
})

// loader.rs 変更後（ReplayLoadEvent を経由する）
Task::perform(loader::load_klines(stream, range), |result| match result {
    Ok(r) => ReplayMessage::Load(ReplayLoadEvent::KlinesLoadCompleted(r.stream, r.range, r.klines)),
    Err(e) => ReplayMessage::Load(ReplayLoadEvent::DataLoadFailed(e)),
})
```

`main.rs` への影響範囲:

```rust
// main.rs 内のパターンマッチが変わる
Message::Replay(ReplayMessage::ToggleMode)    // → User(ReplayUserMessage::ToggleMode)
Message::Replay(ReplayMessage::Play)          // → User(ReplayUserMessage::Play)
// KlinesLoadCompleted / DataLoadFailed の直接参照 → Load(ReplayLoadEvent::...) へ
```

#### `handle_message` 統合ラッパー

移行期間中は `handle_message` を後方互換ラッパーとして残し、P4 完了後に削除可否を判断する:

```rust
pub fn handle_message(
    &mut self,
    msg: ReplayMessage,
    dashboard: &mut Dashboard,
    main_window_id: iced::window::Id,
) -> (Task<ReplayMessage>, Option<Toast>) {
    match msg {
        ReplayMessage::User(m) => self.handle_user_message(m, dashboard, main_window_id),
        ReplayMessage::Load(e) => {
            let toast = self.handle_load_event(e, dashboard, main_window_id);
            (Task::none(), toast)
        }
        ReplayMessage::System(e) => self.handle_system_event(e, dashboard, main_window_id),
    }
}
```

### TDD サイクル

#### RED: 書くテスト

```rust
/// KlinesLoadCompleted を handle_load_event で処理すると Toast なし（正常時）
#[test]
fn load_event_completed_returns_no_toast() {
    // ... pending_count=1 の Loading 状態で KlinesLoadCompleted → None
    let toast = ctrl.handle_load_event(event, &mut dashboard, win);
    assert!(toast.is_none());
}

/// DataLoadFailed を handle_load_event で処理すると Toast が返ること
#[test]
fn load_event_failed_returns_error_toast() {
    let toast = ctrl.handle_load_event(
        ReplayLoadEvent::DataLoadFailed("connection refused".to_string()),
        &mut dashboard, win,
    );
    assert!(toast.is_some());
}

/// handle_load_event は Task を返さない（型レベルで保証）
/// このテストはコンパイルが通ることで保証される
#[test]
fn load_event_handler_signature_returns_option_toast() {
    let mut ctrl = make_loading_controller();
    let mut dashboard = Dashboard::default();
    let win = window::Id::unique();
    // Option<Toast> に代入できることをコンパイラが保証
    let _: Option<Toast> = ctrl.handle_load_event(
        ReplayLoadEvent::DataLoadFailed("err".to_string()),
        &mut dashboard, win,
    );
}
```

#### GREEN / REFACTOR

P4 は影響範囲が広いためサブステップで進める:

1. **enum 定義**: `ReplayUserMessage` / `ReplayLoadEvent` / `ReplaySystemEvent` を追加し、
   `ReplayMessage` を `User(ReplayUserMessage) | Load(ReplayLoadEvent) | System(ReplaySystemEvent)` に変更
2. **変換レイヤー**: `loader.rs` の Task closure を更新
3. **`handle_load_event` を抽出**: `Option<Toast>` を返す形で
4. **`handle_user_message` を抽出**
5. **`handle_system_event` を抽出**
6. **`handle_message` をラッパーに書き換え**
7. **`main.rs` の全参照を更新**

```bash
# 各サブステップ後にテストを実行
cargo test -p flowsurface
```

### 完了条件

- [ ] `ReplayUserMessage` / `ReplayLoadEvent` / `ReplaySystemEvent` が定義されている
- [ ] `handle_load_event` の戻り型が `Option<Toast>`（Task なし）になっている
- [ ] `handle_user_message` / `handle_system_event` が分離されている
- [ ] `loader.rs` の Task closure が新しい `ReplayLoadEvent` を使っている
- [ ] `main.rs` の全 `ReplayMessage::KlinesLoadCompleted` / `DataLoadFailed` 参照が更新されている
- [ ] 新規テスト 3 件以上 PASS
- [ ] 既存テスト全 PASS
- [ ] `cargo clippy -- -D warnings` エラーなし

---

## 非目標（本計画書全体）

以下は本計画書のスコープ外とする:

| 項目 | 理由 |
|---|---|
| `pending_auto_play` の `StartupOrchestrator` への外部化 | Dashboard API の設計に依存 |
| `ReloadKlineStream` の seek パターンへの統合 | reset → load → inject の順序が異なる |
| `ClockStatus` の変更 | Playing/Paused/Waiting の3状態は適切 |

---

## テスト実行コマンド

```bash
# 全テスト実行
cargo test -p flowsurface

# リプレイ関連のみ
cargo test -p flowsurface replay

# セッション遷移テストのみ
cargo test -p flowsurface session_

# カバレッジ確認
cargo llvm-cov --package flowsurface --html
# → target/llvm-cov/html/index.html

# Lint
cargo clippy -- -D warnings
```

---

## 進捗

### P1: seek_to メソッド統一
- [ ] RED: テスト追加（seek_to 3件）
- [ ] GREEN: seek_to 実装・4箇所の重複コード置き換え
- [ ] REFACTOR: doc comment で例外ケースを明記
- [ ] 完了条件確認

### P2: play_with_range 追加・pending_auto_play 方針確定
- [ ] RED: テスト追加（play_with_range 2件）
- [ ] GREEN: play_with_range 実装・main.rs 書き換え
- [ ] REFACTOR: pending_auto_play に非目標コメント追加
- [ ] 完了条件確認

### P3: ReplaySession State Machine 導入
- [ ] RED: テスト追加（遷移テスト 5件以上）
- [ ] GREEN: ReplaySession enum 定義・全アクセス箇所を session match に移行・tick() 更新
- [ ] REFACTOR: try_resume_from_waiting 廃止・exhaustive match 整備
- [ ] 完了条件確認

### P4: ReplayMessage 責務分割
- [ ] RED: テスト追加（ハンドラ 3件以上）
- [ ] GREEN: enum 分割・loader.rs 更新・ハンドラ抽出（サブステップ 7段階）・main.rs 全参照更新
- [ ] REFACTOR: handle_message ラッパーの存廃を判断
- [ ] 完了条件確認
