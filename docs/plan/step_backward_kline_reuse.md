# StepBackward Kline 再フェッチ撤廃 — バッファ再利用プラン

## 問題

⏮（StepBackward）を押すたびに **Kline を全範囲再フェッチ** している。
StepForward（⏭）はバッファから進めるだけで即座に完了するのに対し、
StepBackward はネットワーク往復が発生し体感で重い。

### 根本原因

```
StepBackward
  → prepare_replay()
    → rebuild_content_for_replay()
      → KlineChart::new(..., &[], ...)   ← 空データで再生成
      → enable_replay_mode()             ← 空の ReplayKlineBuffer を作成
  → kline_fetch_task(start..end)         ← 同じデータを再取得
  → DataLoaded → replay_advance(new_time)
```

`rebuild_content_for_replay()` が KlineChart ごと破棄するため、
内部の `ReplayKlineBuffer`（既に全 kline を保持済み）も消えてしまう。

---

## 方針

rebuild 前に `ReplayKlineBuffer` を退避し、新チャートに復元する。
フェッチを完全に排除し、StepForward と同等の軽量処理にする。

### Before / After

| | Before (現状) | After (修正後) |
|---|---|---|
| チャート再構築 | `rebuild_content_for_replay()` で空データ生成 | `rebuild_content_for_step_backward()` でバッファ保持 |
| kline データ | 再フェッチ（ネットワーク） | バッファ退避→復元（メモリ内） |
| kline 挿入 | `DataLoaded` 後に `replay_advance` | 即座に `replay_advance(new_time)` |
| ステータス遷移 | `Loading` → `DataLoaded` → `Paused` | 即座に `Paused` |
| 所要時間 | 数百ms〜数秒（ネットワーク依存） | 即座 |

---

## 修正対象ファイル

| ファイル | 変更内容 |
|---|---|
| `src/chart/kline.rs` | `replay_kline_buffer` フィールドを `pub(crate)` に変更 |
| `src/screen/dashboard/pane.rs` | `rebuild_content_for_step_backward` メソッド追加 |
| `src/main.rs` | `StepBackward` ハンドラからフェッチ削除、バッファ保持版リビルドに置換 |

---

## 実装ステップ

### ✅ Step 1: ReplayKlineBuffer の可視性変更

**ファイル**: `src/chart/kline.rs`

`ReplayKlineBuffer` 構造体と `replay_kline_buffer` フィールドを `pub(crate)` にする。

```rust
/// リプレイ用 kline バッファ
pub(crate) struct ReplayKlineBuffer {
    pub(crate) klines: Vec<Kline>,
    pub(crate) cursor: usize,
}

// KlineChart 内
pub(crate) replay_kline_buffer: Option<ReplayKlineBuffer>,
```

### ✅ Step 2: PaneState にバッファ保持版リビルドを追加

**ファイル**: `src/screen/dashboard/pane.rs`

`rebuild_content` でチャートを再生成する前にバッファを退避し、再生成後に復元する。
中間レイヤー不要で、1 メソッドに集約する。

```rust
/// StepBackward 用: チャートをリビルドしつつ kline バッファを保持する。
pub fn rebuild_content_for_step_backward(&mut self) {
    // バッファを退避
    let saved_buf = match &mut self.content {
        Content::Kline { chart, .. } => {
            chart.as_mut().and_then(|c| c.replay_kline_buffer.take())
        }
        _ => None,
    };

    self.rebuild_content(true);

    // バッファを復元（cursor=0 にリセット）
    if let (Content::Kline { chart, .. }, Some(mut buf)) = (&mut self.content, saved_buf) {
        if let Some(c) = chart.as_mut() {
            buf.cursor = 0;
            c.replay_kline_buffer = Some(buf);
        }
    }
}
```

### ✅ Step 3: StepBackward ハンドラを書き換え

**ファイル**: `src/main.rs` — `ReplayMessage::StepBackward` ブロック（L958-993）

```rust
ReplayMessage::StepBackward => {
    if let Some(pb) = &mut self.replay.playback {
        let step_ms = 60_000u64;
        let new_time = pb.current_time.saturating_sub(step_ms).max(pb.start_time);
        pb.current_time = new_time;

        // TradeBuffer のカーソルをリセットし、new_time まで早送り
        for buffer in pb.trade_buffers.values_mut() {
            buffer.cursor = 0;
            buffer.drain_until(new_time);
        }

        // StepBackward 後は Paused で止める（Loading 不要）
        pb.status = replay::PlaybackStatus::Paused;
    }

    let main_window_id = self.main_window.id;
    let dashboard = self.active_dashboard_mut();

    // バッファ保持版リビルド
    for (_, _, state) in dashboard.iter_all_panes_mut(main_window_id) {
        state.rebuild_content_for_step_backward();
    }

    // new_time まで kline を再挿入（フェッチ不要）
    let current_time = self.replay.playback.as_ref().map(|pb| pb.current_time);
    if let Some(ct) = current_time {
        dashboard.replay_advance_klines(ct, main_window_id);
    }

    // ← fetch_tasks / DataLoaded チェーン 完全削除
}
```

**削除されるもの**:
- `layout_id` 取得
- `kline_fetch_task` ループ
- `fetch_tasks.is_empty()` 分岐
- `Task::batch(fetch_tasks).chain(data_loaded)` の return
- `pb.resume_status` の設定（Loading を経由しないため不要）
- `pb.status = Loading` の設定

---

## リスク・注意点

1. **ReplayKlineBuffer の可視性変更**: `pub(crate)` にするだけで十分。外部クレートには公開しない。
2. **初回 `prepare_replay` への影響なし**: 既存の `rebuild_content_for_replay` は変更しない。
3. **インジケーター**: `replay_advance` 内で `on_insert_klines` が呼ばれるため、バッファ復元後の advance で正しく再計算される。
4. **非 Kline ペイン**: `saved_buf` が `None` なのでスキップされる。影響なし。
5. **kline 全量再挿入**: cursor=0 から `replay_advance(new_time)` で全 kline を再挿入するため、kline 数が非常に多い場合のコストはあるが、ネットワーク往復と比較すれば無視できる。

---

## 実装メモ

### 追加で行った変更

- `src/screen/dashboard.rs`: `rebuild_for_step_backward()` メソッド追加。`iter_all_panes_mut` が private なため Dashboard 側に public ラッパーが必要。
- `src/replay.rs`: テスト内の `PlaybackState` 初期化に `resume_status` フィールドが不足していたため全 11 箇所を修正。
- `src/chart/kline.rs`: テストモジュール追加（`buffer_cursor_reset_allows_full_reinsert`, `buffer_take_and_restore_preserves_klines`）

### 設計判断

- **`resume_status` フィールドは削除しない**: `DataLoaded` ハンドラなど他の箇所で依然使用されている。StepBackward では使わなくなっただけ。
- **StepForward と対称的な構造**: StepForward は `replay_advance_klines` のみ。StepBackward は `rebuild_for_step_backward` + `replay_advance_klines`。リビルドが必要な理由はチャートのスケール・描画状態をリセットする必要があるため。

## 検証

- ⏮ を押して即座にチャートが巻き戻ること（Loading 状態を経由しない）
- ⏮ → ▶ で正しい位置から再生が再開されること
- ⏮ を連打して start_time でクランプされること
- ⏮ → ⏭ でインジケーターが正しく描画されること
- 複数ペイン構成で各ペインが独立に正しく巻き戻ること
