#!/usr/bin/env bash
# s12_pre_start_history.sh — スイート S12: Start 以前の履歴バー表示
#
# 検証シナリオ:
#   TC-S12-01: StepBackward 後 current_time >= start_time（下限クランプ）
#   TC-S12-02-1〜5: StepBackward 連打 5 回でも start_time クランプ維持
#   TC-S12-03: resume 後 current_time 正常前進（10x でポーリング）
#   TC-S12-04: PEND — chart-snapshot API 実装後に bar_count 直接検証
#
# 仕様根拠:
#   docs/replay_header.md §6.3 — PRE_START_HISTORY_BARS=300・start_time 下限クランプ
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S12: Start 以前の履歴バー表示 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app

if ! wait_playing 30; then
  fail "TC-S12-precond" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S12-precond" "Paused に遷移せず"
  exit 1
fi

START_MS=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
echo "  start_time=$START_MS"

# TC-S12-01: 1 回 StepBackward → current_time >= start_time
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
wait_status Paused 10 || true
CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if [ "$(bigt_ge "$CT" "$START_MS")" = "true" ]; then
  pass "TC-S12-01: StepBackward 後 current_time($CT) >= start_time($START_MS)"
else
  fail "TC-S12-01" "current_time=$CT < start_time=$START_MS"
fi

# TC-S12-02: StepBackward 連打（5 回）でも start_time クランプ
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.5
  wait_status Paused 10 || true
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if [ "$(bigt_ge "$CT" "$START_MS")" = "true" ]; then
    pass "TC-S12-02-$i: StepBackward #$i current_time($CT) >= start_time($START_MS)"
  else
    fail "TC-S12-02-$i" "current_time=$CT < start_time=$START_MS"
  fi
done

# TC-S12-03: resume 後に current_time が正常前進（10x でポーリング）
curl -s -X POST "$API/replay/resume" > /dev/null
if ! wait_status Playing 10; then
  fail "TC-S12-03-pre" "Playing に遷移せず"
else
  speed_to_10x
  CT_BASE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if CT_AFTER=$(wait_for_time_advance "$CT_BASE" 15); then
    pass "TC-S12-03: resume 後 current_time 前進 ($CT_BASE → $CT_AFTER)"
  else
    fail "TC-S12-03" "15 秒待機しても current_time が前進しなかった"
  fi
fi

# TC-S12-04: バー本数直接検証（chart-snapshot API 未実装のため PEND）
pend "TC-S12-04" "GET /api/pane/chart-snapshot 未実装 → 実装後に追加"

print_summary
[ $FAIL -eq 0 ]
