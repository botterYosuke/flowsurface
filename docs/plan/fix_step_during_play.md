# 計画書: 再生中の ⏭/⏮ 挙動修正

**作成日**: 2026-04-14  
**ブランチ**: sasa/develop  
**対象ファイル**: `src/replay/clock.rs`, `src/replay/controller.rs`

---

## 概要

再生 (`▶`) 中に ⏭/⏮ を押したときの挙動を修正する。

### 現在の挙動（問題）

| 状態 | ボタン | 現挙動 |
|---|---|---|
| Playing | ⏭ | no-op（早期 return） |
| Playing | ⏮ | 1bar 前に戻って停止（Paused と同じ） |

### 仕様（修正後）

| 状態 | ボタン | 新挙動 |
|---|---|---|
| Paused | ⏭ | 1bar 追加（変更なし） |
| Paused | ⏮ | 1bar 減らす（変更なし） |
| Playing | ⏭ | **range.end を 1 bar 延長 → 再生継続 → 終端到達で start に戻して停止** |
| Playing | ⏮ | **range.end を 1 bar 縮小 → 停止 → 新終端位置へシーク** |

---

## 設計・実装方針

### StepClock への変更 (`clock.rs`)

#### 新フィールド

```rust
seek_to_start_on_end: bool,  // true: 終端到達時に range.start へ戻って停止
```

#### 新メソッド

| メソッド | 説明 |
|---|---|
| `extend_range_end(step: u64)` | range.end を step 分延長する |
| `shrink_range_end(step: u64) -> u64` | range.end を step 分縮小（range.start まで）、新 end を返す |
| `set_seek_to_start_on_end(v: bool)` | フラグを設定する |

#### `tick()` の変更

終端到達時:
```rust
if self.now_ms >= self.range.end {
    if self.seek_to_start_on_end {
        self.now_ms = self.range.start;
        self.seek_to_start_on_end = false;
    }
    self.status = ClockStatus::Paused;
    self.next_step_at = None;
    break;
}
```

### コントローラへの変更 (`controller.rs`)

#### StepForward

```
Playing 時:
  → clock.extend_range_end(step_size)
  → clock.set_seek_to_start_on_end(true)
  → 再生継続（no-op）

Paused 時:
  → 既存ロジック（変更なし）
```

#### StepBackward

```
Playing 時:
  → new_end = clock.shrink_range_end(step_size)
  → clock.pause()
  → clock.seek(new_end)
  → dashboard.reset_charts_for_seek()
  → inject_klines_up_to(new_end)

Paused 時:
  → 既存ロジック（変更なし）
```

---

## 注意点・設計判断

### EventStore と range.end の関係

`extend_range_end` で range.end を延長すると、EventStore には元の range.end 以降のデータが存在しない場合がある。`klines_in` は空スライスを返すだけでパニックしないため、データなし区間はスキップされる（空バー）。  
将来的に追加データフェッチが必要な場合は別タスクとする。

### range_input (UI テキスト) は更新しない

再生中に range.end を変更しても `range_input` は更新しない。`range_input` はユーザーが次回 Play を押したときの入力として使われるため、現在の再生状態と乖離しても問題ない。

---

## 進捗

- ✅ 計画書作成
- ✅ `clock.rs`: `seek_to_start_on_end` フィールド追加
- ✅ `clock.rs`: `extend_range_end` / `shrink_range_end` / `set_seek_to_start_on_end` 追加
- ✅ `clock.rs`: `tick()` で `seek_to_start_on_end` を処理
- ✅ `controller.rs`: `StepForward` Playing 時の挙動追加
- ✅ `controller.rs`: `StepBackward` Playing 時の挙動変更
- ✅ `cargo check` / `cargo clippy` 通過

---

## バグ修正（レビューで発見）

### B-1: `seek_to_start_on_end` 発火時に `reached_end` が伝播されない

**原因**: `tick()` が逆転レンジ (`prev..range.start`) を返し、`dispatcher.rs` の `is_empty()` チェックで早期リターンされ `reached_end = false` のままになっていた。

**修正** (`dispatcher.rs`): 逆転レンジ (`range.start > range.end`) かつ Paused の場合のみ `reached_end = true` を返すよう変更。

### B-2: `pause()` が `seek_to_start_on_end` フラグをクリアしない

**原因**: StepForward(Playing) → StepBackward(pause) → Resume の操作で、フラグが残存したまま再 play すると次の終端到達で意図せず `range.start` へジャンプしていた。

**修正** (`clock.rs`: `pause()`): `pause()` で `seek_to_start_on_end = false` をクリア。

---

## ユニットテスト追加 (`src/replay/clock.rs`)

追加テスト数: 22 件

- `extend_range_end`: 5 件（基本動作、累積、ステータス保持、延長後の継続・停止）
- `shrink_range_end`: 6 件（基本、戻り値、クランプ、境界値、ゼロ起点、連打）
- `seek_to_start_on_end`: 7 件（リセット、Pause、非ゼロ start、フラグリセット、false 時、連続 tick、その後の空レンジ）
- 組み合わせ: 2 件（extend + seek_to_start フル通し、複数 extend）
- `pause()` によるフラグクリア: 2 件

- ✅ `cargo test` 191 tests passed
- ✅ `cargo clippy -- -D warnings` 通過

---

## 追加バグ修正（2026-04-15）

### B-3: Playing 中に ⏮ を押しても start に戻らない

**ブランチ**: `sasa/develop`  
**対象ファイル**: `src/replay/controller.rs`, `src/replay/clock.rs`

---

#### 現象

再生 (`▶`) 中に ⏮ を押すと以下の誤った挙動が発生する。

**期待する挙動**:
1. 再生が停止する（`Paused`）
2. `current_time` が `start_time`（再生開始時刻）に戻る
3. 全チャートが `start_time` の状態に戻る

**実際の挙動**:
1. 再生は停止する（`Paused`）✓
2. `current_time` が `start_time` ではなく **`end_time - step_size`（range の末尾から 1 bar 戻った位置）** にシークされる ✗
3. チャートも `start_time` ではなく上記の誤った時刻に更新される ✗
4. 副作用として `range.end` 自体が `step_size` 分縮小される ✗（ユーザーが設定した終了時刻が変わってしまう）

**再現例**（HTTP API で確認）:

```
⏮ 実行前:
  status:       Playing
  current_time: 1760745600000   ← 再生途中の時刻
  start_time:   1743465600000   ← 2025-04-01 00:00
  end_time:     1775005200000   ← 2026-04-01 01:00

⏮ 実行後:
  status:       Paused          ← 停止は正しい
  current_time: 1774918800000   ← end - 1day（start に戻っていない）✗
  end_time:     1774918800000   ← range.end も縮小された ✗
```

UI では左上の `current time` 表示が `2026-03-31`（= end_time - 1 day）となり、start（`2025-04-01`）に戻らない。

---

#### 根本原因

`src/replay/controller.rs` の `StepBackward` ハンドラにある Playing 分岐のロジックが誤っていた。

**問題のコード（修正前）**:

```rust
if self.state.is_playing() {
    // Playing 中: range.end を 1 bar 縮小 → 停止 → 新終端位置へシーク
    let new_end = if let Some(clock) = &mut self.state.clock {
        let new_end = clock.shrink_range_end(step_size);  // ← range.end を縮小
        clock.pause();
        clock.seek(new_end);  // ← start ではなく縮小後の end にシーク
        new_end
    } else {
        return (Task::none(), None);
    };

    dashboard.reset_charts_for_seek(main_window_id);
    self.inject_klines_up_to(new_end, dashboard, main_window_id);  // ← start ではなく new_end まで注入
    return (Task::none(), None);
}
```

このコードは当初「⏮ = range を 1 bar 縮小して新終端に止まる」という仕様（`feat: 再生中の ⏭/⏮ 挙動を修正し range 操作に変更`）で実装されたが、仕様が「⏮ = 初期状態（start）に戻って停止する」に変更されたため、実装と仕様が乖離していた。

---

#### 修正内容

**`src/replay/controller.rs` — StepBackward の Playing 分岐を書き換え**:

```rust
if self.state.is_playing() {
    // Playing 中: 停止して start に戻す
    let start_ms = self
        .state
        .clock
        .as_ref()
        .map(|c| c.full_range().start)
        .unwrap_or(0);
    if let Some(clock) = &mut self.state.clock {
        clock.pause();          // ⏸ 停止
        clock.seek(start_ms);   // current_time → start
    }
    dashboard.reset_charts_for_seek(main_window_id);          // チャートリセット
    self.inject_klines_up_to(start_ms, dashboard, main_window_id);  // start までのデータ注入
    return (Task::none(), None);
}
```

変更点:
- `shrink_range_end(step_size)` を **削除**（range.end を変更しない）
- `seek(new_end)` → `seek(start_ms)` に変更（start_ms は `clock.full_range().start`）
- `inject_klines_up_to(new_end)` → `inject_klines_up_to(start_ms)` に変更

**`src/replay/clock.rs` — `shrink_range_end` メソッドを削除**:

`controller.rs` から参照がなくなった `shrink_range_end` メソッドおよびそのユニットテスト 7 件を削除した（dead code）。

---

#### 検証

- `cargo clippy -- -D warnings`: 警告・エラーなし ✅
- `cargo test`: リプレイ関連テスト全件パス ✅

---

### 進捗

- ✅ B-3 現象確認（API 検証 + 目視確認）
- ✅ `controller.rs`: StepBackward Playing 分岐を修正
- ✅ `clock.rs`: 不要になった `shrink_range_end` とテストを削除
- ✅ `cargo clippy -- -D warnings` 通過
- ✅ `controller.rs`: B-3 ユニットテスト 3 件追加（`cargo test` 187 passed）
  - `step_backward_while_playing_pauses_clock`
  - `step_backward_while_playing_seeks_to_range_start`
  - `step_backward_while_playing_preserves_range_end`
- ✅ `docs/replay_header.md`: §6.5 に Playing 中 ⏮ の挙動を追記
- ✅ `controller.rs`: B-3 修正に対するユニットテスト 3 件を追加

---

## ユニットテスト追加（B-3: `src/replay/controller.rs`）

**追加テスト数**: 3 件（`#[cfg(test)] mod tests`）

### テスト一覧

| テスト名 | 検証内容 |
|---|---|
| `step_backward_while_playing_pauses_clock` | Playing 中に ⏮ → clock.status が Paused になること |
| `step_backward_while_playing_seeks_to_range_start` | Playing 中に ⏮ → current_time が range.start に戻ること |
| `step_backward_while_playing_preserves_range_end` | Playing 中に ⏮ → range.end が変化しないこと |

### 設計メモ

- `handle_message` が `&mut Dashboard` を要求するため、テストでは `Dashboard::default()` と `window::Id::unique()` を使用した（`dashboard.rs` の既存テストと同じパターン）。
- `active_streams` が空のため、`inject_klines_up_to` と `reset_charts_for_seek` はどちらも実質 no-op になる。これにより GUI 依存なしでクロック状態だけを純粋に検証できる。
- `seeks_to_range_start` テストでは clock を 200ms tick して `now_ms` を `start_ms` から離してから ⏮ を押し、pre-condition assertion で事前に `now_ms != start_ms` を確認している。
- `handle_message` の戻り値 `(Task<_>, Option<Toast>)` は `let _ =` で破棄（テストでは使わないが `#[must_use]` 警告を抑制するため必要）。

### 最終確認

- `cargo test`: 187 tests passed ✅
- `cargo clippy -- -D warnings`: 警告・エラーなし ✅
