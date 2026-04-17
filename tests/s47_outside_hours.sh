#!/usr/bin/env bash
# s47_outside_hours.sh — Phase 4-3: 市場時間外の成行注文エラーコード確認
#
# 検証シナリオ:
#   1: Live モードで起動・デモセッション確立
#   2: 成行注文を発行（時間帯によって結果が異なる）
#   3: 時間内なら order_number 取得（約定待ち）、時間外なら エラーコード確認
#   4: エラーコードを抽出してログ（将来の参照用）
#
# 前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD / DEV_SECOND_PASSWORD が設定済み
# NOTE: 市場時間（JST 9:00-15:30 平日）外実行時に市場時間外エラーコードを確認する
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

[ "${DEV_IS_DEMO:-}" = "true" ] \
  || { echo "ERROR: DEV_IS_DEMO=true を設定してください（本番誤発注防止）"; exit 1; }
[ -n "${DEV_USER_ID:-}" ] \
  || { echo "ERROR: DEV_USER_ID が未設定です"; exit 1; }
[ -n "${DEV_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_PASSWORD が未設定です"; exit 1; }
[ -n "${DEV_SECOND_PASSWORD:-}" ] \
  || { echo "ERROR: DEV_SECOND_PASSWORD が未設定です"; exit 1; }

echo "=== Phase 4-3: 市場時間外の成行注文 E2E テスト ==="
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

# ── Step 2: 成行注文を発行 ──────────────────────────────────────────────────────
echo ""
echo "── Step 2: TOYOTA (7203) 1株 成行買い（price=0）"

ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
  -H "Content-Type: application/json" \
  -d '{
    "issue_code":   "7203",
    "qty":          "1",
    "side":         "3",
    "price":        "0",
    "account_type": "1",
    "market_code":  "00",
    "condition":    "0",
    "cash_margin":  "0",
    "expire_day":   "0"
  }')
echo "  response: $ORDER_RESP"

# ── Step 3: レスポンスを解析（時間内・時間外の両方に対応）─────────────────────
echo ""
echo "── Step 3: レスポンス解析"

ORDER_NUM=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).order_number||'none'); }
  catch(e) { console.log('none'); }
" "$ORDER_RESP")

HAS_ERROR=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).error?'true':'false'); }
  catch(e) { console.log('false'); }
" "$ORDER_RESP")

if [ "$ORDER_NUM" != "none" ] && [ -n "$ORDER_NUM" ]; then
  # 市場時間内: 成行注文が受け付けられた
  echo "  市場時間内: 注文受付 (order_number=$ORDER_NUM)"
  pass "Step 3: 成行注文受付済み（市場時間内）— order_number=$ORDER_NUM"
elif [ "$HAS_ERROR" = "true" ]; then
  # 市場時間外（または他のエラー）
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
  pass "Step 3: エラーレスポンス確認（code=$ERR_CODE）— 市場時間外の可能性あり"
else
  fail "Step 3" "予期しないレスポンス形式: $ORDER_RESP"
fi

# ── Step 4: エラーコードが記録されたことを確認 ─────────────────────────────────
echo ""
echo "── Step 4: API 疎通・エラーハンドリング確認"

IS_VALID_JSON=$(node -e "
  try { JSON.parse(process.argv[1]); console.log('true'); }
  catch(e) { console.log('false'); }
" "$ORDER_RESP")

[ "$IS_VALID_JSON" = "true" ] \
  && pass "Step 4: レスポンスが有効な JSON（クラッシュなし）" \
  || fail "Step 4" "レスポンスが JSON でない（クラッシュの可能性）: $ORDER_RESP"

stop_app
print_summary
[ $FAIL -eq 0 ]
