# リプレイ日付範囲 Live モード中の保持

**作成日**: 2026-04-16
**ブランチ**: sasa/develop
**ステータス**: ✅ 完了

---

## 問題

Replay モードで `range_start` / `range_end` を設定してから LIVE モードに切り替えると、
`toggle_mode()` 内で `range_input = ReplayRangeInput::default()` が実行され、
日付が空文字列にリセットされる。

その結果:
- Live モードで `POST /api/app/save` や終了時の自動保存が走ると `saved-state.json` の
  `range_start` / `range_end` が空文字列になる
- 再び Replay モードに戻したとき日付が消えている

```json
// 期待: Live モード保存後も日付が残る
"replay": {
  "mode": "live",
  "range_start": "2026-04-10 04:49",
  "range_end": "2026-04-15 06:49"
}

// 現状: リセットされてしまう
"replay": {
  "mode": "live",
  "range_start": "",
  "range_end": ""
}
```

---

## 根本原因

`src/replay/mod.rs` の `toggle_mode()`:

```rust
ReplayMode::Replay => {
    self.mode = ReplayMode::Live;
    self.session = ReplaySession::Idle;
    self.range_input = ReplayRangeInput::default();  // ← ここが原因
    self.pending_auto_play = false;
}
```

`range_input` は「ユーザーが入力したテキスト」であり、モード遷移とは独立して保持すべき値。
セッション状態 (`session`) と auto-play フラグ (`pending_auto_play`) だけをリセットすれば十分。

---

## 修正方針

### 変更ファイル: `src/replay/mod.rs`

**1. `toggle_mode()` の修正（1行削除）**

```rust
// Before
ReplayMode::Replay => {
    self.mode = ReplayMode::Live;
    self.session = ReplaySession::Idle;
    self.range_input = ReplayRangeInput::default();  // ← 削除
    self.pending_auto_play = false;
}

// After
ReplayMode::Replay => {
    self.mode = ReplayMode::Live;
    self.session = ReplaySession::Idle;
    self.pending_auto_play = false;
}
```

**2. テストの更新**

`toggle_mode_switches_replay_to_live_and_resets` の検証内容を更新:
- range_input が**空にならない**（保持される）ことを確認
- session が Idle になること・mode が Live になることは変わらず確認

---

## 影響範囲の整理

| コンポーネント | 影響 | 理由 |
|---|---|---|
| `toggle_mode()` | 修正対象 | range_input リセット削除 |
| `data/src/config/state.rs` | 変更なし | 既に `mode: "live"` + 有効 range の組み合わせをサポート済み（テストあり） |
| `src/main.rs` 起動時 | 変更なし | `pending_auto_play = mode == "replay" && has_valid_range` のため Live モード時は auto-play 発火しない |
| 保存ロジック (`save_state`) | 変更なし | `range_input_start()` / `range_input_end()` をそのまま使用、range が空でなければ保存される |

### Live モードで range が残ることの副作用確認

- **UI 表示**: Live モード中は `range_input` を表示しない（read-only テキストで `on_input = None`）→ 表示上問題なし
- **auto-play**: `pending_auto_play = mode == "replay" && has_valid_range` のため、Live で保存後の次回起動は Live 起動 → auto-play 発火しない
- **Replay 再開**: Live → Replay トグル時は `ReplayMode::Live => { self.mode = ReplayMode::Replay; }` のみ実行され、保持された range_input がそのまま使われる ✅

---

## 実装ステップ

- ✅ `src/replay/mod.rs`: `toggle_mode()` の `self.range_input = ReplayRangeInput::default();` を削除
- ✅ `src/replay/mod.rs`: `toggle_mode_switches_replay_to_live_and_resets` テストを更新（range 保持を確認するテストも追加）
- ✅ `cargo test replay` で 163 件パス確認

---

## 完了条件

1. Live モードに切り替えた後、`POST /api/app/save` を実行しても `saved-state.json` の `range_start` / `range_end` が保持される
2. その後 Replay モードに戻すと、日付入力テキストが復元されている
3. 既存テスト全件パス
