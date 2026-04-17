#!/usr/bin/env bash
# x4_virtual_order_live_guard.sh — X4: 仮想注文 LIVE モードガード (クイック検証)
#
# 検証シナリオ:
#   01-03: LIVE モード時は /order・/portfolio・/state がすべて HTTP 400
#   04-05: Replay (Idle) モードに切替後は /order・/portfolio が HTTP 200
#   06:    LIVE モードに戻すと /order が再び HTTP 400（ガード復元）
#
# 仕様根拠:
#   docs/replay_header.md §11.2 — 「REPLAY モード専用。LIVE モード時は 400 を返す。」
#   docs/order_windows.md §REPLAY モード Safety Guard
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, Live モード起動
#   (Replay Play 不要。モードトグルのみで検証可能なため高速)
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== X4: 仮想注文 LIVE モードガード ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

if ! is_headless; then
  cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X4","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"X4"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF
fi

start_app

if is_headless; then
  # headless は常に Replay モードのため Live guard テストは非対応
  pend "TC-01" "headless は Live モードなし"
  pend "TC-02" "headless は Live モードなし"
  pend "TC-03" "headless は Live モードなし"
  # TC-04/05: headless は既に Replay Idle → HTTP 200
  CODE_04=$(api_post_code /api/replay/order \
    '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
  [ "$CODE_04" = "200" ] \
    && pass "TC-04: Replay 中 POST /api/replay/order → HTTP 200" \
    || fail "TC-04" "HTTP=$CODE_04 (expected 200)"
  CODE_05=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/portfolio")
  [ "$CODE_05" = "200" ] \
    && pass "TC-05: Replay 中 GET /api/replay/portfolio → HTTP 200" \
    || fail "TC-05" "HTTP=$CODE_05 (expected 200)"
  pend "TC-06" "headless は Live モードなし"
  stop_app
  print_summary
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-01〜03: LIVE モード時は全エンドポイントが HTTP 400
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-01〜03: LIVE モード時は HTTP 400"

LIVE_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
[ "$LIVE_MODE" = "Live" ] || { fail "precond" "LIVE モード起動失敗 (mode=$LIVE_MODE)"; print_summary; exit 1; }
echo "  起動モード確認: mode=$LIVE_MODE"

CODE_01=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
[ "$CODE_01" = "400" ] \
  && pass "TC-01: LIVE 中 POST /api/replay/order → HTTP 400" \
  || fail "TC-01" "HTTP=$CODE_01 (expected 400)"

CODE_02=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/portfolio")
[ "$CODE_02" = "400" ] \
  && pass "TC-02: LIVE 中 GET /api/replay/portfolio → HTTP 400" \
  || fail "TC-02" "HTTP=$CODE_02 (expected 400)"

CODE_03=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/state")
[ "$CODE_03" = "400" ] \
  && pass "TC-03: LIVE 中 GET /api/replay/state → HTTP 400" \
  || fail "TC-03" "HTTP=$CODE_03 (expected 400)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-04〜05: Replay モード（Idle）に切替後は HTTP 200
# Play 不要。toggle で mode=Replay になれば VirtualExchangeEngine が初期化される。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-04〜05: Replay (Idle) モードに切替 → HTTP 200"

api_post /api/replay/toggle > /dev/null
REPLAY_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
echo "  toggle 後のモード: $REPLAY_MODE"
[ "$REPLAY_MODE" = "Replay" ] || { fail "precond" "Replay モード遷移失敗 (mode=$REPLAY_MODE)"; print_summary; exit 1; }

CODE_04=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
[ "$CODE_04" = "200" ] \
  && pass "TC-04: Replay (Idle) 中 POST /api/replay/order → HTTP 200" \
  || fail "TC-04" "HTTP=$CODE_04 (expected 200)"

CODE_05=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/portfolio")
[ "$CODE_05" = "200" ] \
  && pass "TC-05: Replay (Idle) 中 GET /api/replay/portfolio → HTTP 200" \
  || fail "TC-05" "HTTP=$CODE_05 (expected 200)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-06: LIVE モードに戻すと HTTP 400 が復元される
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-06: LIVE に戻すと HTTP 400 が復元される"

api_post /api/replay/toggle > /dev/null
LIVE_MODE_AGAIN=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
echo "  再 toggle 後のモード: $LIVE_MODE_AGAIN"

CODE_06=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
[ "$CODE_06" = "400" ] \
  && pass "TC-06: LIVE 復帰後 POST /api/replay/order → HTTP 400（ガード復元）" \
  || fail "TC-06" "HTTP=$CODE_06 (expected 400)"

stop_app
print_summary
