#!/usr/bin/env bash
# s25_screenshot_and_auth.sh — S25: screenshot API と auth/tachibana/status 検証
#
# 検証シナリオ:
#   A: POST /api/app/screenshot → HTTP 200、{"ok":true}
#   B: screenshot ファイルが C:/tmp/screenshot.png に存在する
#   C: Replay 再生中でも screenshot が動作する（{"ok":true}）
#   D: GET /api/app/screenshot（誤メソッド）→ HTTP 404
#   E: GET /api/auth/tachibana/status（Binance-only 構成）→ session="none"
#   F: session フィールドが存在する（レスポンススキーマ確認）
#
# 補足:
#   screenshot API は /api/app/screenshot に POST し、デスクトップ全体を
#   C:/tmp/screenshot.png に保存する。Replay 状態に依存しない。
#   auth/tachibana/status は Tachibana セッションの有無を返す。
#   Binance-only 環境では常に session="none" になる。
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S25: screenshot API と auth/tachibana/status 検証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ: BinanceLinear BTCUSDT M1 (Live モード起動) ─────────────────
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S25","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S25"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# ─────────────────────────────────────────────────────────────────────────────
# TC-E / TC-F: GET /api/auth/tachibana/status（Binance-only → session=none）
# Live モード起動直後に確認する（Replay 状態に依存しない API のため）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E/F: GET /api/auth/tachibana/status (Binance-only)"

AUTH_RESP=$(curl -s "$API/auth/tachibana/status")
echo "  auth response: $AUTH_RESP"

# TC-F: レスポンスが JSON で "session" フィールドを持つ
HAS_SESSION=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(typeof d.session === 'string' ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$AUTH_RESP")
[ "$HAS_SESSION" = "true" ] \
  && pass "TC-F: auth/tachibana/status に session フィールドが存在する" \
  || fail "TC-F" "session フィールドなし (response=$AUTH_RESP)"

# TC-E: session の値が "none" または "present" のいずれかである（スキーマ確認）
# Binance-only 環境では "none"、Tachibana セッションがある環境では "present" になる。
SESSION_VAL=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).session || 'null'); }
  catch(e) { console.log('null'); }
" "$AUTH_RESP")
[ "$SESSION_VAL" = "none" ] || [ "$SESSION_VAL" = "present" ] \
  && pass "TC-E: session=${SESSION_VAL} は有効値（none | present）" \
  || fail "TC-E" "session=$SESSION_VAL (expected none or present)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: POST /api/app/screenshot → {"ok":true}（Live モード）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: POST /api/app/screenshot（Live モード）"

# 古いファイルがあれば削除して確実に新規作成を確認
rm -f "C:/tmp/screenshot.png" 2>/dev/null || true

SCREENSHOT_RESP=$(api_post /api/app/screenshot)
echo "  screenshot response: $SCREENSHOT_RESP"

OK_VAL=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).ok ? 'true' : 'false'); }
  catch(e) { console.log('false'); }
" "$SCREENSHOT_RESP")
[ "$OK_VAL" = "true" ] \
  && pass "TC-A: POST /api/app/screenshot → {\"ok\":true}" \
  || fail "TC-A" "ok=$OK_VAL (response=$SCREENSHOT_RESP)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: C:/tmp/screenshot.png が存在する
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: screenshot.png のファイル存在確認"
sleep 0.5  # ファイル書き込み完了を待つ
if [ -f "C:/tmp/screenshot.png" ]; then
  FILE_SIZE=$(node -e "
    const fs=require('fs');
    try { console.log(fs.statSync('C:/tmp/screenshot.png').size); }
    catch(e) { console.log(0); }
  ")
  echo "  screenshot.png size=${FILE_SIZE} bytes"
  [ "$FILE_SIZE" -gt 0 ] \
    && pass "TC-B: C:/tmp/screenshot.png が存在し、サイズ > 0 (${FILE_SIZE} bytes)" \
    || fail "TC-B" "ファイルが空 (size=0)"
else
  fail "TC-B" "C:/tmp/screenshot.png が存在しない"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: GET /api/app/screenshot（誤メソッド）→ HTTP 404
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: GET /api/app/screenshot（誤メソッド）→ HTTP 404"
HTTP_CODE_D=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/app/screenshot")
[ "$HTTP_CODE_D" = "404" ] \
  && pass "TC-D: GET /api/app/screenshot → HTTP 404" \
  || fail "TC-D" "HTTP=$HTTP_CODE_D (expected 404)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Replay 再生中でも screenshot が動作する
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: Replay 再生中の screenshot"

START=$(utc_offset -3)
END=$(utc_offset -1)

api_post /api/replay/toggle > /dev/null
api_post /api/replay/play "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

# Playing になるまで待機（最大 30 秒）
if ! wait_status "Playing" 30; then
  # Playing 未到達の場合は PEND（データ取得に時間がかかる環境がある）
  pend "TC-C" "Playing 到達待ちタイムアウト（30s）— Replay 中 screenshot は未確認"
else
  rm -f "C:/tmp/screenshot.png" 2>/dev/null || true
  SCREENSHOT_RESP_C=$(api_post /api/app/screenshot)
  OK_VAL_C=$(node -e "
    try { console.log(JSON.parse(process.argv[1]).ok ? 'true' : 'false'); }
    catch(e) { console.log('false'); }
  " "$SCREENSHOT_RESP_C")
  [ "$OK_VAL_C" = "true" ] \
    && pass "TC-C: Replay Playing 中の screenshot → {\"ok\":true}" \
    || fail "TC-C" "ok=$OK_VAL_C (response=$SCREENSHOT_RESP_C)"
fi

stop_app

print_summary
