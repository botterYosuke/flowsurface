#!/bin/bash
# E2E テスト: リプレイ API エンドポイント検証
# 使い方: bash tests/e2e/e2e_replay_api.sh
#
# 前提: flowsurface が FLOWSURFACE_API_PORT で起動済み
# テスト用データディレクトリを使用し、本番データを汚さない

set -e

PORT=${FLOWSURFACE_API_PORT:-9877}
BASE="http://127.0.0.1:${PORT}"
PASS=0
FAIL=0
ERRORS=""

# JSON から指定キーの値を取得（jq/python 不要）
json_value() {
    echo "$1" | grep -o "\"$2\":[^,}]*" | head -1 | sed "s/\"$2\"://" | tr -d '"'
}

# 30日幅: D1チャートでも step-forward/backward が動作するよう十分な幅を確保
START_TIME="2026-03-01 00:00"
END_TIME="2026-03-31 00:00"

echo "=== E2E Replay API Test ==="
echo "Port: $PORT"
echo "Start: $START_TIME"
echo "End:   $END_TIME"
echo ""

assert_contains() {
    local test_name="$1"
    local response="$2"
    local expected="$3"
    if echo "$response" | grep -q "$expected"; then
        echo "  PASS: $test_name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $test_name (expected '$expected' in response: $response)"
        FAIL=$((FAIL + 1))
        ERRORS="$ERRORS\n  - $test_name"
    fi
}

assert_http_status() {
    local test_name="$1"
    local status="$2"
    local expected="$3"
    if [ "$status" = "$expected" ]; then
        echo "  PASS: $test_name (HTTP $status)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $test_name (expected HTTP $expected, got $status)"
        FAIL=$((FAIL + 1))
        ERRORS="$ERRORS\n  - $test_name"
    fi
}

# ── 起動待ち ──
echo "Waiting for app to start..."
for i in $(seq 1 30); do
    if curl -s "${BASE}/api/replay/status" > /dev/null 2>&1; then
        echo "App is ready."
        break
    fi
    if [ "$i" = "30" ]; then
        echo "FATAL: App did not start within 30s. Ensure flowsurface is running with FLOWSURFACE_API_PORT=$PORT"
        exit 1
    fi
    sleep 1
done

# 保存ステートで Replay モードで復元された場合に備えて Live に正規化する
curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"live"}' > /dev/null
echo ""

# ── Test 1: GET /api/replay/status (初期状態) ──
echo "[Test 1] GET /api/replay/status (initial)"
RESP=$(curl -s "${BASE}/api/replay/status")
assert_contains "mode is Live" "$RESP" '"mode":"Live"'

# ── Test 2: POST /api/app/set-mode (Live→Replay) ──
echo "[Test 2] POST /api/app/set-mode {mode:replay} (Live→Replay)"
RESP=$(curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"replay"}')
assert_contains "mode switched to Replay" "$RESP" '"mode":"Replay"'

# ── Test 3: GET /api/replay/status (Replay確認) ──
echo "[Test 3] GET /api/replay/status (after set-mode)"
RESP=$(curl -s "${BASE}/api/replay/status")
assert_contains "mode is Replay" "$RESP" '"mode":"Replay"'

# ── Test 4: POST /api/replay/play (再生開始) ──
echo "[Test 4] POST /api/replay/play"
RESP=$(curl -s -X POST "${BASE}/api/replay/play" -d "{\"start\":\"${START_TIME}\",\"end\":\"${END_TIME}\"}")
assert_contains "play returns Replay mode" "$RESP" '"mode":"Replay"'
echo "  INFO: status=$(json_value "$RESP" status)"

# ── Test 5: speed cycle ──
echo "[Test 5] POST /api/replay/speed (4x cycle)"
RESP=$(curl -s -X POST "${BASE}/api/replay/speed")
assert_contains "speed is 2x" "$RESP" '"speed":"2x"'
RESP=$(curl -s -X POST "${BASE}/api/replay/speed")
assert_contains "speed is 5x" "$RESP" '"speed":"5x"'
RESP=$(curl -s -X POST "${BASE}/api/replay/speed")
assert_contains "speed is 10x" "$RESP" '"speed":"10x"'
RESP=$(curl -s -X POST "${BASE}/api/replay/speed")
assert_contains "speed wraps to 1x" "$RESP" '"speed":"1x"'

# ── Test 6: pause ──
echo "[Test 6] POST /api/replay/pause"
# Loading→Playing 遷移を待つ（最大 30 秒）
for i in $(seq 1 30); do
    ST=$(curl -s "${BASE}/api/replay/status" | grep -o '"status":"[^"]*"' | head -1 | sed 's/"status":"//;s/"//')
    [ "$ST" = "Playing" ] && break
    sleep 1
done
RESP=$(curl -s -X POST "${BASE}/api/replay/pause")
assert_contains "pause returns status" "$RESP" '"mode":"Replay"'

# ── Test 7: step-backward (end_time を脱出してから current_time 減少確認) ──
# pause 直後は clock が end_time に達している場合があるため、step-backward で start 方向へ
echo "[Test 7] POST /api/replay/step-backward"
BEFORE_TIME=$(json_value "$(curl -s "${BASE}/api/replay/status")" current_time)
RESP=$(curl -s -X POST "${BASE}/api/replay/step-backward")
sleep 1  # step-backward does chart rebuild
AFTER_TIME=$(json_value "$(curl -s "${BASE}/api/replay/status")" current_time)
if [ -n "$AFTER_TIME" ] && [ -n "$BEFORE_TIME" ] && [ "$AFTER_TIME" -lt "$BEFORE_TIME" ] 2>/dev/null; then
    echo "  PASS: current_time decreased ($BEFORE_TIME → $AFTER_TIME)"
    PASS=$((PASS + 1))
else
    echo "  FAIL: current_time did not decrease ($BEFORE_TIME → $AFTER_TIME)"
    FAIL=$((FAIL + 1))
    ERRORS="$ERRORS\n  - step-backward current_time decrease"
fi

# ── Test 8: step-forward (current_time 増加) ──
echo "[Test 8] POST /api/replay/step-forward"
BEFORE_TIME=$AFTER_TIME
RESP=$(curl -s -X POST "${BASE}/api/replay/step-forward")
AFTER_TIME=$(json_value "$RESP" current_time)
if [ -n "$AFTER_TIME" ] && [ -n "$BEFORE_TIME" ] && [ "$AFTER_TIME" -gt "$BEFORE_TIME" ] 2>/dev/null; then
    echo "  PASS: current_time increased ($BEFORE_TIME → $AFTER_TIME)"
    PASS=$((PASS + 1))
else
    echo "  FAIL: current_time did not increase ($BEFORE_TIME → $AFTER_TIME)"
    FAIL=$((FAIL + 1))
    ERRORS="$ERRORS\n  - step-forward current_time increase"
fi

# ── Test 9: toggle (Paused→Playing) ──
# この時点でセッションは Paused 状態（pause 後 step 操作のみ）
echo "[Test 9] POST /api/replay/toggle (Paused→Playing)"
RESP=$(curl -s -X POST "${BASE}/api/replay/toggle")
assert_contains "toggle resumes replay" "$RESP" '"status":"Playing"'

# ── Test 10: toggle (Playing→Paused) ──
# is_playing()=true なので Pause が送られる
echo "[Test 10] POST /api/replay/toggle (Playing→Paused)"
RESP=$(curl -s -X POST "${BASE}/api/replay/toggle")
assert_contains "toggle pauses replay" "$RESP" '"status":"Paused"'

# ── Test 11: resume (Paused→Playing) ──
echo "[Test 11] POST /api/replay/resume"
RESP=$(curl -s -X POST "${BASE}/api/replay/resume")
assert_contains "resume returns Playing" "$RESP" '"status":"Playing"'

# ── Test 12: POST /api/app/set-mode (Replay→Live) ──
echo "[Test 12] POST /api/app/set-mode {mode:live} (Replay→Live)"
RESP=$(curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"live"}')
assert_contains "mode back to Live" "$RESP" '"mode":"Live"'

# ── Test 13: status after returning to Live ──
echo "[Test 13] GET /api/replay/status (back to Live)"
RESP=$(curl -s "${BASE}/api/replay/status")
assert_contains "mode is Live again" "$RESP" '"mode":"Live"'

# ── Test 14: 不正パス → 404 ──
echo "[Test 14] GET /api/replay/unknown → 404"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "${BASE}/api/replay/unknown")
assert_http_status "unknown path returns 404" "$STATUS" "404"

# ── Test 15: 不正 JSON body → 400 ──
echo "[Test 15] POST /api/replay/play with bad JSON → 400"
# Replay モードに切り替えてからテスト
curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"replay"}' > /dev/null
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "${BASE}/api/replay/play" -d "not json")
assert_http_status "bad JSON returns 400" "$STATUS" "400"

# ── Test 16: POST /api/replay/play with missing fields → 400 ──
echo "[Test 16] POST /api/replay/play with missing 'end' → 400"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "${BASE}/api/replay/play" -d '{"start":"2026-04-01 09:00"}')
assert_http_status "missing end returns 400" "$STATUS" "400"

# ── Test 17: POST /api/replay/play with empty body → 400 ──
echo "[Test 17] POST /api/replay/play with empty body → 400"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" -X POST "${BASE}/api/replay/play" -d '')
assert_http_status "empty body returns 400" "$STATUS" "400"

# ── Test 18: double set-mode round-trip ──
echo "[Test 18] Double set-mode round-trip (Replay→Live→Replay→Live)"
# Currently in Replay from Test 15
RESP=$(curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"live"}')  # Replay→Live
assert_contains "set-mode to Live" "$RESP" '"mode":"Live"'
RESP=$(curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"replay"}')  # Live→Replay
assert_contains "set-mode to Replay" "$RESP" '"mode":"Replay"'
RESP=$(curl -s -X POST "${BASE}/api/app/set-mode" -d '{"mode":"live"}')  # Replay→Live
assert_contains "set-mode back to Live" "$RESP" '"mode":"Live"'

echo ""
echo "=== Results ==="
echo "Passed: $PASS"
echo "Failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
    echo -e "Failures:$ERRORS"
    exit 1
fi
echo "All tests passed!"
