# Live モード時のリプレイヘッダー簡略化

**作成日**: 2026-04-16
**ブランチ**: sasa/develop
**ステータス**: ✅ 完了

---

## 目的

Live モード時に不要なコントロールを非表示にし、ヘッダーをすっきりさせる。

---

## 現状

`view_replay_header()` は常時以下を全て表示している:

```
🕐 時刻  [LIVE/REPLAY]  [開始入力 ~ 終了入力]  ⏮ ▶⏸ ⏭ 1x
```

Live モードでは入力テキストボックスは read-only、再生ボタン類は `on_press` なし（無効化）の
状態で表示されているが、クリックできないのに見える = 混乱の元。

---

## 修正方針

`src/main.rs` の `view_replay_header()` を修正する。

### Live モード時

```
🕐 時刻  [LIVE]
```

表示するのは現在時刻 + モードトグルボタンのみ。
以下を **非表示（= row に push しない）**:
- 開始日時 text_input
- `~` セパレータ
- 終了日時 text_input
- ⏮ ▶/⏸ ⏭ 速度 の controls row

### Replay モード時

変更なし（現行と同じ）:

```
🕐 仮想時刻  [REPLAY]  [開始入力 ~ 終了入力]  ⏮ ▶⏸ ⏭ 1x  [Loading...]
```

---

## 実装

### 変更対象: `src/main.rs` `view_replay_header()`

現状の `row![..., start_input, "~", end_input, controls]` を常時構築していたのを、
`is_replay` が true のときのみ push する形に書き換える。

```rust
let mut header = row![time_display, mode_toggle];

if is_replay {
    header = header
        .push(start_input.width(140))
        .push(text("~").size(11))
        .push(end_input.width(140))
        .push(controls);
    if is_loading {
        header = header.push(text("Loading...").size(11));
    }
}
```

---

## 影響範囲

| 項目 | 影響 |
|---|---|
| UI 表示 | Live 時にヘッダーが短くなる（設計意図通り）|
| ロジック | なし（表示だけの変更）|
| テスト | なし（view 関数は単体テスト対象外）|
| E2E テスト | ヘッダー要素の存在確認をしているテストがあれば要確認 |

---

## 実装ステップ

- ✅ `src/main.rs`: `view_replay_header()` を修正
- ✅ `cargo check` パス
- [ ] 目視確認（Live / Replay 切替でヘッダーが切り替わること）

---

## 完了条件

1. Live モード: 時刻 + `[LIVE]` ボタンのみ表示
2. Replay モード: 従来通り全コントロールが表示される
3. `cargo check` パス
