#!/usr/bin/env bash
# run_all_binance.sh — Binance-only E2E スクリプトを順番に実行し PASS/FAIL を集計する
# Usage: bash tests/run_all_binance.sh [--skip-endurance]
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TESTS_DIR="$REPO_ROOT/tests"
EXE="${FLOWSURFACE_EXE:-$REPO_ROOT/target/debug/flowsurface.exe}"
SKIP_ENDURANCE="${1:-}"

export FLOWSURFACE_EXE="$EXE"

# 実行対象スクリプト（順番通り、Tachibana 除外）
SCRIPTS=(
  s1_basic_lifecycle.py
  s2_persistence.sh
  s3_autoplay.sh
  s4_multi_pane_binance.sh
  s6_mixed_timeframes.sh
  s7_mid_replay_pane.sh
  s8_error_boundary.sh
  s9_speed_step.sh
  s10_range_end.sh
  s11_bar_step_discrete.sh
  s12_pre_start_history.sh
  s13_step_backward_quality.sh
  s15_chart_snapshot.sh
  s16_replay_resilience.sh
  s17_error_boundary.sh
  s23_mid_replay_ticker_change.sh
  s24_sidebar_select_ticker.sh
  s27_cyclespeed_reset.sh
  s28_ticker_change_while_loading.sh
  s33_sidebar_split_pane.sh
  s34_virtual_order_basic.sh
  s35_virtual_portfolio.sh
  s36_sidebar_order_pane.sh
  s37_order_panels_integrated.sh
  s39_buying_power_portfolio.sh
  s40_virtual_order_fill_cycle.sh
  s41_limit_order_round_trip.sh
  s42_naked_short_cycle.sh
  s43_get_state_endpoint.sh
)

if [ "$SKIP_ENDURANCE" != "--skip-endurance" ]; then
  SCRIPTS+=(s18_endurance.sh)
fi

TOTAL=0
PASSED=0
FAILED=0
declare -a FAILED_LIST=()

LOG_DIR="${RUNNER_TEMP:-/tmp}"
MASTER_LOG="$LOG_DIR/run_all_binance.log"

echo "======================================================"
echo "  flowsurface E2E — Binance scripts"
echo "  EXE: $EXE"
echo "  Scripts: ${#SCRIPTS[@]}"
echo "  Log: $MASTER_LOG"
echo "======================================================"
echo ""

for script in "${SCRIPTS[@]}"; do
  path="$TESTS_DIR/$script"
  if [ ! -f "$path" ]; then
    echo "  SKIP (not found): $script"
    continue
  fi

  TOTAL=$((TOTAL + 1))
  echo "------------------------------------------------------"
  echo "  [$TOTAL/${#SCRIPTS[@]}] $script"
  echo "------------------------------------------------------"

  # アプリが残っていたらクリーンアップ
  taskkill //f //im flowsurface.exe > /dev/null 2>&1 || true
  sleep 1

  set +e
  if [[ "$script" == *.py ]]; then
    FLOWSURFACE_BINARY="$(cygpath -w "$EXE")" uv run "$path" 2>&1 | tee -a "$MASTER_LOG"
  else
    bash "$path" 2>&1 | tee -a "$MASTER_LOG"
  fi
  exit_code=${PIPESTATUS[0]}
  set -e

  if [ $exit_code -eq 0 ]; then
    echo "  >>> SCRIPT PASSED: $script <<<"
    PASSED=$((PASSED + 1))
  else
    echo "  >>> SCRIPT FAILED (exit=$exit_code): $script <<<"
    FAILED=$((FAILED + 1))
    FAILED_LIST+=("$script")
  fi
  echo ""
done

echo "======================================================"
echo "  RESULTS: $PASSED/$TOTAL passed, $FAILED failed"
if [ ${#FAILED_LIST[@]} -gt 0 ]; then
  echo "  FAILED scripts:"
  for s in "${FAILED_LIST[@]}"; do
    echo "    - $s"
  done
fi
echo "======================================================"

[ $FAILED -eq 0 ]
