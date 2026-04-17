#!/usr/bin/env bash
# s42_naked_short_cycle.sh — S42: 裸ショートフルサイクル
#
# 検証シナリオ:
#   A-B: Playing 到達 → Pause
#   C:   Long ポジションなしを確認（裸ショートの前提）
#   D-F: 成行売り（Long なし）→ step-forward → Short open → cash 増加 (A-1/A-2)
#   G-K: 成行買い → step-forward → Short クローズ → PnL 確定 (A-0/A-2 対称拡張)
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S42: 裸ショートフルサイクル ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

setup_single_pane "$E2E_TICKER" "M1" "$START" "$END"

if ! is_headless; then
  if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
    echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — 自動ログインが無効です"
    exit 0
  fi
fi

start_app
headless_play

if ! is_headless; then
  echo "  Tachibana セッション確立待ち（最大 60 秒）..."
  if ! wait_tachibana_session 60; then
    diagnose_playing_failure
    fail "precond" "Tachibana セッションが確立されなかった（DEV_USER_ID でのログインに失敗）"
    print_summary; exit 1
  fi
  echo "  Tachibana セッション確立"
fi

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
# TC-C: Long ポジションなしを確認（裸ショートの前提）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: 裸ショート前提確認（open_positions == 0）"

PORTFOLIO=$(api_get /api/replay/portfolio)
OPEN=$(jqn "$PORTFOLIO" "d.open_positions.length")
[ "$OPEN" -eq 0 ] \
  && pass "TC-C: open_positions=0 — 裸ショートの前提を満たす" \
  || fail "TC-C" "open_positions=$OPEN (expected 0 — Long ポジションがあると裸ショートにならない)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D〜F: 成行売り（Long なし）→ step-forward → Short open → cash 増加
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D〜F: 裸ショート open → cash 増加 (A-1/A-2)"

SHORT_RESP=$(api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"sell\",\"qty\":1.0,\"order_type\":\"market\"}")
echo "  naked short response: $SHORT_RESP"
SHORT_ID=$(jqn "$SHORT_RESP" "d.order_id")
[ "$SHORT_ID" != "null" ] && [ -n "$SHORT_ID" ] \
  && pass "TC-D: 成行売り（Long なし）→ order_id=$SHORT_ID" \
  || fail "TC-D" "order_id が返らない (resp=$SHORT_RESP)"

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
  && pass "TC-D-check: step-forward で Short open → open_positions=$OPEN (A-2: 裸ショート)" \
  || fail "TC-D-check" "10 回 step-forward しても open_positions が増えない (=$OPEN)"

SIDE=$(jqn "$PORTFOLIO" "d.open_positions[0].side")
echo "  open_positions[0].side: $SIDE"
[ "$SIDE" = "Short" ] \
  && pass "TC-E: open_positions[0].side=Short — 裸ショートが Short として記録 (A-2)" \
  || fail "TC-E" "side=$SIDE (expected Short)"

CASH=$(jqn "$PORTFOLIO" "d.cash")
echo "  cash after short open: $CASH"
node -e "process.exit(parseFloat('$CASH') > 1000000 ? 0 : 1)" 2>/dev/null \
  && pass "TC-F: cash > 1,000,000 (=$CASH) — Short open で売り代金を受け取った (A-1)" \
  || fail "TC-F" "cash=$CASH (expected > 1000000 — Short open では cash が増加するはず)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-G〜K: 成行買い → step-forward → Short クローズ → PnL 確定 (A-0/A-2 対称拡張)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-G〜K: 買い注文で Short クローズ → PnL 確定 (A-0/A-2 対称拡張)"

BUY_RESP=$(api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"buy\",\"qty\":1.0,\"order_type\":\"market\"}")
echo "  buy to close response: $BUY_RESP"
BUY_ID=$(jqn "$BUY_RESP" "d.order_id")
[ "$BUY_ID" != "null" ] && [ -n "$BUY_ID" ] \
  && pass "TC-G: 成行買い（Short クローズ用）→ order_id=$BUY_ID" \
  || fail "TC-G" "order_id が返らない (resp=$BUY_RESP)"

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
  && pass "TC-H: Short クローズ → open_positions=0 (A-2 対称拡張: buy closes Short)" \
  || fail "TC-H" "10 回 step-forward しても Short がクローズされない (=$OPEN)"

CLOSED=$(jqn "$PORTFOLIO" "d.closed_positions.length")
[ "$CLOSED" -eq 1 ] \
  && pass "TC-I: closed_positions.length=1 — Short の record_close() 呼び出し確認" \
  || fail "TC-I" "closed_positions.length=$CLOSED (expected 1)"

REALIZED=$(jqn "$PORTFOLIO" "d.realized_pnl")
echo "  realized_pnl: $REALIZED"
node -e "process.exit(parseFloat('$REALIZED') !== 0 ? 0 : 1)" 2>/dev/null \
  && pass "TC-J: realized_pnl != 0 (=$REALIZED) — Short の PnL 計算確認 (A-0)" \
  || fail "TC-J" "realized_pnl=0 (PnL が確定していない)"

CASH=$(jqn "$PORTFOLIO" "d.cash")
CASH_OK=$(node -e "
  const diff = Math.abs(parseFloat('$CASH') - 1000000 - parseFloat('$REALIZED'));
  console.log(diff < 1.0 ? 'true' : 'false');
")
[ "$CASH_OK" = "true" ] \
  && pass "TC-K: cash ($CASH) = 1,000,000 + realized_pnl ($REALIZED) — Short close の A-0 パス確認" \
  || fail "TC-K" "cash=$CASH, realized=$REALIZED — cash=initial+realized が成立しない"

stop_app
print_summary
