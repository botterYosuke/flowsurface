#!/usr/bin/env bash
# s24_sidebar_select_ticker.sh — S24: POST /api/sidebar/select-ticker 経路の検証
#
# 検証シナリオ:
#   TC-A: Replay Paused 中に kind=null で ticker 変更 → pane の ticker が切り替わる
#   TC-B: Replay Playing 中に kind=null で ticker 変更 → status が Paused になる
#   TC-C: Playing 中変更後 Resume → Playing 復帰
#   TC-D: kind="KlineChart" を指定した場合 → HTTP 200、エラーなし
#   TC-E: 不正な pane_id → HTTP 400
#   TC-F: ticker フィールド欠落 → HTTP 400
#
# 仕様根拠:
#   docs/replay_header.md §9.1 — Sidebar::TickerSelected 経路
#   kind=null → switch_tickers_in_group（リンクグループ全ペイン更新）
#   kind=Some → init_focused_pane（ペイン種別ごとの初期化）
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S24: POST /api/sidebar/select-ticker 経路の検証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── ヘルパー ─────────────────────────────────────────────────────────────────

get_pane_id() {
  local panes
  panes=$(curl -s "$API/pane/list")
  node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$panes"
}

get_pane_ticker() {
  local pane_id="$1"
  local panes
  panes=$(curl -s "$API/pane/list")
  node -e "
    const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.id==='$pane_id');
    console.log(p?p.ticker||'null':'null');
  " "$panes"
}

get_status() {
  jqn "$(curl -s "$API/replay/status")" "d.status"
}

poll_status() {
  local want="$1" timeout="${2:-15}"
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local s
    s=$(get_status)
    [ "$s" = "$want" ] && return 0
    sleep 0.5
  done
  return 1
}

sidebar_select() {
  local pane_id="$1" ticker="$2" kind="${3:-}"
  if [ -n "$kind" ]; then
    api_post /api/sidebar/select-ticker \
      "{\"pane_id\":\"$pane_id\",\"ticker\":\"$ticker\",\"kind\":\"$kind\"}"
  else
    api_post /api/sidebar/select-ticker \
      "{\"pane_id\":\"$pane_id\",\"ticker\":\"$ticker\"}"
  fi
}

# ── フィクスチャ: BinanceLinear BTCUSDT M1, 過去 3h〜1h（autoplay）────────────

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S24","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S24"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# autoplay で Playing に到達するまで待機
if ! wait_status "Playing" 60; then
  fail "S24-precond" "Playing 到達せず（timeout）"
  print_summary
  exit 1
fi

PANE_ID=$(get_pane_id)
echo "  PANE_ID=$PANE_ID"
if [ -z "$PANE_ID" ]; then
  fail "S24-precond" "ペイン ID 取得失敗"
  print_summary
  exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Playing 中に sidebar/select-ticker (kind=null) → 即座に Paused
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: Playing 中に sidebar/select-ticker → Paused"
RESP_B=$(sidebar_select "$PANE_ID" "BinanceLinear:ETHUSDT")
HTTP_OK_B=$(node -e "
  try { JSON.parse(process.argv[1]); console.log('ok'); }
  catch(e) { console.log('err'); }
" "$RESP_B")
[ "$HTTP_OK_B" = "ok" ] \
  && pass "TC-B1: sidebar/select-ticker レスポンスが JSON" \
  || fail "TC-B1" "レスポンス: $RESP_B"

sleep 0.5
ST_B=$(get_status)
[ "$ST_B" = "Paused" ] \
  && pass "TC-B2: sidebar/select-ticker 後 status=Paused" \
  || fail "TC-B2" "status=$ST_B (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Paused 中に sidebar/select-ticker (kind=null) → ticker が変わる
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: Paused 中に sidebar/select-ticker → ticker 変更確認"
# 現在 ETHUSDT に変更済み。SOLUSDT にもう一度変更して確認。
RESP_A=$(sidebar_select "$PANE_ID" "BinanceLinear:SOLUSDT")
HTTP_OK_A=$(node -e "
  try { JSON.parse(process.argv[1]); console.log('ok'); }
  catch(e) { console.log('err'); }
" "$RESP_A")
[ "$HTTP_OK_A" = "ok" ] \
  && pass "TC-A1: Paused 中の sidebar/select-ticker がエラーなし" \
  || fail "TC-A1" "レスポンス: $RESP_A"

# streams_ready を待機（ticker 変更後にバックフィルが走る）
if wait_for_streams_ready "$PANE_ID" 30; then
  TICKER_A=$(get_pane_ticker "$PANE_ID")
  [ "$TICKER_A" = "BinanceLinear:SOLUSDT" ] \
    && pass "TC-A2: ticker が BinanceLinear:SOLUSDT に変更された" \
    || fail "TC-A2" "ticker=$TICKER_A (expected BinanceLinear:SOLUSDT)"
else
  fail "TC-A2" "streams_ready タイムアウト（30s）"
fi

# status は Paused のまま（自動再生されない）
ST_A=$(get_status)
[ "$ST_A" = "Paused" ] \
  && pass "TC-A3: ticker 変更後も status=Paused（自動再生なし）" \
  || fail "TC-A3" "status=$ST_A (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Paused → Resume → Playing 復帰
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: sidebar/select-ticker 後 Resume → Playing"
api_post /api/replay/resume > /dev/null
if poll_status "Playing" 30; then
  pass "TC-C: Resume 後 status=Playing"
else
  fail "TC-C" "status=$(get_status) (expected Playing)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: kind="KlineChart" を指定（init_focused_pane 経路）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: kind=KlineChart を指定した sidebar/select-ticker"
RESP_D=$(sidebar_select "$PANE_ID" "BinanceLinear:BTCUSDT" "KlineChart")
HTTP_OK_D=$(node -e "
  try { JSON.parse(process.argv[1]); console.log('ok'); }
  catch(e) { console.log('err'); }
" "$RESP_D")
[ "$HTTP_OK_D" = "ok" ] \
  && pass "TC-D: kind=KlineChart 指定で HTTP 200 JSON レスポンス" \
  || fail "TC-D" "レスポンス: $RESP_D"

# エラー toast が出ていないことを確認
NOTIFS_D=$(curl -s "$API/notification/list")
HAS_ERR_D=$(node -e "
  const ns=(JSON.parse(process.argv[1]).notifications||[]);
  console.log(ns.some(n=>n.level==='error')?'true':'false');
" "$NOTIFS_D")
[ "$HAS_ERR_D" = "false" ] \
  && pass "TC-D2: kind=KlineChart でエラー toast なし" \
  || fail "TC-D2" "error toast が発生した"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E: 不正な pane_id → HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E: 不正な pane_id → HTTP 400"
HTTP_CODE_E=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST -H "Content-Type: application/json" \
  -d '{"pane_id":"not-a-uuid","ticker":"BinanceLinear:BTCUSDT"}' \
  "$API_BASE/api/sidebar/select-ticker")
[ "$HTTP_CODE_E" = "400" ] \
  && pass "TC-E: 不正 pane_id → HTTP 400" \
  || fail "TC-E" "HTTP=$HTTP_CODE_E (expected 400)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-F: ticker フィールド欠落 → HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F: ticker フィールド欠落 → HTTP 400"
HTTP_CODE_F=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$PANE_ID\"}" \
  "$API_BASE/api/sidebar/select-ticker")
[ "$HTTP_CODE_F" = "400" ] \
  && pass "TC-F: ticker 欠落 → HTTP 400" \
  || fail "TC-F" "HTTP=$HTTP_CODE_F (expected 400)"

stop_app

print_summary
