#!/usr/bin/env bash
# s18_endurance.sh — スイート S18: 耐久テスト
# 長時間再生・高速操作でのメモリリークやデッドロックがないことを確認する
# 警告: このスクリプトは完走に 15〜30 分かかる
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S18: 耐久テスト ==="
echo "  警告: このスクリプトは完走に 15〜30 分かかる"
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── TC-S18-01: 2 時間 range を 10x 速度で再生し終了 → Paused ─────────────
# 2 時間 / 10x = 12 分 の実時間。wait_status Paused 900 (15 分) でカバー
echo "  [TC-S18-01] 10x 速度 2h range 完走テスト..."
START_LONG=$(utc_offset -4)
END_LONG=$(utc_offset -2)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START_LONG" "$END_LONG"
start_app

if ! wait_playing 30; then
  fail "TC-S18-01-pre" "Playing 到達せず"
  exit 1
fi

speed_to_10x
echo "  10x 速度で再生中（最大 900 秒待機）..."
if wait_status Paused 900; then
  pass "TC-S18-01: 2h range 10x 完走 → Paused 到達"
else
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  fail "TC-S18-01" "900 秒経過後も status=$STATUS（Paused 未到達）"
fi

stop_app

# ── TC-S18-02: Step 1000 回（各方向 500 回）→ crash なし ──────────────────
# Pause 後に step-forward × 500 → step-backward × 500
# 1 step あたり 0.3 秒 → 約 300 秒（5 分）
echo "  [TC-S18-02] Step 1000 回耐久テスト..."
# 広い range を使って StepForward 500 回分の空間を確保
START_WIDE=$(utc_offset -12)
END_WIDE=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START_WIDE" "$END_WIDE"
start_app

if ! wait_playing 30; then
  fail "TC-S18-02-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S18-02-pre" "Paused に遷移せず"
  exit 1
fi

echo "  StepForward × 500..."
CRASH=false
for i in $(seq 1 500); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    echo "  CRASH detected at forward step #$i"
    break
  fi
  # 進捗表示（100 回ごと）
  if [ $((i % 100)) -eq 0 ]; then
    echo "    forward step $i/500..."
  fi
done

if $CRASH; then
  fail "TC-S18-02-fwd" "StepForward 連打中にアプリがクラッシュした"
else
  wait_status Paused 15 || true
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S18-02-fwd: StepForward 500 回完了 → status=Paused" \
    || fail "TC-S18-02-fwd" "status=$STATUS (Paused 期待)"
fi

echo "  StepBackward × 500..."
CRASH=false
for i in $(seq 1 500); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    echo "  CRASH detected at backward step #$i"
    break
  fi
  if [ $((i % 100)) -eq 0 ]; then
    echo "    backward step $i/500..."
  fi
done

if $CRASH; then
  fail "TC-S18-02-bwd" "StepBackward 連打中にアプリがクラッシュした"
else
  wait_status Paused 15 || true
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S18-02-bwd: StepBackward 500 回完了 → status=Paused" \
    || fail "TC-S18-02-bwd" "status=$STATUS (Paused 期待)"
fi

stop_app

# ── TC-S18-03: ペイン CRUD サイクル 20 回（Playing 中）→ Playing 維持 ───
echo "  [TC-S18-03] Playing 中 split→close × 20 サイクル..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S18-03-pre" "Playing 到達せず"
  exit 1
fi

CRUD_FAIL=false
for i in $(seq 1 20); do
  # 初期ペイン ID 取得
  PANES=$(curl -s "$API/pane/list")
  PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
  if [ -z "$PANE0" ]; then
    fail "TC-S18-03-$i" "ペイン ID 取得失敗"
    CRUD_FAIL=true
    break
  fi

  # split
  curl -s -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null
  if ! wait_for_pane_count 2 10; then
    fail "TC-S18-03-$i" "split 後ペイン数が 2 にならなかった"
    CRUD_FAIL=true
    break
  fi

  # 新ペイン ID 取得
  PANES=$(curl -s "$API/pane/list")
  NEW_PANE=$(node -e "
    const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.id!=='$PANE0');
    console.log(p?p.id:'');
  " "$PANES")
  if [ -z "$NEW_PANE" ]; then
    fail "TC-S18-03-$i" "新ペイン ID 取得失敗"
    CRUD_FAIL=true
    break
  fi

  # close
  curl -s -X POST "$API/pane/close" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$NEW_PANE\"}" > /dev/null
  if ! wait_for_pane_count 1 10; then
    fail "TC-S18-03-$i" "close 後ペイン数が 1 にならなかった"
    CRUD_FAIL=true
    break
  fi

  # Playing 維持確認（5 サイクルごと）
  if [ $((i % 5)) -eq 0 ]; then
    STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    if [ "$STATUS" != "Playing" ]; then
      fail "TC-S18-03-$i" "CRUD サイクル $i 回後 status=$STATUS (Playing 期待)"
      CRUD_FAIL=true
      break
    fi
    echo "    cycle $i/20: status=Playing OK"
  fi
done

if ! $CRUD_FAIL; then
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Playing" ] \
    && pass "TC-S18-03: CRUD 20 サイクル完了 → status=Playing 維持" \
    || fail "TC-S18-03" "20 サイクル後 status=$STATUS (Playing 期待)"
fi

print_summary
[ $FAIL -eq 0 ]
