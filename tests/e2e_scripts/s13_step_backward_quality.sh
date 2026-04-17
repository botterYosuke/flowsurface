#!/usr/bin/env bash
# s13_step_backward_quality.sh — スイート S13: StepBackward 品質保証
#
# 検証シナリオ:
#   TC-S13-01: StepBackward 後 2s 以内に Loading 解消（チラつき防止）
#   TC-S13-02-1〜10: 10 回 StepBackward、各ステップ後 streams_ready=true
#   TC-S13-03: resume 後 delta が 60000ms 倍数（live data 非混入）
#   TC-S13-04-1〜5: StepForward ↔ StepBackward 交互 × 5 → status=Paused 維持
#
# 仕様根拠:
#   docs/replay_header.md §6.3 — StepBackward 品質保証（チラつき防止・live data 非混入）
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S13: StepBackward 品質保証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app

if ! wait_playing 30; then
  fail "TC-S13-precond" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S13-precond" "Paused に遷移せず"
  exit 1
fi

PANES=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
echo "  PANE_ID=$PANE_ID"

# 少し前進させてから StepBackward のテストを行う（start_time 境界を避ける）
for _ in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
done
wait_status Paused 10 || true

# TC-S13-01: StepBackward 後 2 秒以内に Loading が解消される
curl -s -X POST "$API/replay/step-backward" > /dev/null
T_START=$SECONDS
RESOLVED=false
FINAL_STATUS="unknown"
while [ $((SECONDS - T_START)) -le 2 ]; do
  FINAL_STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  if [ "$FINAL_STATUS" = "Paused" ] || [ "$FINAL_STATUS" = "Playing" ]; then
    RESOLVED=true
    break
  fi
  sleep 0.2
done
if $RESOLVED; then
  pass "TC-S13-01: StepBackward 後 $((SECONDS - T_START))s 以内に status=$FINAL_STATUS（Loading 解消）"
else
  FINAL_STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  fail "TC-S13-01" "2 秒経過後も status=$FINAL_STATUS（Loading 継続の疑い）"
fi

wait_status Paused 10 || true

# TC-S13-02: 10 回 StepBackward — 各ステップ後に streams_ready=true を個別確認
for i in $(seq 1 10); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  wait_status Paused 10 || true
  sleep 0.3
  PANES=$(curl -s "$API/pane/list")
  READY=$(node -e "
    const ps = (JSON.parse(process.argv[1]).panes || []);
    const p = ps.find(x => x.id === '$PANE_ID');
    console.log(p && p.streams_ready ? 'true' : 'false');
  " "$PANES")
  [ "$READY" = "true" ] \
    && pass "TC-S13-02-$i: StepBackward #$i 後 streams_ready=true" \
    || fail "TC-S13-02-$i" "streams_ready=$READY（チラつき発生の疑い）"
done

# TC-S13-03: resume 後の delta がバー境界に揃う（live data 非混入確認）
curl -s -X POST "$API/replay/resume" > /dev/null
if ! wait_status Playing 10; then
  fail "TC-S13-03-pre" "Playing に遷移せず"
else
  speed_to_10x
  T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if T2=$(wait_for_time_advance "$T1" 15); then
    DELTA=$(node -e "console.log(String(BigInt('$T2') - BigInt('$T1')))")
    MOD=$(node -e "console.log(String(BigInt('$DELTA') % BigInt('60000')))")
    [ "$MOD" = "0" ] \
      && pass "TC-S13-03: resume 後 delta=$DELTA ms（60000ms 倍数、live data 非混入）" \
      || fail "TC-S13-03" "delta=$DELTA, mod=$MOD（live data 混入の疑い）"
  else
    fail "TC-S13-03" "15 秒待機しても current_time が変化しなかった"
  fi
fi

# TC-S13-04: StepForward ↔ StepBackward 交互 × 5 でも status=Paused 維持
curl -s -X POST "$API/replay/pause" > /dev/null
wait_status Paused 10 || true
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  wait_status Paused 10 || true
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  wait_status Paused 10 || true
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S13-04-$i: 交互 Step #$i 後 status=Paused" \
    || fail "TC-S13-04-$i" "status=$STATUS"
done

print_summary
[ $FAIL -eq 0 ]
