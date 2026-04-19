#!/usr/bin/env bash
# s40_virtual_order_fill_cycle.sh — S40: 仮想取引フルサイクル（成行ラウンドトリップ）
#
# 検証シナリオ:
#   A-B: Playing 到達 → Pause（step-forward は Paused 時のみ 1 bar 前進）
#   C-E: 成行買い → step-forward 約定 → cash 減算確認 (A-1: record_open)
#   F-K: 成行売り → step-forward Long クローズ → PnL/cash 確定 (A-0/A-2)
#
# 約定メカニズム:
#   step-forward が synthetic_trades_at_current_time() で kline close 価格の
#   合成トレードを生成し on_tick() へ渡す。成行注文は 1 回目で約定する。
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S40: 仮想取引フルサイクル（成行ラウンドトリップ）==="
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
# TC-B: Pause（step-forward は Paused 時のみ 1 bar 前進する。Playing 中は range 末尾へジャンプ）
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
# TC-C〜E: 成行買い → step-forward 約定 → cash 確認 (A-1)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C〜E: 成行買い → step-forward 約定 → cash 確認 (A-1)"

BUY_RESP=$(api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"buy\",\"qty\":1.0,\"order_type\":\"market\"}")
echo "  buy response: $BUY_RESP"
BUY_ID=$(jqn "$BUY_RESP" "d.order_id")
[ "$BUY_ID" != "null" ] && [ -n "$BUY_ID" ] \
  && pass "TC-C: 成行買い → order_id=$BUY_ID" \
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
  && pass "TC-D: step-forward で約定 → open_positions=$OPEN (A-2)" \
  || fail "TC-D" "10 回 step-forward しても open_positions が増えない (=$OPEN)"

CASH=$(jqn "$PORTFOLIO" "d.cash")
echo "  cash after buy: $CASH"
node -e "process.exit(parseFloat('$CASH') < 1000000 ? 0 : 1)" 2>/dev/null \
  && pass "TC-E: cash < 1,000,000 (=$CASH) — 購入コスト減算確認 (A-1)" \
  || fail "TC-E" "cash=$CASH (expected < 1000000)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-F〜K: 成行売り → step-forward Long クローズ → PnL 確定 (A-0/A-2)
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F〜K: 成行売り → Long クローズ → PnL 確定 (A-0/A-2)"

SELL_RESP=$(api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"sell\",\"qty\":1.0,\"order_type\":\"market\"}")
echo "  sell response: $SELL_RESP"
SELL_ID=$(jqn "$SELL_RESP" "d.order_id")
[ "$SELL_ID" != "null" ] && [ -n "$SELL_ID" ] \
  && pass "TC-F: 成行売り → order_id=$SELL_ID" \
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
  && pass "TC-G: Long クローズ → open_positions=0 (A-2)" \
  || fail "TC-G" "10 回 step-forward しても open_positions が 0 にならない (=$OPEN)"

CLOSED=$(jqn "$PORTFOLIO" "d.closed_positions.length")
[ "$CLOSED" -eq 1 ] \
  && pass "TC-H: closed_positions.length=1 — record_close() 呼び出し確認 (A-2)" \
  || fail "TC-H" "closed_positions.length=$CLOSED (expected 1)"

REALIZED=$(jqn "$PORTFOLIO" "d.realized_pnl")
echo "  realized_pnl: $REALIZED"
# PnL は (売値 - 買値) * qty。同価格約定なら 0 も正当な値。
# ここでは「数値として確定している」ことを確認する（TC-J が整合性を検証）。
node -e "
  const v = parseFloat('$REALIZED');
  process.exit(isFinite(v) ? 0 : 1);
" 2>/dev/null \
  && pass "TC-I: realized_pnl=$REALIZED (PnL が数値として確定)" \
  || fail "TC-I" "realized_pnl が数値でない (=$REALIZED)"

CASH=$(jqn "$PORTFOLIO" "d.cash")
CASH_OK=$(node -e "
  const diff = Math.abs(parseFloat('$CASH') - 1000000 - parseFloat('$REALIZED'));
  console.log(diff < 1.0 ? 'true' : 'false');
")
[ "$CASH_OK" = "true" ] \
  && pass "TC-J: cash ($CASH) = 1,000,000 + realized_pnl ($REALIZED) — 売却代金返還確認 (A-0)" \
  || fail "TC-J" "cash=$CASH, realized=$REALIZED — cash=initial+realized が成立しない"

UNREALIZED=$(jqn "$PORTFOLIO" "d.unrealized_pnl")
EQUITY=$(jqn "$PORTFOLIO" "d.total_equity")
EQUITY_OK=$(node -e "
  const diff = Math.abs(parseFloat('$EQUITY') - (parseFloat('$CASH') + parseFloat('$UNREALIZED')));
  console.log(diff < 1.0 ? 'true' : 'false');
")
[ "$EQUITY_OK" = "true" ] \
  && pass "TC-K: total_equity ($EQUITY) = cash ($CASH) + unrealized ($UNREALIZED)" \
  || fail "TC-K" "total_equity=$EQUITY ≠ cash+unrealized — スキーマ不整合"

stop_app
print_summary
