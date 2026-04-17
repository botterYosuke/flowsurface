#!/usr/bin/env bash
# s46_wrong_password.sh — Phase 4-2: 発注パスワード誤り E2E テスト
#
# 検証シナリオ:
#   1: Live モードで起動・デモセッション確立
#   2: 誤った発注パスワードで新規注文 → エラーレスポンス確認
#   3: レスポンスに order_number が含まれないことを確認
#   4: エラーコードを抽出してログ（将来の参照用）
#
# 前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済み
# NOTE: 意図的に DEV_SECOND_PASSWORD に依存しない（誤パスワードを使用する）
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

[ "${DEV_IS_DEMO:-}" = "true" ] \
  || { echo "ERROR: DEV_IS_DEMO=true を設定してください（本番誤発注防止）"; exit 1; }
[ -n "${DEV_USER_ID:-}" ] \
  || { echo "ERROR: DEV_USER_ID が未設定です"; exit 1; }
[ -n "${DEV_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_PASSWORD が未設定です"; exit 1; }

echo "=== Phase 4-2: 発注パスワード誤り E2E テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

cat > "$DATA_DIR/saved-state.json" <<'EOF'
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

# ── Step 1: Live モード・セッション確認 ────────────────────────────────────────
echo ""
echo "── Step 1: Live モード・セッション確認"
STATUS=$(curl -s "$API/replay/status")
MODE=$(node -e "try{console.log(JSON.parse(process.argv[1]).mode||'null');}catch(e){console.log('null');}" "$STATUS")
[ "$MODE" = "Live" ] \
  && pass "Step 1a: Live モード確認 (mode=$MODE)" \
  || fail "Step 1a" "mode=$MODE (expected Live)"

wait_tachibana_session 120 \
  && pass "Step 1b: デモセッション確立" \
  || fail "Step 1b" "セッション未確立"

# ── Step 2: 誤パスワードで新規注文 ─────────────────────────────────────────────
echo ""
echo "── Step 2: 誤パスワード (WRONG_PASSWORD_PHASE4TEST) で TOYOTA 成行買い"

ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
  -H "Content-Type: application/json" \
  -d '{
    "issue_code":       "7203",
    "qty":              "100",
    "side":             "3",
    "price":            "0",
    "account_type":     "1",
    "market_code":      "00",
    "condition":        "0",
    "cash_margin":      "0",
    "expire_day":       "0",
    "second_password":  "WRONG_PASSWORD_PHASE4TEST"
  }')
echo "  response: $ORDER_RESP"

# ── Step 3: order_number が含まれないこと（誤パスワードで約定してはいけない）───
echo ""
echo "── Step 3: 誤パスワードで order_number が返らないことを確認"

HAS_ORDER_NUM=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(d.order_number && d.order_number !== '' ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$ORDER_RESP")

if [ "$HAS_ORDER_NUM" = "true" ]; then
  fail "Step 3" "誤パスワードなのに order_number が返った（セキュリティ上の問題）: $ORDER_RESP"
else
  pass "Step 3: 誤パスワードで order_number なし（期待どおり）"
fi

# ── Step 4: エラーコードを抽出してログ ────────────────────────────────────────
echo ""
echo "── Step 4: エラーコード抽出"

HAS_ERROR=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(d.error ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$ORDER_RESP")

if [ "$HAS_ERROR" = "true" ]; then
  ERR_MSG=$(node -e "
    try { console.log(JSON.parse(process.argv[1]).error||''); }
    catch(e) { console.log(''); }
  " "$ORDER_RESP")
  ERR_CODE=$(node -e "
    const m = process.argv[1].match(/code=([^,}]+)/);
    console.log(m ? m[1].trim() : 'unknown');
  " "$ERR_MSG")
  echo "  エラーコード: $ERR_CODE"
  echo "  エラーメッセージ: $ERR_MSG"
  pass "Step 4: エラーレスポンス取得（code=$ERR_CODE）"
else
  fail "Step 4" "エラーフィールドなし: $ORDER_RESP"
fi

stop_app
print_summary
[ $FAIL -eq 0 ]
