# Phase 3 Headless モード E2E テスト実装計画

## 目標

既存の E2E テストスクリプトを `IS_HEADLESS=true` 環境変数で  
**headless / GUI 両対応**にする。新しい独立スクリプトは作らない。

---

## アーキテクチャ

```
IS_HEADLESS=false (デフォルト)  →  GUI モード起動（従来通り）
IS_HEADLESS=true               →  --headless --ticker $E2E_TICKER --timeframe M1
```

### common_helpers.sh の拡張

| 追加要素 | 内容 |
| :--- | :--- |
| `IS_HEADLESS` 変数 | 環境変数から取得（デフォルト `false`） |
| `is_headless()` | `[ "$IS_HEADLESS" = "true" ]` を返す述語 |
| `start_app()` | `is_headless` に応じて `_start_gui_app` / `_start_headless_app` に分岐 |
| `_start_gui_app()` | 既存の GUI 起動ロジック |
| `_start_headless_app()` | `DEV_IS_DEMO=true $EXE --headless --ticker $E2E_TICKER ...` |

---

## テストスクリプトの両対応パターン

### GUI 専用ブロック

```bash
if ! is_headless; then
  # saved-state.json セットアップ
  # streams_ready 待機
  # Live ↔ Replay トグル検証
fi
```

### headless 専用ブロック

```bash
if is_headless; then
  # pane/list → 501 確認
  pend "TC-xxx" "headless は Live モードなし"
fi
```

### TC ごとの期待値分岐

```bash
if is_headless; then
  [ "$MODE" = "Replay" ] && pass "..." || fail "..."
else
  [ "$MODE" = "Live" ] && pass "..." || fail "..."
fi
```

---

## headless モードでの差分一覧（s1_basic_lifecycle.sh 例）

| TC | GUI | headless |
| :--- | :--- | :--- |
| TC-S1-01 | mode=Live | mode=Replay |
| TC-S1-02 | toggle → Replay | toggle は no-op、Replay のまま |
| TC-S1-03〜13 | 同一 | 同一 |
| TC-S1-14 | StepBackward -60000ms | PEND（未実装） |
| TC-S1-15 | Live 復帰リセット | PEND（Live なし） |
| TC-S1-H09 | なし | GET /api/pane/list → 501 |

---

## CI 統合

`.github/workflows/e2e.yml` に `IS_HEADLESS=true` で S1 を追加：

```yaml
- name: "S1 Headless lifecycle (HyperliquidLinear:BTC)"
  env:
    E2E_TICKER: HyperliquidLinear:BTC
    IS_HEADLESS: "true"
  run: bash tests/s1_basic_lifecycle.sh
```

他のテストスクリプトも同じパターンで両対応可能。

---

## 追加ヘルパー（common_helpers.sh）

| ヘルパー | 内容 |
| :--- | :--- |
| `_HEADLESS_START/END/TIMEFRAME` | `setup_single_pane` が格納する headless 用変数 |
| `setup_single_pane()` | headless では JSON 書込みをスキップし変数だけ保存 |
| `headless_play([start] [end])` | headless のみ replay/play を発行（GUI は no-op） |
| `ensure_replay_mode()` | GUI のみ toggle（headless は常に Replay） |
| `pend_if_headless(label, reason)` | headless で pend して return 0、GUI で return 1 |
| `order_symbol()` | E2E_TICKER のシンボル部分（例: "BTC"） |

---

## 実装ステータス

### common_helpers.sh
- ✅ `IS_HEADLESS` / `is_headless()` / `_start_headless_app()`
- ✅ `headless_play()` / `ensure_replay_mode()` / `pend_if_headless()` / `order_symbol()`
- ✅ `setup_single_pane()` headless 対応（`_HEADLESS_*` 変数格納、JSON スキップ）
- ✅ `_start_headless_app()` で `_HEADLESS_TIMEFRAME` 使用

### テストスクリプト（headless/GUI 両対応済み）
- ✅ `s1_basic_lifecycle.sh`
- ✅ `s3_autoplay.sh` — TC-S3-05 は GUI 専用
- ✅ `s9_speed_step.sh` — TC-S9-04 (StepBackward) は headless PEND
- ✅ `s10_range_end.sh` — TC-S10-03/04 (StepBackward) は headless PEND
- ✅ `s11_bar_step_discrete.sh` — TC-S11-05 (pane split) は headless PEND
- ✅ `s12_pre_start_history.sh` — TC-S12-01/02 (StepBackward) は headless PEND
- ✅ `s13_step_backward_quality.sh` — TC-S13-01/02/04 は headless PEND
- ✅ `s16_replay_resilience.sh` — TC-S16-02b/03/04/05 は headless PEND
- ✅ `s18_endurance.sh` — TC-S18-02-bwd/03 は headless PEND
- ✅ `s26_ticker_change_after_replay_end.sh` — TC-A/B/C は headless PEND（pane API）
- ✅ `s27_cyclespeed_reset.sh` — 全 TC headless 対応
- ✅ `s35_virtual_portfolio.sh` — TC-K/L (toggle) は headless PEND
- ✅ `s40_virtual_order_fill_cycle.sh` — DEV_USER_ID チェックを headless でスキップ
- ✅ `s41_limit_order_round_trip.sh` — 同上
- ✅ `s42_naked_short_cycle.sh` — 同上
- ✅ `s43_get_state_endpoint.sh` — TC-A PEND、current_time_ms/current_time 両対応
- ✅ `x2_buttons.sh` — TC-X2-02/03/08 は headless PEND
- ✅ `x4_virtual_order_live_guard.sh` — TC-01/02/03/06 は headless PEND

### CI 統合
- ✅ `.github/workflows/e2e.yml` — S1/S3/S27 headless ステップ追加

---

## CI 失敗分析（2026-04-17 実行）

### 概要

| テスト | 結果 | 根本原因 |
| :--- | :--- | :--- |
| GUI: S9 Speed & Step | ❌ | Tachibana UTC/JST タイムゾーンズレ |
| GUI: S27 CycleSpeed | ❌ | 同上 |
| GUI: S35 Virtual portfolio | ❌ | 同上 |
| GUI: S40 Virtual order fill cycle | ❌ | 同上 |
| GUI: S41 Limit order round trip | ❌ | 同上 |
| GUI: S42 Naked short cycle | ❌ | 同上 |
| Headless: S9 TC-S9-03 | ❌ | Playing 中 StepForward が Pause しない |
| Headless: S40 TC-D/E/H/I | ❌ | step-forward で on_tick にトレードが渡らない |
| Headless: S41 TC-D/E/K | ❌ | 同上 + pending orders 消失 |
| Headless: S42 TC-D-check | ❌ | 同上 |
| Headless: S43 TC-L | ❌ | Idle 時 GET /api/replay/state が 200 を返す |

---

## 修正計画

### Bug 1: GUI テスト全般 — Tachibana UTC/JST タイムゾーン問題

**根本原因**

CI runner は UTC タイムゾーンで動作している。Tachibana API の `p_sd_date` は
ローカル時刻（JST = UTC+9）を期待するが、`exchange/src/adapter/tachibana.rs:125` の
`current_p_sd_date()` が `chrono::Local::now()` を使うため UTC 時刻を送信してしまう。

```
p_sd_date:[2026.04.17-13:55:42] is exceed time limit:[2026.04.17-22:55:16]
                 ↑ UTC (runner)                              ↑ JST (server)
```

**CI runner は Windows Server 2025** のため `TZ: Asia/Tokyo` 環境変数は効かない
（Windows はレジストリでタイムゾーンを管理しており、Unix の `TZ` 変数は Rust の
`chrono::Local` に伝わらない）。

**修正箇所**: `exchange/src/adapter/tachibana.rs` `current_p_sd_date()` 関数

**修正方法**: `chrono::Local::now()` を JST 固定オフセット（UTC+9）に変える。
環境非依存で確実に動作する。

```rust
fn current_p_sd_date() -> String {
    let jst = chrono::FixedOffset::east_opt(9 * 3600).expect("valid offset");
    let now = chrono::Utc::now().with_timezone(&jst);
    now.format("%Y.%m.%d-%H:%M:%S%.3f").to_string()
}
```

---

### Bug 2: Headless S9 TC-S9-03 — Playing 中 StepForward が Paused にならない

**根本原因**

`src/headless.rs` の `step_forward()` 先頭で `is_paused()` チェックを行い、
Playing 状態では即座に `"not paused"` を返す。テストは Playing 中に
`StepForward` を呼んで Paused になることを期待しているが、headless では
この遷移が未実装。

```rust
fn step_forward(&mut self) -> String {
    if !self.state.is_paused() {
        return serde_json::json!({"ok": false, "error": "not paused"}).to_string();
    }
```

**修正箇所**: `src/headless.rs` `step_forward()` 関数

**修正方法**: Playing 中に `step_forward` が呼ばれた場合、既存の `pause()` メソッドを
呼んでから step を実行する。`pause()` は `&mut self` → `String` を返す高レベルメソッドなので、
戻り値を捨てて呼べる。

```rust
fn step_forward(&mut self) -> String {
    // Playing 中は先に自動 pause する（GUI 動作に合わせる）
    if self.state.is_playing() {
        let _ = self.pause();  // clock.pause() を内包する既存メソッド
    }
    if !self.state.is_paused() {
        return serde_json::json!({"ok": false, "error": "not paused"}).to_string();
    }
    // ... 既存のステップ処理
```

---

### Bug 3: Headless S40/S41/S42 — step-forward で on_tick にトレードが渡らない

**根本原因**

`src/headless.rs` `step_forward()` 内で `active_streams` を全件イテレートし
`store.trades_in(stream, ...)` を呼んでいるが、`active_streams` に含まれるのは
`StreamKind::Kline { ... }` バリアントであり、`store.trades` マップは
`StreamKind::Trades { ... }` をキーとしているため常に空 `&[]` が返る。
結果として `on_tick` に一度もトレードが渡らず、仮想注文が一切約定しない。

```rust
// step_forward 内（現状・バグあり）
for stream in active_streams.iter() {
    let trades = store.trades_in(stream, start..end);  // Kline stream → 常に &[]
    if !trades.is_empty() {
        self.virtual_engine.on_tick(ticker_str, trades, new_time);  // 呼ばれない
    }
}
```

`get_state_json` では同じ問題を既に正しく回避している（Kline stream から
対応する Trades stream を導出）。

```rust
// get_state_json 内（正しい参照パターン）
let trade_stream = StreamKind::Trades { ticker_info: *ticker_info };
let all_trades = store.trades_in(&trade_stream, trade_start..now_ms + 1);
```

**根本原因（追加）**

`store.trades_in()` の問題を修正しても、そもそも `handle_load_result` が
トレードデータを一切ストアに保存していない：

```rust
store.ingest_loaded(stream, range, LoadedData {
    klines,
    trades: vec![],  // 常に空
});
```

`StreamKind::Trades` キーで引いても空。Kline→Trades 変換だけでは不十分。

**修正箇所**: `src/headless.rs` `step_forward()` 関数内のトレード取得ループ

**修正方法**: GUI `ReplayController::synthetic_trades_at_current_time()`（controller.rs:939）
と同じパターンを適用する。kline の close 価格から合成 `Trade` を生成し `on_tick` に渡す。
`store.trades_in()` は使わない。

```rust
// step_forward 内：store.trades_in() ループを以下に置き換える
let ticker_str = self.ticker_str.clone();
if let ReplaySession::Active { store, active_streams, .. } = &self.state.session {
    let synthetic: Vec<Trade> = active_streams
        .iter()
        .filter(|s| matches!(s, StreamKind::Kline { .. }))
        .filter_map(|stream| {
            let klines = store.klines_in(stream, 0..new_time + 1);
            klines.iter().rev().find(|k| k.time <= new_time).map(|k| Trade {
                time: new_time,
                is_sell: false,
                price: k.close,
                qty: exchange::unit::qty::Qty::from_f32(1.0),
            })
        })
        .collect();
    if !synthetic.is_empty() {
        self.virtual_engine.on_tick(&ticker_str, &synthetic, new_time);
    }
}
```

**`tick()` は変更しない**。`tick()` は `dispatch_tick()` を経由する GUI 共通パスで
あり、S40/S41/S42 の失敗は全て step-forward 経由と確認済み。`tick()` を触ると
GUI 共通パスを壊すリスクがある。

---

### Bug 4: Headless S43 TC-L — Idle 時 GET /api/replay/state が 200 を返す

**根本原因**

`src/headless.rs` の `GetState` ハンドラが常に `reply.send()` (HTTP 200) を使用。
`get_state_json()` は Idle/Loading 時に `{"error":"replay not active"}` という
文字列を返すが、それを HTTP 200 で送信してしまう。

```rust
// 現状（バグあり）
ApiCommand::VirtualExchange(VirtualExchangeCommand::GetState) => {
    reply.send(self.get_state_json(200));  // 常に HTTP 200
}

// get_state_json の Idle 分岐
_ => r#"{"error":"replay not active"}"#.to_string(),  // 400 で返すべき
```

**修正箇所**: `src/headless.rs` の `GetState` コマンドハンドラ（line 592 付近）

**修正方法**: session が Active でない場合は `reply.send_status(400, ...)` を使う。

```rust
ApiCommand::VirtualExchange(VirtualExchangeCommand::GetState) => {
    match &self.state.session {
        ReplaySession::Active { .. } => reply.send(self.get_state_json(200)),
        _ => reply.send_status(
            400,
            r#"{"error":"replay not active"}"#.to_string(),
        ),
    }
}
```

---

## 修正実装ステータス

- ✅ Bug 1: `exchange/src/adapter/tachibana.rs` — `current_p_sd_date()` を JST 固定オフセットに変更
- ✅ Bug 2: `src/headless.rs` — Playing 中 StepForward 自動 pause 実装
- ✅ Bug 3: `src/headless.rs` — step_forward の合成トレード生成で on_tick を正しく呼ぶ
- ✅ Bug 4: `src/headless.rs` — GetState Idle 時 400 返却修正
- ✅ Bug 5: `src/headless.rs` — `get_orders_json()` を `{"orders":[...]}` 形式に修正（TC-K 対応）
