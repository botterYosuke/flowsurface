# E2E テスト: bash → Python 移行計画

**作成**: 2026-04-19  
**ブランチ**: sasa/python

---

## 目標

`tests/*.sh` E2E テストスクリプトをすべて `tests/*.py`（flowsurface-sdk 版）に移行する。
`tests/s1_basic_lifecycle.py` を実装テンプレートとして使用する。

---

## 共通インフラ

| ファイル | 役割 | 状態 |
|:---|:---|:---|
| `tests/helpers.py` | Python 版共通ヘルパー（bash `common_helpers.sh` 相当） | ✅ 作成済み |
| `python/env.py` | `FlowsurfaceEnv` SDK | 既存 |
| `tests/s1_basic_lifecycle.py` | 移行テンプレート | ✅ 完了 |

---

## 移行対象外（bash のまま）

| ファイル | 理由 |
|:---|:---|
| `tests/common_helpers.sh` | Python 版に取り込み済み |
| `tests/e2e_replay_api.sh` | CI 統合スクリプト |
| `tests/run_all_binance.sh` | 一括実行スクリプト（.py 対応済み） |
| `tests/diag_s20.sh` | デバッグ診断用 |

---

## 移行スクリプト一覧

### Priority 1: Headless 対応・シンプル

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s3_autoplay.sh` | headless | ✅ 完了 |
| `s9_speed_step.sh` | GUI + headless | ✅ 完了 |
| `s10_range_end.sh` | headless | ✅ 完了 |
| `s27_cyclespeed_reset.sh` | GUI + headless | ✅ 完了 |
| `s43_get_state_endpoint.sh` | headless | ✅ 完了 |
| `x1_current_time.sh` | headless | ✅ 完了 |
| `x2_buttons.sh` | headless | ✅ 完了 |
| `x4_virtual_order_live_guard.sh` | headless | ✅ 完了 |

### Priority 2: Headless 対応・中程度

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s11_bar_step_discrete.sh` | headless | ✅ 完了 |
| `s12_pre_start_history.sh` | headless | ✅ 完了 |
| `s13_step_backward_quality.sh` | headless | ✅ 完了 |
| `s16_replay_resilience.sh` | headless | ✅ 完了 |
| `s17_error_boundary.sh` | headless | ✅ 完了 |
| `s18_endurance.sh` | headless | ✅ 完了 |
| `s26_ticker_change_after_replay_end.sh` | headless | ✅ 完了 |
| `s28_ticker_change_while_loading.sh` | headless | ✅ 完了 |
| `s35_virtual_portfolio.sh` | GUI + headless | ✅ 完了 |

### Priority 3: 複数ペイン・複雑

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s7_mid_replay_pane.sh` | headless | ✅ 完了 |
| `s8_error_boundary.sh` | headless | ✅ 完了 |
| `s23_mid_replay_ticker_change.sh` | headless | ✅ 完了 |
| `s40_virtual_order_fill_cycle.sh` | headless + tachibana | ✅ 完了 |
| `s41_limit_order_round_trip.sh` | headless + tachibana | ✅ 完了 |
| `s42_naked_short_cycle.sh` | headless + tachibana | ✅ 完了 |

### Priority 4: GUI テスト

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s2_persistence.py` | GUI | ✅ 完了 |
| `s6_mixed_timeframes.py` | GUI | ✅ 完了 |
| `s30_mixed_sample_loading.py` | GUI | ✅ 完了 |
| `s31_replay_end_restart.py` | GUI | ✅ 完了 |
| `s34_virtual_order_basic.py` | GUI | ✅ 完了 |
| `s1b_limit_buy.py` | GUI (real auth) | ✅ 完了 |
| `s1c_market_sell.py` | GUI (real auth) | ✅ 完了 |
| `s1d_limit_sell.py` | GUI (real auth) | ✅ 完了 |

### Priority 5: GUI 専用（IS_HEADLESS=false 時のみ）

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s24_sidebar_select_ticker.py` | GUI | ✅ 完了 |
| `s33_sidebar_split_pane.py` | GUI | ✅ 完了 |
| `s36_sidebar_order_pane.py` | GUI | ✅ 完了 |
| `s37_order_panels_integrated.py` | GUI | ✅ 完了 |
| `s39_buying_power_portfolio.py` | GUI | ✅ 完了 |

### Priority 6: Tachibana 系（証券 API 必須）

> ローカル実行不可。CI のみで検証。DEV_USER_ID/DEV_PASSWORD が必要。

| スクリプト | CI ジョブ | 状態 |
|:---|:---|:---|
| `s5_tachibana_mixed.py` | GUI | ✅ 完了 |
| `s14_autoplay_event_driven.py` | tachibana | ✅ 完了 |
| `s19_tachibana_chart_snapshot.py` | (CI 外) | ✅ 完了 |
| `s20_tachibana_replay_resilience.py` | tachibana | ✅ 完了 |
| `s21_tachibana_error_boundary.py` | tachibana | ✅ 完了 |
| `s22_tachibana_endurance.py` | tachibana | ✅ 完了 |
| `s29_tachibana_holiday_skip.py` | tachibana | ✅ 完了 |
| `s32_toyota_candlestick_add.py` | tachibana | ✅ 完了 |
| `s44_order_list.py` | tachibana | ✅ 完了 |
| `s45_order_correct_cancel.py` | tachibana | ✅ 完了 |
| `s46_wrong_password.py` | GUI | ✅ 完了 |
| `s47_outside_hours.py` | GUI | ✅ 完了 |
| `s48_invalid_issue.py` | GUI | ✅ 完了 |
| `s49_account_info.py` | tachibana | ✅ 完了 |

### CI 外スクリプト（完了後に追加移行）

| スクリプト | 備考 | 状態 |
|:---|:---|:---|
| `s4_multi_pane_binance.sh` | CI 未登録 | 後回し |
| `s15_chart_snapshot.sh` | chart-snapshot API 未実装 | 後回し |
| `s25_screenshot_and_auth.sh` | CI 未登録 | 後回し |
| `sX_toyota_buy_demo.sh` | デモ用 | 後回し |
| `x3_chart_update.sh` | CI 未登録 | 後回し |

---

## 実装ルール

1. **IS_HEADLESS 環境変数** で GUI / headless 分岐（`os.environ.get("IS_HEADLESS", "").lower() == "true"`）
2. **FlowsurfaceEnv._start_process()** でプロセス起動、`env.close()` で終了（`finally`）
3. **backup_state() / restore_state()** を `main()` で呼ぶ
4. **pytest 対応**: `test_<name>()` 関数を実装
5. **実行方法**: `uv run tests/<script>.py`（`FLOWSURFACE_BINARY` 環境変数でバイナリ指定）
6. **移行完了後**: 元 `.sh` を `tests/archive/` に移動
7. **CI 更新**: `e2e.yml` の `script:` を `.sh` → `.py` に変更

---

## CI 更新状況（e2e.yml）

すべてのジョブで `.sh` → `.py` 更新完了 ✅。`test-gui-tachibana-session` に `setup-uv` ステップ追加・`uv run` 対応済み。

---

## 知見・設計上の決定

- `helpers.py` を `tests/` に配置し、各スクリプトが `from helpers import *` で使用する
- `setup_single_pane()` は headless 時に `_HEADLESS_*` 変数を設定し、GUI 時に saved-state.json を書き込む
- `headless_play()` は headless 時のみ `/api/replay/play` を POST する（GUI は saved-state 自動再生）
- Tachibana テストは `DEV_USER_ID` / `DEV_PASSWORD` が未設定の場合、全 TC を PEND して exit 0
- `speed_to_10x()` は 3 回 CycleSpeed を呼んで 1x→2x→5x→10x
- `secondary_ticker()` / `tertiary_ticker()` は `E2E_TICKER` の取引所から自動決定
