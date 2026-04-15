# リプレイ機能リファクタリング計画

**作成**: 2026-04-15  
**最終更新**: 2026-04-15（R4-1〜R4-7 全フェーズ完了）
**対象ブランチ**: `sasa/develop`
**目的**: 試行錯誤で蓄積した技術的負債を解消し、最高品質のコードベースへ再建する

---

## 背景と診断

`src/replay/` は R1〜R3 の段階的リファクタリングと機能追加（Tachibana 対応・mid-replay 操作・auto-play）を経た結果、以下の問題が複合的に積み重なっている。

| 重大度 | 問題 | 影響箇所 |
|:---:|---|---|
| 🔴 | `Deref`/`DerefMut` アンチパターン（カプセル化崩壊） | controller.rs, main.rs |
| 🔴 | `pub` フィールドへの直接書き込みが main.rs に散在 | main.rs, mod.rs |
| 🟡 | `StartTimeChanged`/`EndTimeChanged` ハンドラの完全重複 | controller.rs |
| 🟡 | `DataLoadFailed` のリカバリーが不完全（残留 state） | controller.rs |
| 🟡 | 未使用 dead code の `#[allow(dead_code)]` 放置 | clock.rs, store.rs |
| 🟡 | `seek_to_start_on_end` フラグが本番コードで未使用 | clock.rs, dispatcher.rs |
| 🟡 | `inject_klines_up_to` の範囲 `0..` が不正確 | controller.rs |
| 🟢 | テストヘルパー重複（3 ファイルに同一コード） | dispatcher.rs, store.rs, loader.rs |
| 🟢 | `loader.rs` 内 Tachibana 特殊ケースが抽象化を破る | loader.rs |
| 🟢 | `CycleSpeed` の副作用（位置リセット）が仕様書に未記載 | controller.rs, replay_header.md |
| 🟢 | `SortedVec::dedup_by_key` が同一 ms trade を消す意図が不明 | store.rs |

---

## アーキテクチャ上の根本問題

```
現状（問題あり）:
  main.rs
    ├── self.replay.pending_auto_play = false   // コントローラを通り越した直接書き込み
    ├── self.replay.range_input.start = ...      // 同上
    ├── self.replay.clock.is_some()             // private であるべきフィールドへの直接アクセス
    └── self.replay.active_streams              // 同上

  ReplayController
    ├── Deref<Target=ReplayState>               // 暗黙の委譲がカプセル化を破る
    └── state: ReplayState (pub フィールド群)

理想（目指す姿）:
  main.rs
    └── self.replay.xxx()  // 公開メソッドのみ経由

  ReplayController
    ├── 公開メソッド: handle_message / tick / is_replay / current_time / ...
    └── state: ReplayState (プライベート、外部から不可視)
```

---

## フェーズ計画

### Phase R4-1: Dead Code 除去（安全・最初に実施）

**目標**: `#[allow(dead_code)]` を一切なくす。未使用コードは削除する。

**削除対象**:

| 対象 | 場所 | 理由 |
|---|---|---|
| `SPEED_INSTANT` 定数 | `clock.rs:9` | 本番コード未使用。テストのみ。定数をテスト内定義に移す |
| `extend_range_end()` メソッド | `clock.rs:74` | 呼び出し元ゼロ |
| `set_seek_to_start_on_end()` メソッド | `clock.rs:80` | 本番コード呼び出し元ゼロ |
| `seek_to_start_on_end` フィールド全体 | `clock.rs` | 本番で使われず、テストのみ保持する意義がない |
| `extend_loaded_range_end_to()` | `store.rs:119` | 呼び出し元ゼロ |
| `drop_stream()` | `store.rs:131` | `#[cfg(test)]` のみ使用。テスト内ヘルパーとして残すか削除 |

**`seek_to_start_on_end` 除去の影響**:
- `dispatcher.rs`: `range.start > range.end` という逆転レンジによる `reached_end` 判定が不要になる
- `dispatcher.rs` の `reached_end` 判定を単純化: `clock.status() == ClockStatus::Paused` のみで十分

**⚠️ テスト崩壊に注意（約 18 本）**:

`seek_to_start_on_end` と `extend_range_end` を削除すると `clock.rs` のテストが大量に崩壊する。
削除と同時にテストも整理する（削除）:

| テストグループ | 本数 | 対応 |
|---|---:|---|
| `seek_to_start_on_end_*` 系 | 8 | 削除（機能ごと削除するため） |
| `extend_range_end_*` 系 | 6 | 削除（メソッドごと削除するため） |
| `extend_then_seek_to_start_*` 系 | 2 | 削除 |
| `pause_clears_seek_to_start_on_end_flag` 等 | 2 | 削除 |

`SPEED_INSTANT` は `clock.rs` の `instant_speed_*` テスト内で使用中。
定数を `#[cfg(test)]` ブロック内に移動すればよい（別ファイルへの移動は不要）。

---

### Phase R4-2: `Deref`/`DerefMut` 廃止とカプセル化再建

**目標**: `main.rs` が `ReplayState` のフィールドに直接触れられなくする。

#### Step 1: `ReplayState` フィールドを非公開化

```rust
// 変更前
pub struct ReplayState {
    pub mode: ReplayMode,
    pub range_input: ReplayRangeInput,
    pub clock: Option<StepClock>,
    pub event_store: EventStore,
    pub active_streams: HashSet<StreamKind>,
    pub pending_auto_play: bool,
}

// 変更後
pub struct ReplayState {
    mode: ReplayMode,
    range_input: ReplayRangeInput,
    clock: Option<StepClock>,
    event_store: EventStore,
    active_streams: HashSet<StreamKind>,
    pending_auto_play: bool,
}
```

#### Step 2: `ReplayController` から `Deref`/`DerefMut` を削除

```rust
// 削除
impl std::ops::Deref for ReplayController { ... }
impl std::ops::DerefMut for ReplayController { ... }
```

#### Step 3: `main.rs` が必要とするアクセスをメソッドで提供

`main.rs` が現在直接アクセスしているフィールド一覧と、対応するメソッド:

| `main.rs` の現在のアクセス | 追加するメソッド |
|---|---|
| `self.replay.pending_auto_play = false` | `replay.clear_pending_auto_play()` |
| `self.replay.range_input.start = s` / `.end = s` | `replay.set_range_input(start, end)` |
| `self.replay.range_input.start` (read) | `replay.range_input()` → `&ReplayRangeInput` |
| `self.replay.clock.is_some()` | `replay.has_clock() -> bool` |
| `self.replay.clock.as_ref().is_some_and(\|c\| c.now_ms() >= c.full_range().end)` | `replay.is_at_end() -> bool` |
| `self.replay.mode` (read, serialize) | `replay.mode() -> ReplayMode` |
| `self.replay.active_streams` (iter) | `replay.active_kline_streams() -> impl Iterator<Item=&StreamKind>` |

**影響範囲**: `main.rs` の約 20 箇所を修正（読み取り専用のものは const getter、書き込みは専用メソッド）

**⚠️ controller.rs テストの修正も必要**:

`controller.rs` のテスト（`ctrl.state.clock`, `ctrl.state.current_time()` 等）が
`state` フィールドに直接アクセスしている。フィールド非公開化後は
テストも公開メソッド経由に書き直す必要がある（`ctrl.state.clock.as_ref().unwrap().status()` →
`ctrl.clock_status()` のような getter を追加して対応する）。

#### Step 4: `ReplayController.state` フィールドを非公開化

```rust
pub struct ReplayController {
    state: ReplayState,  // pub → private
}
```

---

### Phase R4-3: コントローラロジックの整理

**目標**: ハンドラ重複除去、副作用の明示化、リカバリー修正

#### 3-1: `StartTimeChanged`/`EndTimeChanged` 重複解消

`controller.rs` の 2 ハンドラは完全同一ロジック。プライベートメソッドに抽出:

```rust
fn handle_range_input_change(&mut self, dashboard: &mut Dashboard, main_window_id: iced::window::Id) {
    let start_ms = self.state.clock.as_ref().map(|c| c.full_range().start);
    if let Some(start_ms) = start_ms {
        if let Some(clock) = &mut self.state.clock {
            clock.pause();
            clock.seek(start_ms);
        }
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(start_ms, dashboard, main_window_id);
    }
}
```

#### 3-2: `CycleSpeed` の副作用を仕様化または除去

現状の挙動: 速度変更 → クロックを先頭へシーク + 停止（驚き）

**選択肢 A**: 副作用を除去し、純粋な速度切替にする（UI を直感的に）
```rust
ReplayMessage::CycleSpeed => {
    self.state.cycle_speed();
    (Task::none(), None)
}
```

**選択肢 B**: 副作用を維持するが仕様書に明記、UI にアイコン変化で示す

→ **A を採用**: 速度変更がシークを引き起こす理由がユーザーには不明。Playing 中の speed 変更は `StepClock::set_speed()` で即時反映されるため副作用不要。

**⚠️ テスト 3 本の削除と新テスト追加が必要**:

`controller.rs` に副作用を「正しい挙動」として検証するテストが存在する:
- `cycle_speed_while_playing_pauses_clock`
- `cycle_speed_while_playing_seeks_to_range_start`
- `cycle_speed_while_paused_seeks_to_range_start`

R4-3 実施時にこれら 3 本を削除し、「速度変更のみで位置・ClockStatus は変わらない」ことを
検証する新テストに置き換える。

#### 3-3: `DataLoadFailed` のリカバリー修正

現状は `clock = None` のみ。`event_store` と `active_streams` の残留により次回 Play で干渉する可能性。

```rust
ReplayMessage::DataLoadFailed(err) => {
    // clock だけでなく関連状態を全リセット
    self.state.clock = None;
    self.state.event_store = EventStore::new();
    self.state.active_streams = HashSet::new();
    (Task::none(), Some(Toast::error(...)))
}
```

→ Phase R4-2 後は `self.state.reset_session()` として `ReplayState` のメソッドに集約。

#### 3-4: `inject_klines_up_to` の範囲修正

現状: `0..target_ms + 1` → pre-history バー（start_ms 前）が Seek 操作のたびに再注入される。

修正: `start_ms` を保持して `pre_history_start..target_ms + 1` を使用する。
`pre_history_start` = `start_ms - PRE_START_HISTORY_BARS * min_step_ms`

ただし、`reset_charts_for_seek` → `inject_klines_up_to` のペアで動作するため、
履歴バーも含めて再注入することが**意図的**か確認が必要。

**診断**: `KlinesLoadCompleted` で pre-history を `ingest_replay_klines` しているため、
`inject_klines_up_to` でも `0..` で取得しないとチャートに履歴バーが表示されない。
→ 現状の `0..` は**意図的**。ただしそのことをコメントで明示する。

---

### Phase R4-4: `dispatcher.rs` の単純化

**目標**: `reached_end` の判定から逆転レンジへの依存を除去。

`seek_to_start_on_end` 除去後、`dispatch_tick` の `reached_end` 判定は以下に単純化できる:

```rust
// 変更前（複雑、逆転レンジ依存）
let reached_end = range.start > range.end && clock.status() == ClockStatus::Paused;

// 変更後（単純）
// early-return パスでは range.is_empty() == true なので !range.is_empty() は常に false
// → early-return では reached_end: false で十分
// 通常パス（range が進んだ場合）では clock.status() == Paused で終端到達を判定
let reached_end = !range.is_empty() && clock.status() == ClockStatus::Paused;
```

**`controller.rs` の対応ブロックも削除する**（R4-4 スコープ）:

`seek_to_start_on_end` 発火検出のために書かれた `controller.rs:434-441` のブロックを削除する。
フラグ除去後この条件は永遠に true にならないためデッドコードになる:

```rust
// 削除対象（controller.rs tick() 内）
// seek_to_start_on_end が発火すると current_time == range.start になる。
if dispatch.reached_end {
    let range_start = self.state.clock.as_ref().map(|c| c.full_range().start);
    if Some(dispatch.current_time) == range_start {
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(dispatch.current_time, dashboard, main_window_id);
    }
}
```

削除後の `tick()` 末尾:

```rust
TickOutcome {
    trade_events,
    reached_end: dispatch.reached_end,
}
```

---

### Phase R4-5: テストヘルパー共通化

**目標**: `dispatcher.rs`, `store.rs`, `loader.rs` の test モジュールにある重複ヘルパーを統一。

共通ヘルパーを `src/replay/testutil.rs` に定義し、`mod.rs` で条件付き公開:

```rust
// src/replay/testutil.rs
use exchange::adapter::StreamKind;
use exchange::{Kline, Trade};

pub fn dummy_trade(time: u64) -> Trade { ... }
pub fn dummy_kline(time: u64) -> Kline { ... }
pub fn trade_stream() -> StreamKind { ... }
pub fn kline_stream() -> StreamKind { ... }
```

```rust
// src/replay/mod.rs に追加
#[cfg(test)]
pub(crate) mod testutil;
```

各テストファイルで `use crate::replay::testutil::*;` で参照。

**⚠️ 可視性の注意**: `pub(super)` では `dispatcher.rs` / `store.rs` / `loader.rs` の
テストから参照できない（`super` は `src/replay/` の親 `src/` を指すため）。
`mod.rs` で `pub(crate)` として公開し、各テストは `crate::replay::testutil` で参照する。

---

### Phase R4-6: `loader.rs` の取引所結合解消

**目標**: `fetch_all_klines` 内の Tachibana 分岐を取引所アダプタ層に移動。

現状:
```rust
// loader.rs — Tachibana を特別扱い（抽象化違反）
if ticker_info.ticker.exchange.venue() == Venue::Tachibana {
    return crate::connector::fetcher::fetch_tachibana_daily_klines(...).await;
}
adapter::fetch_klines(...).await
```

改善案: `exchange::adapter::fetch_klines` が Tachibana を内部で透過的に扱えるよう、
`exchange` クレート側の adapter に Tachibana ルーティングを移動する。

`loader.rs` は常に `adapter::fetch_klines(ticker_info, timeframe, range)` を呼ぶだけにする。

---

### Phase R4-7: `SortedVec::dedup_by_key` の設計意図明示

`dedup_by_key(|t| t.time)` は同一ミリ秒の 2 件目以降の trade を消す。
暗号資産の高頻度取引では同一 ms に複数約定が存在しうる。

**診断**: リプレイは kline ベースの視覚化が主目的で、trade 件数の正確性より
連続したデータストリームとしての整合性が重要。同一 ms trade の消失は
リプレイビューでは許容範囲内。

**対応**: コメントで設計判断を明記し、`dedup_by_key` は維持する。

```rust
// DESIGN: 同一ミリ秒の trade は先着 1 件のみ保持する。
// リプレイは視覚化目的のため、ms 精度の trade 完全再現は非ゴール。
// より高精度が必要な場合は (time, price, qty) の複合キーでの dedup に変更すること。
self.data.dedup_by_key(|t| t.time);
```

---

## 実装順序と依存関係

```
R4-1 (Dead Code 除去)
  └─→ R4-4 (dispatcher 単純化)  ← seek_to_start_on_end 削除後に単純化可能
       
R4-2 (Deref 廃止・カプセル化)
  └─→ R4-3 (ロジック整理)  ← private フィールド化後に state.reset_session() 等が作れる

R4-5 (テストヘルパー共通化)  ← 独立実施可能

R4-6 (loader 結合解消)       ← 独立実施可能（exchange クレート変更が必要）

R4-7 (SortedVec コメント追加) ← 即時実施可能（コード変更なし）
```

**推奨実施順**: R4-1 → R4-7 → R4-5 → R4-4 → R4-2 → R4-3 → R4-6

---

## 品質基準

- `cargo clippy -- -D warnings` エラーゼロ（`#[allow(dead_code)]` は一切許可しない）
- `cargo test` 全パス
- テストカバレッジ: 変更ファイル 80%+ 維持
- `main.rs` から `ReplayState` のフィールドへの直接アクセスゼロ（Grep で検証）

---

## 完了チェックリスト

### R4-1: Dead Code 除去 ✅
- ✅ `SPEED_INSTANT` 定数を `clock.rs` の `#[cfg(test)]` ブロック内に移動
- ✅ `extend_range_end()` 削除
- ✅ `set_seek_to_start_on_end()` 削除
- ✅ `seek_to_start_on_end` フィールドと `tick()` 内の関連ロジック削除
- ✅ `extend_loaded_range_end_to()` 削除
- ✅ `drop_stream()` は `#[cfg(test)]` のままテスト内ヘルパーとして維持（適切）
- ✅ `#[allow(dead_code)]` 全消去確認
- ✅ **テスト削除**: `seek_to_start_on_end_*` 系 8 本を削除
- ✅ **テスト削除**: `extend_range_end_*` 系 6 本を削除
- ✅ **テスト削除**: `extend_then_seek_to_start_*` 系 2 本を削除
- ✅ **テスト削除**: `pause_clears_seek_to_start_on_end_flag` 等 2 本を削除
- ✅ `cargo test` 全パス確認

### R4-2: Deref 廃止・カプセル化 ✅
- ✅ `ReplayState` の全 `pub` フィールドを非公開化
- ✅ `ReplayController.state` を非公開化
- ✅ `Deref`/`DerefMut` impl 削除
- ✅ `main.rs` が必要とする getter/setter メソッドを `ReplayController` に追加
  - `is_replay/playing/paused/loading/at_end/has_clock/mode/speed_label` 等
  - `is_auto_play_pending / clear_pending_auto_play`
  - `set_range_start / set_range_end / range_input_start / range_input_end`
  - `active_kline_streams / active_stream_debug_labels / format_current_time`
  - `on_session_unavailable / to_status`
  - `from_saved(mode, start, end, pending)` コンストラクタ追加
- ✅ `main.rs` の直接フィールドアクセスを全修正
- ✅ **テスト**: `controller.rs` のテストは子モジュールとして private フィールドに直接アクセス可能なため修正不要（Rust のモジュール可視性ルール上問題なし）

### R4-3: コントローラロジック整理 ✅
- ✅ `handle_range_input_change()` プライベートメソッド抽出
- ✅ `CycleSpeed` の副作用（シーク + 停止）を除去（速度変更のみ）
- ✅ **テスト削除**: `cycle_speed_while_playing_pauses_clock` / `_seeks_to_range_start` / `cycle_speed_while_paused_seeks_to_range_start` の 3 本を削除
- ✅ **テスト追加**: `cycle_speed_while_playing_keeps_status` / `_does_not_seek` / `cycle_speed_while_paused_does_not_seek` の 3 本を追加
- ✅ `DataLoadFailed` で `event_store` + `active_streams` もリセット（`reset_session()` メソッド）
- ✅ `inject_klines_up_to` の `0..` 範囲に設計コメント追記

### R4-4: dispatcher 単純化 ✅
- ✅ `dispatcher.rs` の逆転レンジ依存 `reached_end` 判定を削除し `DispatchResult::empty()` を使用
- ✅ `dispatcher.rs` early-return パスの `reached_end: false` を確認
- ✅ **`controller.rs` 修正**: `tick()` 内の `seek_to_start_on_end` 対応ブロックを削除

### R4-5: テストヘルパー共通化 ✅
- ✅ `src/replay/testutil.rs` 作成（`pub(crate)` で公開）
- ✅ `mod.rs` に `#[cfg(test)] pub(crate) mod testutil;` を追加
- ✅ `dispatcher.rs` / `store.rs` / `loader.rs` の重複ヘルパーを除去し `crate::replay::testutil::*` を参照

### R4-6: loader 結合解消
- [ ] `exchange::adapter::fetch_klines` に Tachibana ルーティング移動（スコープ外: exchange クレート変更が必要）
- [ ] `loader.rs` から Tachibana 分岐削除

### R4-7: SortedVec コメント追加 ✅
- ✅ `dedup_by_key` に設計判断コメント追記

---

## 非目標（スコープ外）

- 仮想売買・PnL 機能の追加
- Trades stream のリプレイ対応拡張
- `EventStore` のカーソル/インクリメンタルロード化
- `ReplayState` の永続化フォーマット変更
