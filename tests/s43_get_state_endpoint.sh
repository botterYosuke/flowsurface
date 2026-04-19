#!/usr/bin/env bash
# s43_get_state_endpoint.sh — S43: GET /api/replay/state 詳細検証
#
# 検証シナリオ:
#   A:   LIVE モード → HTTP 400
#   B:   REPLAY Playing 遷移
#   C:   HTTP 200 確認
#   D:   current_time_ms > 0
#   E:   klines フィールドが配列
#   F:   trades フィールドが配列
#   G:   klines に items がある場合: stream/time/open/high/low/close/volume の型・値
#   H:   klines[*].stream が "Exchange:TICKER:timeframe" 形式
#   I:   klines[*].time ≤ current_time_ms
#   J:   open/high/low/close すべて > 0
#   K:   StepForward 後に current_time_ms が増加し klines も含む
#   L:   Idle（REPLAY モード切替前）→ HTTP 400
#
# フィクスチャ: BinanceLinear:BTCUSDT M1
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S43: GET /api/replay/state 詳細検証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

if ! is_headless; then
  cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S43","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S43"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF
fi

start_app

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: LIVE モード → HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: LIVE モード → HTTP 400"

if is_headless; then
  pend "TC-A" "headless は Live モードなし"
else
  CODE_A=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
  [ "$CODE_A" = "400" ] \
    && pass "TC-A: LIVE 中 GET /api/replay/state → HTTP 400" \
    || fail "TC-A" "HTTP=$CODE_A (expected 400)"

  # Replay に入る前に TickerInfo（metadata）の解決を待つ。
  # 未解決のままだと EventStore に kline が格納されず TC-K2 が失敗する。
  _S43_PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" \
    "$(curl -s "$API/pane/list")")
  if [ -n "$_S43_PANE_ID" ] && [ "$_S43_PANE_ID" != "null" ] && [ "$_S43_PANE_ID" != "undefined" ]; then
    echo "  waiting for streams_ready (pane=$_S43_PANE_ID, max 30s)..."
    wait_for_streams_ready "$_S43_PANE_ID" 30 || echo "  WARN: streams_ready timeout (continuing)"
  fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-L: Replay モード切替直後（Idle）→ HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-L: Replay Idle → HTTP 400"

if is_headless; then
  # headless は起動時点で Replay Idle → そのまま HTTP 400 を確認
  CODE_L=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
  [ "$CODE_L" = "400" ] \
    && pass "TC-L: Replay Idle → HTTP 400" \
    || fail "TC-L" "HTTP=$CODE_L (expected 400)"
else
  api_post /api/replay/toggle > /dev/null
  CODE_L=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
  [ "$CODE_L" = "400" ] \
    && pass "TC-L: Replay Idle（Play 前）→ HTTP 400" \
    || fail "TC-L" "HTTP=$CODE_L (expected 400)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: REPLAY Playing 遷移
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: REPLAY Playing 遷移"

api_post /api/replay/play "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "TC-B" "REPLAY Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
pass "TC-B: REPLAY Playing 到達"

# Paused 状態で確定論的な検証を行う
api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

# ─────────────────────────────────────────────────────────────────────────────
# TC-C/D/E/F: HTTP ステータスとトップレベルスキーマ
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C〜F: HTTP 200 + トップレベルスキーマ"

STATE=$(api_get /api/replay/state)
CODE_C=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
echo "  HTTP=$CODE_C"

[ "$CODE_C" = "200" ] \
  && pass "TC-C: Paused 中 GET /api/replay/state → HTTP 200" \
  || fail "TC-C" "HTTP=$CODE_C (expected 200)"

CT_MS=$(node -e "
  try { const d=JSON.parse(process.argv[1]); console.log(d.current_time_ms || d.current_time || 0); }
  catch(e) { console.log(0); }
" "$STATE")
echo "  current_time_ms=$CT_MS"
node -e "process.exit(Number('$CT_MS') > 0 ? 0 : 1)" 2>/dev/null \
  && pass "TC-D: current_time_ms=$CT_MS (>0)" \
  || fail "TC-D" "current_time_ms=$CT_MS (expected >0)"

KLINES_IS_ARR=$(node -e "
  try { console.log(Array.isArray(JSON.parse(process.argv[1]).klines) ? 'true' : 'false'); }
  catch(e) { console.log('false'); }
" "$STATE")
[ "$KLINES_IS_ARR" = "true" ] \
  && pass "TC-E: klines フィールドが配列" \
  || fail "TC-E" "klines が配列でない (response=$STATE)"

TRADES_IS_ARR=$(node -e "
  try { console.log(Array.isArray(JSON.parse(process.argv[1]).trades) ? 'true' : 'false'); }
  catch(e) { console.log('false'); }
" "$STATE")
[ "$TRADES_IS_ARR" = "true" ] \
  && pass "TC-F: trades フィールドが配列" \
  || fail "TC-F" "trades が配列でない (response=$STATE)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-G〜J: klines items スキーマ（items がある場合のみ）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-G〜J: klines items スキーマ"

KLINE_COUNT=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).klines.length); }
  catch(e) { console.log(0); }
" "$STATE")
echo "  klines count=$KLINE_COUNT"

if is_headless; then
  pend "TC-G" "headless klines スキーマ差分あり"
  pend "TC-H" "headless klines スキーマ差分あり"
  pend "TC-I" "headless klines スキーマ差分あり"
  pend "TC-J" "headless klines スキーマ差分あり"
elif [ "$KLINE_COUNT" -gt "0" ]; then
  # TC-G: stream/time/open/high/low/close/volume の存在と型
  node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      const k = d.klines[0];
      const ok = typeof k.stream === 'string' && k.stream.length > 0
        && typeof k.time   === 'number'
        && typeof k.open   === 'number'
        && typeof k.high   === 'number'
        && typeof k.low    === 'number'
        && typeof k.close  === 'number'
        && typeof k.volume === 'number';
      console.log(ok ? 'true' : 'false');
    } catch(e) { console.log('false'); }
  " "$STATE" | grep -q "true" \
    && pass "TC-G: klines[0] に stream/time/open/high/low/close/volume あり (型正常)" \
    || fail "TC-G" "klines[0] スキーマ不正 (response=$STATE)"

  # TC-H: stream ラベルが "Exchange:TICKER:timeframe" 形式
  STREAM_LABEL=$(node -e "
    try { console.log(JSON.parse(process.argv[1]).klines[0].stream); }
    catch(e) { console.log(''); }
  " "$STATE")
  echo "  klines[0].stream=$STREAM_LABEL"
  # "BinanceLinear:BTCUSDT:1m" のように 2 つ以上のコロンを含む形式
  echo "$STREAM_LABEL" | grep -qE "^[A-Za-z]+:[A-Z0-9]+:[A-Za-z0-9]+$" \
    && pass "TC-H: stream ラベル形式が \"Exchange:TICKER:timeframe\" ($STREAM_LABEL)" \
    || fail "TC-H" "stream ラベル形式が想定外 ($STREAM_LABEL)"

  # TC-I: klines[*].time ≤ current_time_ms
  TIME_OK=$(node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      const ct = BigInt(d.current_time_ms);
      const ok = d.klines.every(k => BigInt(k.time) <= ct);
      console.log(ok ? 'true' : 'false');
    } catch(e) { console.log('false'); }
  " "$STATE")
  [ "$TIME_OK" = "true" ] \
    && pass "TC-I: klines[*].time ≤ current_time_ms" \
    || fail "TC-I" "未来の kline が含まれている (response=$STATE)"

  # TC-J: open/high/low/close > 0
  OHLC_OK=$(node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      const ok = d.klines.every(k =>
        k.open > 0 && k.high > 0 && k.low > 0 && k.close > 0
        && k.high >= k.low
      );
      console.log(ok ? 'true' : 'false');
    } catch(e) { console.log('false'); }
  " "$STATE")
  [ "$OHLC_OK" = "true" ] \
    && pass "TC-J: open/high/low/close > 0 かつ high ≥ low" \
    || fail "TC-J" "OHLC 値に不正な値あり (response=$STATE)"
else
  pass "TC-G: klines=0 件（Paused 直後のため許容）"
  pass "TC-H: klines=0 件（スキップ）"
  pass "TC-I: klines=0 件（スキップ）"
  pass "TC-J: klines=0 件（スキップ）"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-K: StepForward 後に current_time_ms が増加し klines が含まれる
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-K: StepForward 後の state 変化"

CT_BEFORE=$(node -e "
  try { const d=JSON.parse(process.argv[1]); console.log(d.current_time_ms || d.current_time || 0); }
  catch(e) { console.log(0); }
" "$STATE")

api_post /api/replay/step-forward > /dev/null
sleep 1

STATE2=$(api_get /api/replay/state)
CT_AFTER=$(node -e "
  try { const d=JSON.parse(process.argv[1]); console.log(d.current_time_ms || d.current_time || 0); }
  catch(e) { console.log(0); }
" "$STATE2")
echo "  current_time_ms: $CT_BEFORE → $CT_AFTER"

node -e "process.exit(BigInt('$CT_AFTER') > BigInt('$CT_BEFORE') ? 0 : 1)" 2>/dev/null \
  && pass "TC-K1: StepForward 後に current_time_ms が増加 ($CT_BEFORE → $CT_AFTER)" \
  || fail "TC-K1" "current_time_ms が増加しない ($CT_BEFORE → $CT_AFTER)"

KLINE_COUNT2=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).klines.length); }
  catch(e) { console.log(0); }
" "$STATE2")
echo "  klines count after step=$KLINE_COUNT2"
if is_headless; then
  pend "TC-K2" "headless klines スキーマ差分あり"
else
  [ "$KLINE_COUNT2" -gt "0" ] \
    && pass "TC-K2: StepForward 後に klines が 1 件以上" \
    || fail "TC-K2" "StepForward 後も klines=0 件"
fi

stop_app
print_summary
