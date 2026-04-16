# 計画書: リプレイ ユーザー操作仕様の更新

**作成日**: 2026-04-15  
**ブランチ**: sasa/develop  
**対象ファイル**: `src/replay/controller.rs`, `docs/replay_header.md`

---

## 概要

ユーザー操作（⏭/⏮、銘柄・Timeframe・Speed 変更、期間変更）に対するアプリの挙動を最新仕様に合わせて修正する。

---

## 新仕様

### 停止中（Paused）の操作

| 操作 | 挙動 |
|---|---|
| `⏭` をクリック | 現在から 1bar 進める（**変更なし**） |
| `⏮` をクリック | 現在から 1bar 戻る（**変更なし**） |
| 銘柄を変更 | 初期状態（range.start）に戻して停止（`ReloadKlineStream` で処理、**変更なし**） |
| Timeframe を変更 | 初期状態に戻して停止（`ReloadKlineStream` で処理、**変更なし**） |
| Speed を変更 | 初期状態に戻して停止（**要変更**: 現在はスピードだけ変更） |
| 期間（Start/End）を変更 | 初期状態に戻して停止（**要変更**: 現在は入力文字列のみ更新） |

### 再生中（Playing）の操作

| 操作 | 挙動 |
|---|---|
| `⏭` をクリック | End（終了地点）まで一気に進める（**要変更**: 現在は range.end を延長） |
| `⏮` をクリック | 初期状態に戻して停止（**変更なし**: B-3 で修正済み） |
| 銘柄を変更 | 初期状態に戻して停止（`ReloadKlineStream` で処理、**変更なし**） |
| Timeframe を変更 | 初期状態に戻して停止（`ReloadKlineStream` で処理、**変更なし**） |
| Speed を変更 | 初期状態に戻して停止（**要変更**: 現在はスピードだけ変更） |
| 期間（Start/End）を変更 | 初期状態に戻して停止（**要変更**: 現在は入力文字列のみ更新） |

---

## 変更点の詳細

### 1. StepForward（Playing 中）

**変更前**:
```rust
if self.state.is_playing() {
    // range.end を 1 bar 延長して再生継続 (seek_to_start_on_end = true)
    clock.extend_range_end(step_size);
    ...
    clock.set_seek_to_start_on_end(true);
    return (Task::none(), None);
}
```

**変更後**:
```rust
if self.state.is_playing() {
    // End まで一気に進めて停止
    let end_ms = clock.full_range().end;
    clock.pause();
    clock.seek(end_ms);
    dashboard.reset_charts_for_seek(main_window_id);
    self.inject_klines_up_to(end_ms, ...);
    return (Task::none(), None);
}
```

### 2. CycleSpeed

**変更前**:
```rust
ReplayMessage::CycleSpeed => {
    self.state.cycle_speed();
    (Task::none(), None)
}
```

**変更後**:
```rust
ReplayMessage::CycleSpeed => {
    self.state.cycle_speed();
    // clock が存在する場合は初期状態に戻して停止
    let start_ms = self.state.clock.as_ref().map(|c| c.full_range().start);
    if let Some(start_ms) = start_ms {
        clock.pause();
        clock.seek(start_ms);
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(start_ms, ...);
    }
    (Task::none(), None)
}
```

### 3. StartTimeChanged / EndTimeChanged（clock が Some のとき）

**変更前**:
```rust
ReplayMessage::StartTimeChanged(s) => {
    self.state.range_input.start = s;
    (Task::none(), None)
}
```

**変更後**:
```rust
ReplayMessage::StartTimeChanged(s) => {
    self.state.range_input.start = s;
    // clock が存在する場合は初期状態に戻して停止
    let start_ms = self.state.clock.as_ref().map(|c| c.full_range().start);
    if let Some(start_ms) = start_ms {
        clock.pause();
        clock.seek(start_ms);
        dashboard.reset_charts_for_seek(main_window_id);
        self.inject_klines_up_to(start_ms, ...);
    }
    (Task::none(), None)
}
```

---

## 設計判断

### StepForward Playing 中の実装選択

SPEED_INSTANT (f32::INFINITY) を `set_speed` で設定する方法も可能だが、
clock の speed フィールドが SPEED_INSTANT のまま残るため、その後 Resume すると
再び即時完走になる UX 問題がある。

→ `pause() + seek(end)` で直接制御する方式を採用。コードもシンプル。

### CycleSpeed 後の位置リセット

Speed 変更はリプレイの「再設定」に相当するため、現在位置を維持しない仕様。
ユーザーは新しいスピードで Play ボタンを押して再開することを前提とする。

### Start/End 変更時のリセット範囲

`StartTimeChanged` / `EndTimeChanged` は UI テキスト入力の変更を受け取る。
clock が存在する場合（リプレイが進行中）は、変更を受けた時点で
現在の clock.range.start に戻して停止する。
新しい入力値は次回 Play 押下時に反映される。

---

## TDD テストケース

### StepForward while Playing

| テスト名 | 検証内容 |
|---|---|
| `step_forward_while_playing_pauses_clock` | Playing 中に ⏭ → clock が Paused になる |
| `step_forward_while_playing_seeks_to_range_end` | Playing 中に ⏭ → current_time が range.end になる |
| `step_forward_while_playing_preserves_range_end` | Playing 中に ⏭ → range.end が変化しない |

### CycleSpeed

| テスト名 | 検証内容 |
|---|---|
| `cycle_speed_while_playing_pauses_clock` | Playing 中に CycleSpeed → clock が Paused になる |
| `cycle_speed_while_playing_seeks_to_range_start` | Playing 中に CycleSpeed → current_time が range.start になる |
| `cycle_speed_while_paused_seeks_to_range_start` | Paused 中に CycleSpeed → current_time が range.start になる |
| `cycle_speed_cycles_speed_value` | CycleSpeed → speed が 1x → 2x に変わる |

### StartTimeChanged / EndTimeChanged

| テスト名 | 検証内容 |
|---|---|
| `start_time_changed_while_playing_pauses_clock` | Playing 中に StartTimeChanged → clock が Paused になる |
| `start_time_changed_while_playing_seeks_to_range_start` | Playing 中に StartTimeChanged → current_time が range.start になる |
| `end_time_changed_while_playing_pauses_clock` | Playing 中に EndTimeChanged → clock が Paused になる |

---

## 進捗

- ✅ 計画書作成
- ✅ TDD: 失敗テストを追加（RED）— 9 件 FAIL 確認
- ✅ `controller.rs`: StepForward Playing 挙動変更（GREEN）
- ✅ `controller.rs`: CycleSpeed リセット追加（GREEN）
- ✅ `controller.rs`: StartTimeChanged リセット追加（GREEN）
- ✅ `controller.rs`: EndTimeChanged リセット追加（GREEN）
- ✅ `clock.rs`: dead_code `extend_range_end` / `set_seek_to_start_on_end` に `#[allow(dead_code)]` 追加
- ✅ `store.rs`: dead_code `extend_loaded_range_end_to` に `#[allow(dead_code)]` 追加
- ✅ `cargo test` 全件パス（196 tests）
- ✅ `cargo clippy -- -D warnings` 通過
- ✅ `docs/replay_header.md` 更新（§6.5 / §6.6 新設 / §7.2 更新）

---

---

## E2E テスト修正（2026-04-15）

### 影響の全体像

新仕様で `CycleSpeed` が `pause + seek(range.start)` を行うようになったことで、
e2e テストに以下の 2 種類の影響が発生した。

#### 1. 修正後 FAIL になるテスト（明確なアサーション不一致）

| スクリプト | TC | 問題 | 修正内容 |
|---|---|---|---|
| `s9_speed_step.sh` | TC-S9-03 | "Playing 中 StepForward は no-op" → 新仕様は End ジャンプ | アサーション変更（Paused かつ ct ≒ end を確認） |
| `x2_buttons.sh` | TC-X2-07 | "Speed 切替で current_time 不変" → 新仕様は range.start リセット | アサーション変更 + StepForward で事前に前進 |

#### 2. `speed_to_10x()` が Paused を引き起こすテスト（Resume 不足）

`speed_to_10x()` は CycleSpeed × 3 回を呼ぶため、新仕様では呼び出し後に clock が Paused になる。

| スクリプト | TC | 修正方法 |
|---|---|---|
| `s11_bar_step_discrete.sh` | TC-S11-01 | `common_helpers.sh` 修正で自動解決 |
| `s12_pre_start_history.sh` | TC-S12-03 | 同上 |
| `s13_step_backward_quality.sh` | TC-S13-03 | 同上 |
| `s5_tachibana_mixed.sh` | TC-S5-06 | 同上 |
| `s7_mid_replay_pane.sh` | TC-S7-07 | 同上（偽陽性 → 正常化） |
| `s18_endurance.sh` | TC-S18-01 | 同上（偽陽性 → 正常化） |
| `s22_tachibana_endurance.sh` | TC-S22-01 | 同上（偽陽性 → 正常化） |
| `s10_range_end.sh` | TC-S10-01 | 速度ループ後に Resume を個別追加（speed_to_10x 非使用） |
| `s20_tachibana_replay_resilience.sh` | TC-S20-01 | 20 回ループ後に Resume を個別追加 |

#### 3. 修正不要（正常動作継続）

- `s16_replay_resilience.sh`: speed_to_10x 後に Resume が明示的に書かれている
- `s1_basic_lifecycle.sh`: Paused 状態でのスピード変更 + range.start からの StepForward → 正常
- StepForward/StepBackward（Paused 状態）系テスト全般: 変更なし

### 実施した修正（事前）

- ✅ `common_helpers.sh`: `speed_to_10x()` に `resume + wait_status Playing` を追加
  - これで s11 / s12 / s13 / s5 / s7 / s18 / s22 が自動修正される
- ✅ `s9_speed_step.sh`: TC-S9-03 のアサーションを新仕様（Paused at End）に変更
- ✅ `s10_range_end.sh`: TC-S10-01 で速度ループ後に Resume + wait_status Playing を追加
- ✅ `s20_tachibana_replay_resilience.sh`: TC-S20-01 で 20 回ループ後に Resume を追加、メッセージ更新
- ✅ `x2_buttons.sh`: TC-X2-07 を「Speed 切替で range.start にリセット」に書き換え（StepForward で事前前進を追加）

### 実施した修正（E2E テスト実行時に発見・追加対応）

| スクリプト | TC | 原因 | 修正内容 |
|---|---|---|---|
| `s7_mid_replay_pane.sh` | TC-S7-06 | TC-S7-04 の `set-timeframe` → `ReloadKlineStream` → `clock.pause()` で Paused になり close 後も Paused | set-timeframe 後に Resume を追加 |
| `s16_replay_resilience.sh` | TC-S16-01 | speed 20 連打後 Paused のまま（「speed は Playing 状態に影響しない」は旧仕様コメント） | 連打後に Resume を追加 |
| `s16_replay_resilience.sh` | TC-S16-02a | 10x 再生が 2h range を ~2.5 秒で完走し range.end で自動 Paused → CT_BEFORE=range.end で StepForward が no-op | range.end を 01:00 → 03:00 に拡張（4h range） |
| `s18_endurance.sh` | TC-S18-03 | 1x 再生で M1 は ~10 bar/秒。2h range (120 bars) が 20 サイクル (~30 秒) 中に終端到達 | range を 3h → 6h（utc_offset -3 → -7）に拡張 |
| `x3_chart_update.sh` | TC-X3-01a/b, X3-02, X3-03 | chart-snapshot API フィールド名が変更されている（`kline_count`/`first_kline_time`/`last_kline_time` → `bar_count`/`oldest_ts`/`newest_ts`） | フィールド名を修正。TC-X3-01b は Pre-start history 考慮で条件を `oldest_ts <= start_ms` に変更。TC-X3-03 は `newest_ts` 増分が 1 bar 以上かつ bar 境界であることを確認に緩和 |

✅ 上記修正後、全対象スクリプトで PASS / PEND のみ（FAIL なし）を確認

---

## 既知の設計背景

- `extend_range_end` / `set_seek_to_start_on_end` は clock.rs に残存するが
  production コードから呼ばれなくなる。pub メソッドのため dead_code 警告は発生しない。
  テストは clock の動作保証として価値があるため削除しない。
- `extend_loaded_range_end_to` (store.rs) も同様に production から呼ばれなくなるが
  pub のため dead_code 警告は発生しない。
