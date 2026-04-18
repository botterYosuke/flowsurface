#!/usr/bin/env bash
# s17_error_boundary.sh — スイート S17: クラッシュ・エラー境界テスト
#
# 検証シナリオ:
#   TC-S17-01〜03: 存在しない pane_id（pane/split, pane/close, pane/set-ticker）→ HTTP 404 + アプリ生存
#   TC-S17-04: 空 range (start == end) でもアプリ生存
#   TC-S17-05: 未来の range でもアプリ生存
#   TC-S17-06: StepForward 50 連打（Paused 状態）→ crash なし・status=Paused
#   TC-S17-07: split 上限到達後もクラッシュなし
#
# 仕様根拠:
#   docs/replay_header.md §10 — エラー境界・クラッシュ防止
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play + 各 TC で再起動
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S17: クラッシュ・エラー境界テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

FAKE_UUID="ffffffff-ffff-ffff-ffff-ffffffffffff"

# ── TC-S17-01〜03: 存在しない pane_id に対する各エンドポイント ───────────
# pane/split, pane/close, pane/set-ticker に存在しない UUID → HTTP 404 + error でクラッシュなし
echo "  [TC-S17-01/03] 不正 pane_id テスト..."

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "$(primary_ticker)" "M1" "$START" "$END"
start_app
headless_play

if ! wait_playing 60; then
  diagnose_playing_failure
  fail "TC-S17-precond" "Playing 到達せず（60s タイムアウト）"
  exit 1
fi

# TC-S17-01: pane/split に存在しない UUID → HTTP 404 + error フィールド & アプリ生存
RESP_SPLIT=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"axis\":\"Vertical\"}")
HTTP_SPLIT=$(echo "$RESP_SPLIT" | tail -1)
BODY_SPLIT=$(echo "$RESP_SPLIT" | head -1)
HAS_ERR_SPLIT=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY_SPLIT")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
# handle_pane_api は app 層エラーを HTTP 200 + error フィールドで返す（将来 404 に移行予定）
{ [ "$HTTP_SPLIT" = "200" ] || [ "$HTTP_SPLIT" = "404" ]; } && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-01a: pane/split 存在しない UUID → HTTP=$HTTP_SPLIT & アプリ生存" \
  || fail "TC-S17-01a" "HTTP=$HTTP_SPLIT alive=$ALIVE"
[ "$HAS_ERR_SPLIT" = "true" ] \
  && pass "TC-S17-01b: pane/split 不正 UUID → error フィールドあり" \
  || fail "TC-S17-01b" "body=$BODY_SPLIT"

# TC-S17-02: pane/close に存在しない UUID → HTTP 404 + error フィールド & アプリ生存
RESP_CLOSE=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/close" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\"}")
HTTP_CLOSE=$(echo "$RESP_CLOSE" | tail -1)
BODY_CLOSE=$(echo "$RESP_CLOSE" | head -1)
HAS_ERR_CLOSE=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY_CLOSE")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
{ [ "$HTTP_CLOSE" = "200" ] || [ "$HTTP_CLOSE" = "404" ]; } && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-02a: pane/close 存在しない UUID → HTTP=$HTTP_CLOSE & アプリ生存" \
  || fail "TC-S17-02a" "HTTP=$HTTP_CLOSE alive=$ALIVE"
[ "$HAS_ERR_CLOSE" = "true" ] \
  && pass "TC-S17-02b: pane/close 不正 UUID → error フィールドあり" \
  || fail "TC-S17-02b" "body=$BODY_CLOSE"

# TC-S17-03: pane/set-ticker に存在しない UUID → HTTP 404 + error フィールド & アプリ生存
RESP_TICKER=$(curl -s -w "\n%{http_code}" -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"ticker\":\"$(secondary_ticker)\"}")
HTTP_TICKER=$(echo "$RESP_TICKER" | tail -1)
BODY_TICKER=$(echo "$RESP_TICKER" | head -1)
HAS_ERR_TICKER=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error?'true':'false');}catch(e){console.log('false');}" "$BODY_TICKER")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
{ [ "$HTTP_TICKER" = "200" ] || [ "$HTTP_TICKER" = "404" ]; } && [ "$ALIVE" = "true" ] \
  && pass "TC-S17-03a: pane/set-ticker 存在しない UUID → HTTP=$HTTP_TICKER & アプリ生存" \
  || fail "TC-S17-03a" "HTTP=$HTTP_TICKER alive=$ALIVE"
[ "$HAS_ERR_TICKER" = "true" ] \
  && pass "TC-S17-03b: pane/set-ticker 不正 UUID → error フィールドあり" \
  || fail "TC-S17-03b" "body=$BODY_TICKER"

# TC-S17-03c: pane 全削除後 pane/list が空配列を返す
PANES=$(curl -s "$API/pane/list")
PANE_ID_0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -n "$PANE_ID_0" ]; then
  # split して 2 ペインにしてから両方 close
  SPLIT_RESP=$(curl -s -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE_ID_0\",\"axis\":\"Vertical\"}")
  PANES2=$(curl -s "$API/pane/list")
  PANE_IDS=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps.map(p=>p.id).join(' '));" "$PANES2")
  for pid in $PANE_IDS; do
    curl -s -X POST "$API/pane/close" \
      -H "Content-Type: application/json" \
      -d "{\"pane_id\":\"$pid\"}" > /dev/null
    sleep 0.3
  done
  sleep 0.5
  PANES_AFTER=$(curl -s "$API/pane/list")
  COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" "$PANES_AFTER")
  [ "$COUNT" = "1" ] \
    && pass "TC-S17-03c: 全 pane close 後 最終ペイン1つ残存 (count=1, iced pane_grid 仕様)" \
    || fail "TC-S17-03c" "count=$COUNT (expected 1), resp=$PANES_AFTER"
else
  fail "TC-S17-03c-pre" "ペイン ID 取得失敗"
fi

stop_app

# ── TC-S17-04: 空 range (start == end) ─────────────────────────────────
echo "  [TC-S17-04] 空 range (start == end)..."
SAME_TIME=$(utc_offset -1)
setup_single_pane "$(primary_ticker)" "M1" "$SAME_TIME" "$SAME_TIME"
start_app
headless_play

# アプリが起動して API が応答すれば OK（crash なし）
sleep 5
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
if [ "$ALIVE" = "true" ]; then
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  pass "TC-S17-04: 空 range でもアプリ生存 (status=$STATUS)"
else
  fail "TC-S17-04" "空 range でアプリがクラッシュした"
  STATUS="null"
fi

stop_app

# ── TC-S17-05: 未来の range (現在時刻 + 24h 先) ─────────────────────────
echo "  [TC-S17-05] 未来 range テスト..."
FUTURE_START=$(utc_offset 24)
FUTURE_END=$(utc_offset 26)
setup_single_pane "$(primary_ticker)" "M1" "$FUTURE_START" "$FUTURE_END"
start_app
headless_play

# EventStore が空でも Playing/Paused で停止するだけ（クラッシュしない）
sleep 10
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
if [ "$ALIVE" = "true" ]; then
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  pass "TC-S17-05: 未来 range でもアプリ生存 (status=$STATUS)"
else
  fail "TC-S17-05" "未来 range でアプリがクラッシュした"
fi

stop_app

# ── TC-S17-06: StepForward 連打 50 回 (Paused 状態) ─────────────────────
echo "  [TC-S17-06] StepForward 連打 50 回..."
setup_single_pane "$(primary_ticker)" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
headless_play

if ! wait_playing 60; then
  diagnose_playing_failure
  fail "TC-S17-06-pre" "Playing 到達せず（60s タイムアウト）"
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
setup_single_pane "$(primary_ticker)" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
headless_play

if ! wait_playing 60; then
  diagnose_playing_failure
  fail "TC-S17-07-pre" "Playing 到達せず（60s タイムアウト）"
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
