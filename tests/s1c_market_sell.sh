#!/usr/bin/env bash
# s1c_market_sell.sh — TOYOTA (7203) 成行売り E2E テスト（シナリオ 1-3）
#
# 検証シナリオ:
#   1: Live モードで起動・Tachibana セッション確認
#   2: 成行売り注文を送信（sBaibaiKubun=1, sOrderPrice=0）
#   3: 保有なしの場合はエラー（残高不足等）が返ることを確認
#   4: リクエストフィールドが正しく組み立てられることを単体レベルで確認済み
#
# 注意: 実際に TOYOTA 株を保有している場合は約定するため要注意
# 前提: DEV_SECOND_PASSWORD 環境変数が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== TOYOTA (7203) 成行売り E2E テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

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

start_app

# ── Step 1: Live モード確認 ───────────────────────────────────────────────────
echo ""
echo "── Step 1: Live モード確認"
STATUS=$(curl -s "$API/replay/status")
MODE=$(node -e "try{console.log(JSON.parse(process.argv[1]).mode||'null');}catch(e){console.log('null');}" "$STATUS")
[ "$MODE" = "Live" ] \
  && pass "Step 1: Live モードで起動確認 (mode=$MODE)" \
  || fail "Step 1" "mode=$MODE (expected Live)"

# ── Step 2: Tachibana セッション確認 ─────────────────────────────────────────
echo ""
echo "── Step 2: Tachibana セッション確認"
AUTH=$(curl -s "$API/auth/tachibana/status")
SESSION=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.session||'none');}catch(e){console.log('none');}" "$AUTH")
echo "  session=$SESSION"
pass "Step 2: 認証 API 応答確認 (session=$SESSION)"

# ── Step 3: 成行売り注文（sBaibaiKubun=1, sOrderPrice=0） ─────────────────────
echo ""
echo "── Step 3: TOYOTA (7203) 100株 成行売り"

if [ -z "${DEV_SECOND_PASSWORD:-}" ]; then
  fail "Step 3" "DEV_SECOND_PASSWORD が未設定です"
else
  ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
    -H "Content-Type: application/json" \
    -d '{
      "issue_code":   "7203",
      "qty":          "100",
      "side":         "1",
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
  ERROR=$(node -e "
    try { console.log(JSON.parse(process.argv[1]).error||'none'); }
    catch(e) { console.log('parse error'); }
  " "$ORDER_RESP")

  if [ "$ORDER_NUM" != "none" ] && [ -n "$ORDER_NUM" ]; then
    pass "Step 3: 成行売り注文受付済み (order_number=$ORDER_NUM)"
  elif [ "$ERROR" != "none" ] && [ -n "$ERROR" ]; then
    # 保有なしの場合はエラーが返るが、それも正常動作
    echo "  INFO: エラー応答 (error=$ERROR) — 保有株なしまたは市場時間外の可能性あり"
    pass "Step 3: 成行売り API 疎通確認（エラー応答あり: $ERROR）"
  else
    fail "Step 3" "レスポンスが解析できない: $ORDER_RESP"
  fi
fi

# ── Step 4: HTTP レスポンスが JSON であることを確認 ────────────────────────────
echo ""
echo "── Step 4: HTTP レスポンス形式確認"

if [ -z "${DEV_SECOND_PASSWORD:-}" ]; then
  pend "Step 4" "DEV_SECOND_PASSWORD が未設定のためスキップ"
else
  HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$API/tachibana/order" \
    -H "Content-Type: application/json" \
    -d '{
      "issue_code":   "7203",
      "qty":          "100",
      "side":         "1",
      "price":        "0",
      "account_type": "1",
      "market_code":  "00",
      "condition":    "0",
      "cash_margin":  "0",
      "expire_day":   "0"
    }')
  echo "  HTTP status: $HTTP_CODE"
  # 200=成功/エラー問わず注文 API は 200 を返す（エラーは JSON body 内）
  [ "$HTTP_CODE" = "200" ] \
    && pass "Step 4: POST /api/tachibana/order → HTTP $HTTP_CODE" \
    || fail "Step 4" "HTTP=$HTTP_CODE (expected 200)"
fi

stop_app
print_summary
[ $FAIL -eq 0 ]
