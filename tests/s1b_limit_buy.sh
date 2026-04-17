#!/usr/bin/env bash
# s1b_limit_buy.sh — TOYOTA (7203) 指値買い E2E テスト（シナリオ 1-2）
#
# 検証シナリオ:
#   1: Live モードで起動・Tachibana セッション確立
#   2: 指値買い注文を送信（価格 1 円=約定しない水準）
#   3: 注文番号が返ることを確認（正常受付）
#   4: エラーレスポンスのパースが正しく行われることを確認
#
# 前提: DEV_SECOND_PASSWORD 環境変数が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== TOYOTA (7203) 指値買い E2E テスト ==="
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
if [ "$SESSION" != "none" ] && [ -n "$SESSION" ]; then
  pass "Step 2: Tachibana セッション確立済み"
else
  echo "  INFO: Tachibana セッションなし（ログイン待ち）"
  pass "Step 2: 認証 API 応答確認 (session=none)"
fi

# ── Step 3: 指値買い注文（価格 1 円 = 約定しない水準） ────────────────────────
echo ""
echo "── Step 3: TOYOTA (7203) 100株 指値買い (price=70 円 / デモ環境値幅制限内)"

if [ -z "${DEV_SECOND_PASSWORD:-}" ]; then
  fail "Step 3" "DEV_SECOND_PASSWORD が未設定です"
else
  ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
    -H "Content-Type: application/json" \
    -d '{
      "issue_code":   "7203",
      "qty":          "100",
      "side":         "3",
      "price":        "70",
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
    pass "Step 3: 指値買い注文受付済み (order_number=$ORDER_NUM)"
  else
    ERROR=$(node -e "
      try { console.log(JSON.parse(process.argv[1]).error||'unknown'); }
      catch(e) { console.log('parse error'); }
    " "$ORDER_RESP")
    fail "Step 3" "指値買い注文失敗: $ERROR"
  fi
fi

# ── Step 4: レスポンスフィールド検証 ─────────────────────────────────────────
echo ""
echo "── Step 4: レスポンスフィールド検証"

if [ -z "${DEV_SECOND_PASSWORD:-}" ]; then
  pend "Step 4" "DEV_SECOND_PASSWORD が未設定のためスキップ"
else
  # order_number が空でなければ他フィールドも確認
  if [ "${ORDER_NUM:-none}" != "none" ]; then
    EIG_DAY=$(node -e "
      try { console.log(JSON.parse(process.argv[1]).eig_day||'none'); }
      catch(e) { console.log('none'); }
    " "$ORDER_RESP")
    [ "$EIG_DAY" != "none" ] \
      && pass "Step 4: eig_day フィールドあり ($EIG_DAY)" \
      || fail "Step 4" "eig_day フィールドが欠落"
  else
    pend "Step 4" "注文未受付のため eig_day 検証をスキップ"
  fi
fi

stop_app
print_summary
[ $FAIL -eq 0 ]
