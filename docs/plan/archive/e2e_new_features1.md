# 計画書: mid-replay 銘柄・timeframe 変更時の自動再生防止

**作成日**: 2026-04-14  
**ブランチ**: sasa/develop  
**対象ファイル**: `src/replay/dispatcher.rs`, `src/replay/controller.rs`

---

## 概要

再生 (`▶`) 中に銘柄や timeframe を変更すると、古い再生状態が引き継がれ、
新しいチャートが自動再生されてしまうバグを修正した。

### 修正前の問題

`ReloadKlineStream` ハンドラが `clock.set_waiting()` を呼んでいたため、
ロード完了時に `try_resume_from_waiting` が自動で Playing に戻していた。
さらに `dispatch_tick` が次フレームで Paused → Waiting に強制遷移させていた。

### 修正後の仕様

| 状況 | 変更前 | 変更後 |
|---|---|---|
| Playing 中に銘柄変更 | Waiting → ロード後に **自動 Playing** | Paused → ロード後も **Paused のまま** |
| Playing 中に timeframe 変更 | Waiting → ロード後に **自動 Playing** | Paused → ロード後も **Paused のまま** |
| Paused 中に銘柄変更 | Waiting → ロード後に自動 Playing | Paused → ロード後も Paused のまま |
| 変更後にユーザーが ▶ | - | Playing に移行（正常） |
| 通常の Play ボタン押下 | Waiting → auto Playing | **変更なし** (既存動作を維持) |

---

## 実装変更

### `src/replay/dispatcher.rs`

`dispatch_tick` 内: Paused 状態のときは `set_waiting()` に遷移させない。

```rust
// 変更前
clock.set_waiting();

// 変更後
if clock.status() == ClockStatus::Playing {
    clock.set_waiting();
}
```

### `src/replay/controller.rs`

`ReloadKlineStream` ハンドラ: `set_waiting()` を削除し、`pause()` + `reset_charts_for_seek()` に変更。

```rust
// 変更前
clock.set_step_size(step_size_ms);
clock.seek(start_ms);
clock.set_waiting();

// 変更後
clock.pause();
clock.set_step_size(step_size_ms);
clock.seek(start_ms);
dashboard.reset_charts_for_seek(main_window_id);
```

---

## テスト観点

HTTP API (`127.0.0.1:9876`) 経由で操作し、各シナリオの `status` を検証する。
詳細は `/e2e-testing` スキルに従う。

### シナリオ一覧

| # | 操作シーケンス | 期待するステータス |
|---|---|---|
| A | Play → 銘柄変更 → status 確認 | `"Paused"` |
| B | Play → timeframe 変更 → status 確認 | `"Paused"` |
| C | Play → 銘柄変更 → データロード待機 → status 確認 | `"Paused"` (ロード後も) |
| D | Play → 銘柄変更 → Resume → status 確認 | `"Playing"` |
| E | Pause → 銘柄変更 → status 確認 | `"Paused"` |
| F | Play → Play (通常フロー) | `"Loading"` → `"Playing"` |
| G | Play → 銘柄変更 → 別銘柄に再変更 → Resume | `"Playing"` (正常動作) |

---

## 進捗

- ✅ 計画書作成
- ✅ `dispatcher.rs` 修正
- ✅ `controller.rs` 修正
- ✅ `cargo check` 通過
- ✅ E2E テスト実装 (`tests/e2e_scripts/s23_mid_replay_ticker_change.sh`)
- ✅ コードレビュー
