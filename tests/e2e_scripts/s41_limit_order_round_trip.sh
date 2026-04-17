#!/usr/bin/env bash
# s41_limit_order_round_trip.sh — S41: 指値注文ラウンドトリップ
#
# 検証シナリオ:
#   A-B: Playing 到達 → Pause
#   C-H: 指値買い @999,999,999（必ず約定）→ step-forward → cash 減算 → 指値売り @1 → クローズ
#   I-K: 指値買い @1（絶対約定しない）→ step-forward × 3 → pending のまま残ることを確認
#
# 指値価格のトリック（order_book.rs の約定ロジック):
#   Long 指値: trade_price <= limit → @999,999,999 なら任意の BTCUSDT 価格で約定
#   Short 指値: trade_price >= limit → @1 なら任意の BTCUSDT 価格で約定
#   Long 指値: trade_price <= limit → @1 なら BTCUSDT 実価格（数万〜数十万）では不成立
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S41: 指値注文ラウンドトリップ ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S41","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S41"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — 自動ログインが無効です"
  exit 0
fi

start_app

echo "  Tachibana セッション確立待ち（最大 60 秒）..."
if ! wait_tachibana_session 60; then
  diagnose_playing_failure
  fail "precond" "Tachibana セッションが確立されなかった（DEV_USER_ID でのログインに失敗）"
  print_summary; exit 1
fi
echo "  Tachibana セッション確立"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: REPLAY Playing 到達
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: REPLAY Playing 到達"

if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "TC-A" "auto-play で Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
pass "TC-A: REPLAY Playing 到達"

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Pause
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: Pause"

api_post /api/replay/pause > /dev/null
if ! wait_status "Paused" 10; then
  fail "TC-B" "Pause 遷移せず（10s タイムアウト）"
  print_summary; exit 1
fi
pass "TC-B: Pause 遷移（step-forward 有効化）"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C〜H: 指値買い @999,999,999 → 約定 → cash 確認 → 指値売り @1 → クローズ
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C〜H: 指値ラウンドトリップ（必ず約定するトリック価格）"

BUY_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":1.0,"order_type":{"limit":9999999.0}}')
echo "  limit buy response: $BUY_RESP"
BUY_ID=$(jqn "$BUY_RESP" "d.order_id")
[ "$BUY_ID" != "null" ] && [ -n "$BUY_ID" ] \
  && pass "TC-C: 指値買い @9,999,999 → order_id=$BUY_ID" \
  || fail "TC-C" "order_id が返らない (resp=$BUY_RESP)"

OPEN=0
for i in $(seq 1 10); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.3
  PORTFOLIO=$(api_get /api/replay/portfolio)
  OPEN=$(jqn "$PORTFOLIO" "d.open_positions.length")
  echo "  step $i: open_positions=$OPEN"
  [ "$OPEN" -ge 1 ] && break || true
done
[ "$OPEN" -ge 1 ] \
  && pass "TC-D: 指値買い約定 → open_positions=$OPEN (A-2 指値パス)" \
  || fail "TC-D" "10 回 step-forward しても open_positions が増えない (=$OPEN)"

CASH=$(jqn "$PORTFOLIO" "d.cash")
echo "  cash after limit buy: $CASH"
node -e "process.exit(parseFloat('$CASH') < 1000000 ? 0 : 1)" 2>/dev/null \
  && pass "TC-E: cash < 1,000,000 (=$CASH) — 指値 fill でも cash deduct される (A-1)" \
  || fail "TC-E" "cash=$CASH (expected < 1000000)"

SELL_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":1.0,"order_type":{"limit":1.0}}')
echo "  limit sell response: $SELL_RESP"
SELL_ID=$(jqn "$SELL_RESP" "d.order_id")
[ "$SELL_ID" != "null" ] && [ -n "$SELL_ID" ] \
  && pass "TC-F: 指値売り @1 → order_id=$SELL_ID" \
  || fail "TC-F" "order_id が返らない (resp=$SELL_RESP)"

OPEN=1
for i in $(seq 1 10); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.3
  PORTFOLIO=$(api_get /api/replay/portfolio)
  OPEN=$(jqn "$PORTFOLIO" "d.open_positions.length")
  echo "  step $i: open_positions=$OPEN"
  [ "$OPEN" -eq 0 ] && break || true
done
[ "$OPEN" -eq 0 ] \
  && pass "TC-G: 指値売り → Long クローズ → open_positions=0 (A-2 指値クローズ)" \
  || fail "TC-G" "10 回 step-forward しても Long がクローズされない (=$OPEN)"

REALIZED=$(jqn "$PORTFOLIO" "d.realized_pnl")
CASH=$(jqn "$PORTFOLIO" "d.cash")
CASH_OK=$(node -e "
  const diff = Math.abs(parseFloat('$CASH') - 1000000 - parseFloat('$REALIZED'));
  console.log(diff < 1.0 ? 'true' : 'false');
")
[ "$CASH_OK" = "true" ] \
  && pass "TC-H: cash ($CASH) = 1,000,000 + realized_pnl ($REALIZED) — 指値 close の A-0 パス確認" \
  || fail "TC-H" "cash=$CASH, realized=$REALIZED — cash=initial+realized が成立しない"

# ─────────────────────────────────────────────────────────────────────────────
# TC-I〜K: 絶対約定しない指値 buy @1 → pending のまま残ることを確認
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-I〜K: 未達指値（buy @1）→ pending 維持"

UNMATCHED_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":{"limit":1.0}}')
echo "  unmatched limit buy response: $UNMATCHED_RESP"
UNMATCHED_ID=$(jqn "$UNMATCHED_RESP" "d.order_id")
[ "$UNMATCHED_ID" != "null" ] && [ -n "$UNMATCHED_ID" ] \
  && pass "TC-I: 未達指値注文 → order_id=$UNMATCHED_ID (pending に追加)" \
  || fail "TC-I" "order_id が返らない (resp=$UNMATCHED_RESP)"

for i in $(seq 1 3); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.3
done

PORTFOLIO=$(api_get /api/replay/portfolio)
OPEN=$(jqn "$PORTFOLIO" "d.open_positions.length")
[ "$OPEN" -eq 0 ] \
  && pass "TC-J: step-forward × 3 後も open_positions=0 — 未達指値は約定しない" \
  || fail "TC-J" "open_positions=$OPEN (expected 0 — 指値 @1 は約定しないはず)"

ORDERS=$(api_get /api/replay/orders)
echo "  orders: $ORDERS"
PENDING_COUNT=$(jqn "$ORDERS" "(d.orders||[]).length")
[ "$PENDING_COUNT" -ge 1 ] \
  && pass "TC-K: GET /api/replay/orders → orders.length=$PENDING_COUNT — pending 残存確認" \
  || fail "TC-K" "orders.length=$PENDING_COUNT (expected >= 1 — 未達指値が pending に残るはず)"

stop_app
print_summary
