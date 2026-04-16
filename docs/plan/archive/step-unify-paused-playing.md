# StepForward / StepBackward 挙動統一

## 目的

`[Paused / Waiting]` 状態での `StepForward` / `StepBackward` の「1バー前進 / 1バー後退」挙動を廃止し、
`[Playing]` 状態と同一の挙動（`range.end` へジャンプ / `range.start` へ巻き戻し）に統一する。

---

## 現在の挙動（変更前）

| 状態 | StepForward | StepBackward |
|:--|:--|:--|
| Playing | `range.end` へシーク → Pause | `range.start` へシーク → Pause |
| Paused | `current_time + min_timeframe_ms` (1バー前進) | 前の kline 時刻へシーク (1バー後退) |
| Waiting | no-op (Paused ガードで早期 return) | 前の kline 時刻へシーク (1バー後退) |
| Idle | no-op | no-op |

## 目標の挙動（変更後）

| 状態 | StepForward | StepBackward |
|:--|:--|:--|
| Playing | `range.end` へシーク → Pause（変更なし） | `range.start` へシーク → Pause（変更なし） |
| Paused | `range.end` へシーク | `range.start` へシーク |
| Waiting | `range.end` へシーク | `range.start` へシーク |
| Idle | no-op（変更なし） | no-op（変更なし） |

---

## 変更ファイル一覧

| ファイル | 変更種別 |
|:--|:--|
| `src/replay/controller.rs` | ハンドラ実装の簡略化、docstring 更新、テスト追加 |
| `src/replay/mod.rs` | `compute_step_backward_target` 関数 + テスト削除 |
| `docs/replay_header.md` | §6.5 仕様記述の更新 |

---

## 実装詳細

### 1. `src/replay/controller.rs` — StepForward ハンドラ（現 L370-408）

**変更前（38 行）:**
```rust
ReplayUserMessage::StepForward => {
    let step_size = match &self.state.session {
        ReplaySession::Loading { active_streams, .. }
        | ReplaySession::Active { active_streams, .. } => {
            min_timeframe_ms(active_streams)
        }
        ReplaySession::Idle => return (Task::none(), None),
    };

    if self.state.is_playing() {
        let end_ms = match &self.state.session {
            ReplaySession::Active { clock, .. } => clock.full_range().end,
            _ => 0,
        };
        self.seek_to(end_ms, dashboard, main_window_id);
        return (Task::none(), None);
    }

    // Paused 時のみ位置を 1 bar 前進する
    if !self.state.is_paused() {
        return (Task::none(), None);
    }

    let current_time = self.state.current_time();
    let new_time = current_time + step_size;

    if let ReplaySession::Active { clock, .. } = &mut self.state.session {
        let range_end = clock.full_range().end;
        if new_time > range_end {
            return (Task::none(), None);
        }
        clock.seek(new_time);
    }

    self.inject_klines_up_to(new_time, dashboard, main_window_id);
    (Task::none(), None)
}
```

**変更後（7 行）:**
```rust
ReplayUserMessage::StepForward => {
    let end_ms = match &self.state.session {
        ReplaySession::Active { clock, .. } => clock.full_range().end,
        _ => return (Task::none(), None),
    };
    self.seek_to(end_ms, dashboard, main_window_id);
    (Task::none(), None)
}
```

---

### 2. `src/replay/controller.rs` — StepBackward ハンドラ（現 L416-454）

**変更前（38 行）:**
```rust
ReplayUserMessage::StepBackward => {
    if self.state.is_playing() {
        let start_ms = match &self.state.session {
            ReplaySession::Active { clock, .. } => clock.full_range().start,
            _ => 0,
        };
        self.seek_to(start_ms, dashboard, main_window_id);
        return (Task::none(), None);
    }

    // Paused 時: 1 bar 前の位置へシーク
    let current_time = self.state.current_time();
    let (prev_time, start_ms) = match &self.state.session {
        ReplaySession::Active { clock, store, active_streams, .. } => {
            let prev = active_streams
                .iter()
                .filter_map(|stream| {
                    let klines = store.klines_in(stream, 0..current_time);
                    klines.iter().rev().find(|k| k.time < current_time).map(|k| k.time)
                })
                .max();
            (prev, clock.full_range().start)
        }
        _ => return (Task::none(), None),
    };
    let new_time = super::compute_step_backward_target(prev_time, current_time, start_ms);
    self.seek_to(new_time, dashboard, main_window_id);
    (Task::none(), None)
}
```

**変更後（7 行）:**
```rust
ReplayUserMessage::StepBackward => {
    let start_ms = match &self.state.session {
        ReplaySession::Active { clock, .. } => clock.full_range().start,
        _ => return (Task::none(), None),
    };
    self.seek_to(start_ms, dashboard, main_window_id);
    (Task::none(), None)
}
```

---

### 3. `src/replay/controller.rs` — `seek_to` docstring（現 L655-661）

```rust
// 変更前
/// StepForward/StepBackward (Playing 時)、StepBackward (Paused 時)、
/// および handle_range_input_change から呼ぶ。
///
/// # 対象外
/// - `ReloadKlineStream`: reset_charts → ロード → 注入の順序が異なる
/// - `StepForward` (Paused 時): pause も chart reset も不要（前進のみ）

// 変更後
/// StepForward / StepBackward（状態を問わず）、および handle_range_input_change から呼ぶ。
///
/// # 対象外
/// - `ReloadKlineStream`: reset_charts → ロード → 注入の順序が異なる
```

---

### 4. `src/replay/mod.rs` — `compute_step_backward_target` 削除

- 関数定義（L53-61）を削除
- 関数の単体テスト（`// ── compute_step_backward_target ──` セクション）を削除
- `min_timeframe_ms` は他箇所（L276, L542, L568）で使用継続のため **残す**

---

### 5. テスト追加（`controller.rs` の `#[cfg(test)]` 内）

**新規追加テスト（Paused 状態）:**

| テスト名 | 検証内容 |
|:--|:--|
| `step_forward_while_paused_seeks_to_range_end` | current_time が `range.end` に移動すること |
| `step_forward_while_paused_preserves_range_end` | range.end が変化しないこと |
| `step_forward_while_paused_leaves_clock_paused` | clock が Paused のままであること |
| `step_backward_while_paused_seeks_to_range_start` | current_time が `range.start` に移動すること |
| `step_backward_while_paused_preserves_range_end` | range.end が変化しないこと |
| `step_backward_while_paused_leaves_clock_paused` | clock が Paused のままであること |

**既存テスト（Playing 状態）は変更なし:**
- `step_backward_while_playing_pauses_clock`
- `step_backward_while_playing_seeks_to_range_start`
- `step_backward_while_playing_preserves_range_end`
- `step_forward_while_playing_pauses_clock`
- `step_forward_while_playing_seeks_to_range_end`
- `step_forward_while_playing_preserves_range_end`

---

### 6. `docs/replay_header.md` — §6.5 更新（現 L468-502）

```
### 6.5 StepForward / StepBackward

[StepForward — Playing / Paused / Waiting 中]
  ├─ clock.pause()（Playing 時のみ）
  ├─ clock.seek(range.end)              ← current_time を End まで一気に移動
  ├─ dashboard.reset_charts_for_seek(main_window)
  └─ inject_klines_up_to(range.end)
     range.end は変更しない
     kline 再フェッチは行わない — EventStore から再構成

[StepBackward — Playing / Paused / Waiting 中]
  ├─ clock.pause()（Playing 時のみ）
  ├─ clock.seek(range.start)            ← current_time を start に戻す
  ├─ dashboard.reset_charts_for_seek(main_window)
  └─ inject_klines_up_to(range.start)
     range.end は変更しない
     kline 再フェッチは行わない — EventStore から再構成

[Idle 中]
  └─ no-op（変更なし）
```

---

## 実装順序（TDD）

- [ ] §5: `compute_step_backward_target` 削除（`mod.rs`）
- [ ] §1: StepForward ハンドラ簡略化（`controller.rs`）
- [ ] §2: StepBackward ハンドラ簡略化（`controller.rs`）
- [ ] §3: `seek_to` docstring 更新
- [ ] §テスト: Paused 状態テスト 6 本追加
- [ ] §6: `docs/replay_header.md` §6.5 更新
- [ ] `cargo test` 全テスト通過確認
- [ ] `cargo clippy -- -D warnings` クリーン確認
