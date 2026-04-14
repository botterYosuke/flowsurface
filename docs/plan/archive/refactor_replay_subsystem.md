# リファクタリング提案書: Replay サブシステム

**作成日**: 2026-04-14  
**対象ブランチ**: `sasa/step`  
**調査対象**: `src/replay/`, `src/replay_api.rs`, `src/main.rs`, `src/screen/dashboard.rs`

---

## 概要

`replay_header.md` 完成を機に、現在の実装を精査した。機能は正しく動いているが、**コードの重複**・**main.rs の肥大化**・**ドメイン責務の散逸**という 3 つの構造的課題がある。本書は優先度ごとに具体的なリファクタリング項目を列挙する。

---

## 優先度 A — 即効性が高い（Quick Wins）

### A-1. StepForward / StepBackward のコード重複を解消

**場所**: `src/main.rs` — `ReplayMessage::StepForward` と `StepBackward` ハンドラ

**問題**: kline 再注入ループが 2 箇所で完全重複している。

```rust
// StepForward (~line 993-1000) と StepBackward (~line 1026-1033) で同一パターン
for stream in self.replay.active_streams.clone().iter() {
    let klines = self.replay.event_store.klines_in(stream, 0..new_time + 1);
    if !klines.is_empty() {
        let klines_vec = klines.to_vec();
        self.active_dashboard_mut()
            .ingest_replay_klines(stream, &klines_vec, main_window_id);
    }
}
```

**対策**:  
`main.rs` または `ReplayState` に以下のメソッドを追加し、両ハンドラから呼ぶ。

```rust
/// start_ms..=target_ms の klines を全 active_streams からダッシュボードに注入する
fn inject_klines_up_to(&mut self, target_ms: u64, window_id: window::Id) {
    for stream in self.replay.active_streams.iter() {
        let klines = self.replay.event_store.klines_in(stream, 0..target_ms + 1);
        if !klines.is_empty() {
            self.active_dashboard_mut()
                .ingest_replay_klines(stream, &klines.to_vec(), window_id);
        }
    }
}
```

**副次効果**: `.clone().iter()` → `.iter()` でループ内の HashSet クローンも除去できる。

---

### A-2. `active_streams.clone().iter()` → `active_streams.iter()` に変更

**場所**: `src/main.rs` — StepForward / StepBackward ハンドラ（上記 A-1 と同箇所）  

**問題**: イテレート目的のみで `HashSet<StreamKind>` をクローンしており、不要なアロケーションが発生している。  
**対策**: Rust の借用ルール上、イテレート中に同じ `self` を可変借用しなければ `.clone()` は不要。A-1 の抽出で自然に解消する。

---

### A-3. `reply_replay_status` クロージャを `ReplayState` のメソッドに昇格

**場所**: `src/main.rs` — `Message::ReplayApi` ハンドラ内に毎回定義されるクロージャ

**問題**: `ReplayApi` ハンドラで `let reply_replay_status = || { ... }` クロージャを定義し、8 箇所以上から呼んでいる。状態は `self.replay` だけ参照しており、クロージャである必要がない。

**対策**:

```rust
// src/replay/mod.rs に追加
impl ReplayState {
    pub fn to_json_status(&self) -> String {
        serde_json::to_string(&self.status()).unwrap_or_default()
    }
}
```

`ReplayApi` ハンドラ内で `self.replay.to_json_status()` を呼ぶだけになる。

---

### A-4. `EventStore` 内の `SortedVec<Trade>` / `SortedVec<Kline>` 重複を generics で統合

**場所**: `src/replay/store.rs`

**問題**: `insert_sorted`・`range_slice` の実装が `Trade` と `Kline` でほぼ同一。

**対策**: タイムスタンプを持つ型に対する trait bound を定義し、`SortedVec<T>` を共通化する。

```rust
trait HasTimestamp {
    fn timestamp_ms(&self) -> u64;
}

struct SortedVec<T>(Vec<T>);

impl<T: HasTimestamp + Clone> SortedVec<T> {
    fn insert_sorted(&mut self, items: Vec<T>) { /* 共通実装 */ }
    fn range_slice(&self, range: Range<u64>) -> &[T] { /* 共通実装 */ }
}
```

`Kline` / `Trade` に `HasTimestamp` を実装するだけで重複が消える。

---

## 優先度 B — 構造改善（中規模）

### B-1. `handle_replay_message()` を `update()` から分離

**場所**: `src/main.rs` — `update()` 内の `Message::Replay(msg)` アーム（~200 行）

**問題**: `update()` 全体が 1000 行超の巨大関数であり、Replay ブランチだけで ~200 行を占めている。テストも困難。

**対策**:

```rust
// src/main.rs に追加
impl App {
    fn handle_replay_message(
        &mut self,
        msg: ReplayMessage,
        main_window: window::Id,
    ) -> Task<Message> {
        match msg {
            ReplayMessage::Play => { /* ... */ }
            // ...
        }
    }
}
```

`update()` の `Message::Replay(msg)` アームを `self.handle_replay_message(msg, main_window_id)` の 1 行にする。

---

### B-2. `loaded_ranges` のオーバーラップマージ

**場所**: `src/replay/store.rs` — `ingest_loaded()` / `is_loaded()`

**問題**: `loaded_ranges: Vec<Range<u64>>` にエントリが積み重なり、`is_loaded()` は O(n) の線形スキャンになる。mid-replay バックフィルを繰り返すと微小に悪化する。

**対策**: `ingest_loaded()` 呼び出し後に隣接・重複する range をマージし、`loaded_ranges` を常に最小集合に保つ。

```rust
fn merge_ranges(ranges: &mut Vec<Range<u64>>) {
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<u64>> = Vec::new();
    for r in ranges.drain(..) {
        if let Some(last) = merged.last_mut() {
            if r.start <= last.end { last.end = last.end.max(r.end); continue; }
        }
        merged.push(r);
    }
    *ranges = merged;
}
```

---

### B-3. Dashboard ペイン反復パターンの統合

**場所**: `src/screen/dashboard.rs`

**問題**: 以下 3 メソッドが `iter_all_panes_mut()` + 同型ループを繰り返している。

| メソッド | 呼び出す操作 |
|---|---|
| `clear_chart_for_replay()` | `rebuild_content_for_replay()` |
| `reset_charts_for_seek()` | `reset_for_seek()` |
| `rebuild_for_live()` | `rebuild_content_for_live()` |

**対策**: クロージャを受け取るヘルパーを導入。

```rust
fn apply_to_all_panes_mut<F>(&mut self, mut f: F)
where
    F: FnMut(&mut PaneState),
{
    for (_, state) in self.iter_all_panes_mut() {
        f(state);
    }
}
```

3 メソッドはそれぞれ 1 行の呼び出しになる。

---

### B-4. `ReplySender` の `Arc<Mutex<Option<>>>` を簡略化

**場所**: `src/replay_api.rs`

**問題**: `oneshot::Sender` に `Clone` を持たせるため `Arc<Mutex<Option<Sender>>>` という多重ラップを使用。実装が読みにくく、`take()` 漏れでサイレントに応答が消えるリスクがある。

**対策案**:  
- `Arc<OnceLock<Sender>>` への置き換えを検討（`std::sync::OnceLock` は set が 1 回限りを保証）  
- または内部実装を `parking_lot::Mutex` に変えてロックコストを下げつつ、`debug_assert` で二重 `take()` を検出する

---

## 優先度 C — 設計整理（将来向け）

### C-1. `ReplayController` を独立した構造体として定義

**場所**: `src/main.rs` 全体

**問題**: Replay ロジックが `App` の `update()` に直接書かれており、`Dashboard` 操作・`ReplayState` 操作・Task 生成が混在している。

**提案**:

```
src/replay/
  mod.rs        ← ReplayState (現状維持)
  clock.rs      ← StepClock (現状維持)
  store.rs      ← EventStore (現状維持)
  dispatcher.rs ← dispatch_tick (現状維持)
  loader.rs     ← load_klines (現状維持)
  controller.rs ← ReplayController（新規）
```

`ReplayController` が `ReplayState` + `Dashboard` への操作を統括し、`main.rs` は `Message` ルーティングのみに専念する形に整理する。大きな改修のため、B-1 完了後に着手する。

---

### C-2. `DispatchResult::reached_end` の活用確認

**場所**: `src/replay/dispatcher.rs` L16 / `src/main.rs` の Tick ハンドラ

**問題**: `reached_end` フラグが `DispatchResult` に存在するが、Tick ハンドラ側で明示的に使われているか要確認。終端到達時に UI 通知（Toast 等）を出す拡張を将来行う際に必要。  
**対策**: 使われていなければ削除、使われているなら UI フィードバックを追加する（現状は `clock` が自動 Pause するだけ）。

---

### C-3. API ルーティングの routing table 化

**場所**: `src/replay_api.rs` — `route()` 関数（~60 行の match）

**問題**: `(method, path)` のタプルマッチが 1 関数に詰まっており、エンドポイント追加のたびに関数が伸びる。

**対策**: ルートテーブルを `&[(&str, &str, fn(...) -> ApiCommand)]` として定義し、`route()` はテーブルを走査するだけにする。または `phf` クレートの静的マップを利用する。

---

## 対応順序の推奨

```
A-1 + A-2（同時）→ A-3 → A-4 → B-1 → B-3 → B-2 → B-4 → C-1 → C-2 + C-3
```

A 系はすべてリスクが低く、レビューコストも小さい。B-1 は A 系完了後に着手することで差分が明確になる。C 系は新機能追加の直前に実施するのが最もコスト効率がよい。

---

## 変更しない判断をした箇所

| 箇所 | 理由 |
|---|---|
| `StepClock` 全体 | 状態機械が明確で変更の必要なし |
| `dispatch_tick` のアルゴリズム本体 | ステートレスで副作用が少なく、現状で十分 |
| `ReplySender` の Clone 戦略 | iced の `Message: Clone` 制約に起因する必然的実装。置き換えは B-4 で検討するが優先度は低い |
| WebSocket 制御 (`subscription()`) | 宣言的モデルが適切に機能しており、変更の余地なし |
