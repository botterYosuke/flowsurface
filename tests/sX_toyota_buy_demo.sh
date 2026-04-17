#!/usr/bin/env bash
# sX_toyota_buy_demo.sh — TOYOTA (TachibanaSpot:7203) Live モード 注文パネル デモテスト
#
# 検証シナリオ:
#   1: アプリ Live モードで起動、TOYOTA (TachibanaSpot:7203) ペインをセット
#   2: OrderEntry / OrderList / BuyingPower パネルを順に開く
#   3: Live モードで /api/replay/order が HTTP 400 を返す（仮想注文はReplay専用）
#   4: Tachibana 認証状態を確認
#   5: スクリーンショット取得
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# F-2: デモ環境ガード（本番誤発注防止）
[ "${DEV_IS_DEMO:-}" = "true" ] \
  || { echo "ERROR: DEV_IS_DEMO=true を設定してください（本番誤発注防止）"; exit 1; }
[ -n "${DEV_USER_ID:-}" ] \
  || { echo "ERROR: DEV_USER_ID が未設定です"; exit 1; }
[ -n "${DEV_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_PASSWORD が未設定です"; exit 1; }
[ -n "${DEV_SECOND_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_SECOND_PASSWORD が未設定です"; exit 1; }

echo "=== TOYOTA Live モード 注文パネル デモテスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# Live モードで起動（replay キー無し）
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"Toyota-Live","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"Toyota-Live"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

echo "  fixture: TachibanaSpot:7203 D1, Live モード"

# ── アプリ起動 ───────────────────────────────────────────────────────────────
start_app

# ── Step 1: Live モード確認 ───────────────────────────────────────────────────
echo ""
echo "── Step 1: Live モード確認"

STATUS=$(curl -s "$API/replay/status")
echo "  status: $STATUS"
MODE=$(node -e "try{console.log(JSON.parse(process.argv[1]).mode||'null');}catch(e){console.log('null');}" "$STATUS")
[ "$MODE" = "Live" ] \
  && pass "Step 1: Live モードで起動確認 (mode=$MODE)" \
  || fail "Step 1" "mode=$MODE (expected Live)"

# ── Step 2: TOYOTA ペイン確認 ─────────────────────────────────────────────────
echo ""
echo "── Step 2: TOYOTA ペイン (TachibanaSpot:7203) 確認"

PANES=$(curl -s "$API/pane/list")
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
TICKER0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?(ps[0].ticker||'null'):'null');" "$PANES")
echo "  PANE0=$PANE0  ticker=$TICKER0"

if echo "$TICKER0" | grep -q "7203"; then
  pass "Step 2: TOYOTA ペイン確認 (ticker=$TICKER0)"
else
  fail "Step 2" "ticker=$TICKER0 (expected to contain 7203)"
fi

# ── Step 3: 注文パネルを順に開く ──────────────────────────────────────────────
echo ""
echo "── Step 3: 注文パネルを開く (OrderEntry / OrderList / BuyingPower)"

CODE_OE=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/sidebar/open-order-pane" \
  -H "Content-Type: application/json" \
  -d '{"kind":"OrderEntry"}')
[ "$CODE_OE" = "200" ] \
  && pass "Step 3a: OrderEntry パネル → HTTP 200" \
  || fail "Step 3a" "HTTP=$CODE_OE"
wait_for_pane_count 2 10 || true

CODE_OL=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/sidebar/open-order-pane" \
  -H "Content-Type: application/json" \
  -d '{"kind":"OrderList"}')
[ "$CODE_OL" = "200" ] \
  && pass "Step 3b: OrderList パネル → HTTP 200" \
  || fail "Step 3b" "HTTP=$CODE_OL"
wait_for_pane_count 3 10 || true

CODE_BP=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/sidebar/open-order-pane" \
  -H "Content-Type: application/json" \
  -d '{"kind":"BuyingPower"}')
[ "$CODE_BP" = "200" ] \
  && pass "Step 3c: BuyingPower パネル → HTTP 200" \
  || fail "Step 3c" "HTTP=$CODE_BP"

sleep 1

# ── Step 4: Live モードでは仮想注文 API が HTTP 400 ────────────────────────────
echo ""
echo "── Step 4: Live モードで仮想注文 API ガード確認"

CODE_ORDER=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/replay/order" \
  -H "Content-Type: application/json" \
  -d '{"ticker":"TachibanaSpot:7203","side":"buy","qty":100,"order_type":"market"}')
[ "$CODE_ORDER" = "400" ] \
  && pass "Step 4: Live 中 POST /api/replay/order → HTTP 400（仮想注文はReplay専用）" \
  || fail "Step 4" "HTTP=$CODE_ORDER (expected 400)"

# ── Step 5: Tachibana 認証状態確認 ───────────────────────────────────────────
echo ""
echo "── Step 5: Tachibana 認証状態確認"

AUTH=$(curl -s "$API/auth/tachibana/status")
echo "  auth status: $AUTH"
echo "  auth status: $AUTH"
# F-1: セッション未確立の場合は fail にする
wait_tachibana_session 60 \
  && pass "Step 5: Tachibana デモセッション確立" \
  || fail "Step 5" "セッション未確立 — DEV_IS_DEMO=true でデモログインが必要"

# ── Step 6: スクリーンショット取得 ───────────────────────────────────────────
echo ""
echo "── Step 6: スクリーンショット取得"
SHOT_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/app/screenshot")
[ "$SHOT_CODE" = "200" ] \
  && pass "Step 6: スクリーンショット保存 → /tmp/screenshot.png" \
  || fail "Step 6" "HTTP=$SHOT_CODE"

# ── Step 7: TOYOTA 100株 成行買い注文 ─────────────────────────────────────────
echo ""
echo "── Step 7: TOYOTA (7203) 100株 成行買い注文"

  ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
    -H "Content-Type: application/json" \
    -d '{
      "issue_code":   "7203",
      "qty":          "100",
      "side":         "3",
      "price":        "0",
      "account_type": "1",
      "market_code":  "00",
      "condition":    "0",
      "cash_margin":  "0",
      "expire_day":   "0"
    }')
  echo "  response: $ORDER_RESP"
  ORDER_NUM=$(node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      console.log(d.order_number || 'none');
    } catch(e) { console.log('none'); }
  " "$ORDER_RESP")
  if [ "$ORDER_NUM" != "none" ] && [ -n "$ORDER_NUM" ]; then
    pass "Step 7: 注文受付済み (order_number=$ORDER_NUM)"
  else
    ERROR=$(node -e "
      try { console.log(JSON.parse(process.argv[1]).error||'unknown'); }
      catch(e) { console.log('parse error'); }
    " "$ORDER_RESP")
    fail "Step 7" "注文失敗: $ERROR"
  fi

stop_app
print_summary
[ $FAIL -eq 0 ]
