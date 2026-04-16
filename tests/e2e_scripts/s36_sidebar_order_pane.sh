#!/usr/bin/env bash
# s36_sidebar_order_pane.sh — S36: /api/sidebar/open-order-pane による注文ペイン分割テスト
#
# シナリオ:
#   BinanceLinear:BTCUSDT M1 の単一ペインで起動後、
#   POST /api/sidebar/open-order-pane で OrderEntry / OrderList / BuyingPower を
#   順に開き、各ペインが正しく作成されることを検証する。
#
#   TC-A: {"kind":"OrderEntry"} → ペイン数 2、新ペインの type = "Order Entry"
#   TC-B: {"kind":"OrderList"}  → ペイン数 3、新ペインの type = "Order List"
#   TC-C: {"kind":"BuyingPower"} → ペイン数 4、新ペインの type = "Buying Power"
#   TC-D: エラー通知 0 件
#   TC-E: 元ペイン (PANE0) の type が変わっていない（"Candlestick Chart" のまま）
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S36: sidebar/open-order-pane による注文ペイン分割テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ: 単一ペイン BinanceLinear:BTCUSDT M1 ────────────────────────
START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
echo "  fixture: BTCUSDT M1, replay $START → $END"

# ── アプリ起動 ────────────────────────────────────────────────────────────────
start_app

# autoplay で Playing に到達するまで待機
if ! wait_status "Playing" 60; then
  fail "S36-precond" "Playing 到達せず（timeout）"
  print_summary
  exit 1
fi

# ── 初期ペイン ID 取得 ──────────────────────────────────────────────────────
PANES=$(curl -s "$API/pane/list")
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE0" ]; then
  fail "S36-precond" "初期ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE0=$PANE0"

# ── TC-A: OrderEntry → ペイン数 2 ─────────────────────────────────────────
echo ""
echo "── TC-A: OrderEntry → ペイン数 2"
api_post /api/sidebar/open-order-pane '{"kind":"OrderEntry"}' > /dev/null

if wait_for_pane_count 2 15; then
  pass "TC-A: OrderEntry → ペイン数 2"
else
  ACTUAL_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-A" "15 秒以内に pane count が 2 にならなかった (actual=$ACTUAL_COUNT)"
  print_summary
  exit 1
fi

# 新ペインの type 確認
PANES_A=$(curl -s "$API/pane/list")
PANE_A_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_A")
echo "  new pane type=$PANE_A_TYPE"
if [ "$PANE_A_TYPE" = "Order Entry" ]; then
  pass "TC-A: 新ペイン type = \"Order Entry\""
else
  fail "TC-A" "新ペイン type=\"$PANE_A_TYPE\" (expected \"Order Entry\")"
fi

# ── TC-B: OrderList → ペイン数 3 ──────────────────────────────────────────
echo ""
echo "── TC-B: OrderList → ペイン数 3"
api_post /api/sidebar/open-order-pane '{"kind":"OrderList"}' > /dev/null

if wait_for_pane_count 3 15; then
  pass "TC-B: OrderList → ペイン数 3"
else
  ACTUAL_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-B" "15 秒以内に pane count が 3 にならなかった (actual=$ACTUAL_COUNT)"
  print_summary
  exit 1
fi

# 新ペインの type 確認（PANE0 でも PANE_A でもない最新ペイン）
PANES_B=$(curl -s "$API/pane/list")
PANE_A_ID=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? p.id : '');
" "$PANES_A")
PANE_B_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0' && x.id !== '$PANE_A_ID');
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_B")
echo "  new pane type=$PANE_B_TYPE"
if [ "$PANE_B_TYPE" = "Order List" ]; then
  pass "TC-B: 新ペイン type = \"Order List\""
else
  fail "TC-B" "新ペイン type=\"$PANE_B_TYPE\" (expected \"Order List\")"
fi

# ── TC-C: BuyingPower → ペイン数 4 ────────────────────────────────────────
echo ""
echo "── TC-C: BuyingPower → ペイン数 4"
api_post /api/sidebar/open-order-pane '{"kind":"BuyingPower"}' > /dev/null

if wait_for_pane_count 4 15; then
  pass "TC-C: BuyingPower → ペイン数 4"
else
  ACTUAL_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-C" "15 秒以内に pane count が 4 にならなかった (actual=$ACTUAL_COUNT)"
  print_summary
  exit 1
fi

# 新ペインの type 確認
PANES_C=$(curl -s "$API/pane/list")
PANE_B_ID=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0' && x.id !== '$PANE_A_ID');
  console.log(p ? p.id : '');
" "$PANES_B")
PANE_C_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const known = ['$PANE0', '$PANE_A_ID', '$PANE_B_ID'];
  const p = ps.find(x => !known.includes(x.id));
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_C")
echo "  new pane type=$PANE_C_TYPE"
if [ "$PANE_C_TYPE" = "Buying Power" ]; then
  pass "TC-C: 新ペイン type = \"Buying Power\""
else
  fail "TC-C" "新ペイン type=\"$PANE_C_TYPE\" (expected \"Buying Power\")"
fi

# ── TC-D: エラー通知が出ていない ─────────────────────────────────────────────
echo ""
echo "── TC-D: エラー通知なし確認"
NOTIFS=$(curl -s "$API/notification/list")
ERROR_COUNT=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.filter(n => n.level === 'error').length);
" "$NOTIFS")
echo "  error notification count=$ERROR_COUNT"
[ "$ERROR_COUNT" = "0" ] \
  && pass "TC-D: エラー通知 0 件" \
  || fail "TC-D" "エラー通知が $ERROR_COUNT 件発生"

# ── TC-E: 元ペインの type が変わっていない ─────────────────────────────────
echo ""
echo "── TC-E: 元ペイン (PANE0) の type 確認"
ORIG_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$PANE0');
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_C")
echo "  orig pane type=$ORIG_TYPE"
if [ "$ORIG_TYPE" = "Candlestick Chart" ]; then
  pass "TC-E: 元ペイン type = \"Candlestick Chart\" のまま"
else
  fail "TC-E" "元ペイン type=\"$ORIG_TYPE\" (expected \"Candlestick Chart\" — 上書きされている可能性)"
fi

print_summary
[ $FAIL -eq 0 ]
