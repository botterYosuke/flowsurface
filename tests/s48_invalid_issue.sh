#!/usr/bin/env bash
# s48_invalid_issue.sh — Phase 4-4: 存在しない銘柄コードの注文 E2E テスト
#
# 検証シナリオ:
#   1: Live モードで起動・デモセッション確立
#   2: 存在しない銘柄コード ("0000") で注文 → エラーレスポンス確認
#   3: order_number が返らないことを確認
#   4: エラーコードを抽出してログ（将来の参照用）
#
# 前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD / DEV_SECOND_PASSWORD が設定済み
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

echo "=== Phase 4-4: 存在しない銘柄コード E2E テスト ==="
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

# ── Step 2: 存在しない銘柄コードで注文 ─────────────────────────────────────────
echo ""
echo "── Step 2: 銘柄コード '0000' で成行買い注文"

ORDER_RESP=$(curl -s -X POST "$API/tachibana/order" \
  -H "Content-Type: application/json" \
  -d '{
    "issue_code":   "0000",
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

# ── Step 3: order_number が返らないことを確認 ──────────────────────────────────
echo ""
echo "── Step 3: 無効銘柄コードで order_number が返らないことを確認"

ORDER_NUM=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).order_number||'none'); }
  catch(e) { console.log('none'); }
" "$ORDER_RESP")

if [ "$ORDER_NUM" != "none" ] && [ -n "$ORDER_NUM" ]; then
  fail "Step 3" "無効銘柄コードなのに order_number が返った: $ORDER_RESP"
else
  pass "Step 3: 無効銘柄コードで order_number なし（期待どおり）"
fi

# ── Step 4: エラーコードを抽出してログ ────────────────────────────────────────
echo ""
echo "── Step 4: エラーコード抽出"

HAS_ERROR=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).error?'true':'false'); }
  catch(e) { console.log('false'); }
" "$ORDER_RESP")

IS_VALID_JSON=$(node -e "
  try { JSON.parse(process.argv[1]); console.log('true'); }
  catch(e) { console.log('false'); }
" "$ORDER_RESP")

[ "$IS_VALID_JSON" = "true" ] \
  || fail "Step 4a" "レスポンスが JSON でない（クラッシュの可能性）: $ORDER_RESP"

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
  fail "Step 4" "無効銘柄コードに対して error フィールドが返らなかった: $ORDER_RESP"
fi

stop_app
print_summary
[ $FAIL -eq 0 ]
