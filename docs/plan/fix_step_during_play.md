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
