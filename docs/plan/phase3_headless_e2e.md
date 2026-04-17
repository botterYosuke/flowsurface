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
