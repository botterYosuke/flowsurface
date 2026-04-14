#!/usr/bin/env bash
# s17_error_boundary.sh — スイート S17: クラッシュ・エラー境界テスト
# アプリがクラッシュせず、エラーが適切に処理されることを確認する
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S17: クラッシュ・エラー境界テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

FAKE_UUID="ffffffff-ffff-ffff-ffff-ffffffffffff"

# ── TC-S17-01〜03: 不正 pane_id に対する各エンドポイント ────────────────
# pane/split, pane/close, pane/set-ticker に存在しない UUID → HTTP 200 + error でクラッシュなし
echo "  [TC-S17-01/03] 不正 pane_id テスト..."

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app

if ! wait_playing 30; then
  fail "TC-S17-precond" "Playing 到達せず"
  exit 1
fi

# TC-S17-01: pane/split に不正 UUID
HTTP_SPLIT=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"axis\":\"Vertical\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_SPLIT" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-01: pane/split 不正 UUID → HTTP=$HTTP_SPLIT & アプリ生存" \
  || fail "TC-S17-01" "HTTP=$HTTP_SPLIT alive=$ALIVE"

# TC-S17-02: pane/close に不正 UUID
HTTP_CLOSE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/close" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_CLOSE" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-02: pane/close 不正 UUID → HTTP=$HTTP_CLOSE & アプリ生存" \
  || fail "TC-S17-02" "HTTP=$HTTP_CLOSE alive=$ALIVE"

# TC-S17-03: pane/set-ticker に不正 UUID
HTTP_TICKER=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"ticker\":\"BinanceLinear:ETHUSDT\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_TICKER" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-03: pane/set-ticker 不正 UUID → HTTP=$HTTP_TICKER & アプリ生存" \
  || fail "TC-S17-03" "HTTP=$HTTP_TICKER alive=$ALIVE"

stop_app

# ── TC-S17-04: 空 range (start == end) ─────────────────────────────────
echo "  [TC-S17-04] 空 range (start == end)..."
SAME_TIME=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$SAME_TIME" "$SAME_TIME"
start_app

# アプリが起動して API が応答すれば OK（crash なし）
sleep 5
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S17-04: 空 range でもアプリ生存 (status=$STATUS)" \
  || fail "TC-S17-04" "空 range でアプリがクラッシュした"

stop_app

# ── TC-S17-05: 未来の range (現在時刻 + 24h 先) ─────────────────────────
echo "  [TC-S17-05] 未来 range テスト..."
FUTURE_START=$(utc_offset 24)
FUTURE_END=$(utc_offset 26)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$FUTURE_START" "$FUTURE_END"
start_app

# EventStore が空でも Playing/Paused で停止するだけ（クラッシュしない）
sleep 10
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S17-05: 未来 range でもアプリ生存 (status=$STATUS)" \
  || fail "TC-S17-05" "未来 range でアプリがクラッシュした"

stop_app

# ── TC-S17-06: StepForward 連打 50 回 (Paused 状態) ─────────────────────
echo "  [TC-S17-06] StepForward 連打 50 回..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S17-06-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S17-06-pre" "Paused に遷移せず"
  exit 1
fi

CRASH=false
for i in $(seq 1 50); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  # 軽いポーリング（厳密な wait_status より速い）
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    break
  fi
done
# 最終確認
wait_status Paused 15 || true
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")

if ! $CRASH && [ "$ALIVE" = "true" ] && [ "$STATUS" = "Paused" ]; then
  pass "TC-S17-06: StepForward 50 連打 → crash なし, status=Paused"
else
  fail "TC-S17-06" "crash=$CRASH alive=$ALIVE status=$STATUS"
fi

stop_app

# ── TC-S17-07: split 上限テスト ──────────────────────────────────────────
# ペインを分割し続けて最大ペイン数を超えた後 split → HTTP 200 or エラー応答、クラッシュなし
echo "  [TC-S17-07] split 上限テスト..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S17-07-pre" "Playing 到達せず"
  exit 1
fi

# 最大 10 回 split を試みる（実際の上限に達するまで）
SPLIT_COUNT=0
LAST_HTTP=""
for i in $(seq 1 10); do
  PANES=$(curl -s "$API/pane/list")
  FIRST_PANE=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
  if [ -z "$FIRST_PANE" ]; then
    break
  fi
  LAST_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$FIRST_PANE\",\"axis\":\"Vertical\"}")
  SPLIT_COUNT=$((SPLIT_COUNT + 1))
  PANE_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  sleep 0.5
  # ペイン数が増えなくなったら上限到達
  if [ "$LAST_HTTP" != "200" ]; then
    break
  fi
done

# 上限到達後もアプリが生存しているか
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
PANE_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
  "$(curl -s "$API/pane/list")")
[ "$ALIVE" = "true" ] \
  && pass "TC-S17-07: split $SPLIT_COUNT 回後 (HTTP=$LAST_HTTP) クラッシュなし (panes=$PANE_COUNT)" \
  || fail "TC-S17-07" "split 繰り返し後にアプリがクラッシュした"

print_summary
[ $FAIL -eq 0 ]
