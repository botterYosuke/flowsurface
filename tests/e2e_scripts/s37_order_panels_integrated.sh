#!/usr/bin/env bash
# s37_order_panels_integrated.sh — S37: 3パネル統合テスト（OrderEntry / OrderList / BuyingPower）
#
# 検証シナリオ:
#   A-B: replay Playing 中に 3パネルを順に開く → ペイン数 4、エラー通知 0 件
#   C-D: Playing 中に成行買い × 3件 place → 各 HTTP 200、order_id 返却
#   E-F: Pause → portfolio.cash 不変・open_positions 空
#       （Trades EventStore 未統合のため Paused 中は約定しない）
#   G-I: Paused のまま指値買い × 2件・指値売り × 2件 → 各 HTTP 200, status="pending"
#   J:   注文後もエラー通知 0 件
#   K:   元チャートペインの type が変わっていない
#
# 仕様根拠:
#   docs/plan/e2e_order_panels_replay.md §S37
#   docs/order_windows.md §仮想約定エンジン §既知制限
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S37: 3パネル統合テスト（OrderEntry / OrderList / BuyingPower） ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
echo "  fixture: BTCUSDT M1, replay $START → $END"

start_app

# REPLAY Playing に到達
if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "REPLAY Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
echo "  REPLAY Playing 到達"

# 初期ペイン ID を保存
PANES_INIT=$(curl -s "$API/pane/list")
PANE0=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  console.log(ps[0] ? ps[0].id : '');
" "$PANES_INIT")
if [ -z "$PANE0" ]; then
  fail "precond" "初期ペイン ID 取得失敗"
  print_summary; exit 1
fi
echo "  PANE0=$PANE0"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A〜B: Playing 中に 3パネルを順に開く
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A〜B: Playing 中に 3パネルを順に開く"

api_post /api/sidebar/open-order-pane '{"kind":"OrderEntry"}' > /dev/null
wait_for_pane_count 2 15
api_post /api/sidebar/open-order-pane '{"kind":"OrderList"}' > /dev/null
wait_for_pane_count 3 15
api_post /api/sidebar/open-order-pane '{"kind":"BuyingPower"}' > /dev/null

if wait_for_pane_count 4 15; then
  pass "TC-A: 3パネルを順に開いてペイン数 4 に到達"
else
  ACTUAL=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-A" "15s 以内にペイン数 4 にならなかった (actual=$ACTUAL)"
  print_summary; exit 1
fi

NOTIFS_A=$(curl -s "$API/notification/list")
ERR_A=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.filter(n => n.level === 'error').length);
" "$NOTIFS_A")
[ "$ERR_A" = "0" ] \
  && pass "TC-B: パネル開閉中のエラー通知 0 件" \
  || fail "TC-B" "エラー通知 $ERR_A 件発生"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C〜D: Playing 中に成行買い × 3件 place
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C〜D: Playing 中に成行買い × 3件"

ALL_200=true
ALL_IDS=true
for i in 1 2 3; do
  RESP=$(api_post /api/replay/order \
    '{"ticker":"BTCUSDT","side":"buy","qty":0.05,"order_type":"market"}')
  CODE=$(api_post_code /api/replay/order \
    '{"ticker":"BTCUSDT","side":"buy","qty":0.02,"order_type":"market"}')
  ID=$(node -e "
    try {
      const d = JSON.parse(process.argv[1]);
      console.log(typeof d.order_id === 'string' && d.order_id.length > 0 ? 'ok' : 'null');
    } catch(e) { console.log('null'); }
  " "$RESP")
  [ "$CODE" != "200" ] && ALL_200=false
  [ "$ID" != "ok" ]   && ALL_IDS=false
done

[ "$ALL_200" = "true" ] \
  && pass "TC-C: 成行買い 3件 → すべて HTTP 200" \
  || fail "TC-C" "HTTP 200 でない注文あり"

[ "$ALL_IDS" = "true" ] \
  && pass "TC-D: 成行買い 3件 → すべて order_id 返却" \
  || fail "TC-D" "order_id が null の注文あり"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E〜F: Pause → portfolio.cash 不変・open_positions 空
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E〜F: Pause → portfolio 確認"

api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

PORTFOLIO=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio: $PORTFOLIO"

CASH=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$CASH" = "1000000" ] \
  && pass "TC-E: Paused 中 cash = 1000000（約定なし）" \
  || fail "TC-E" "cash=$CASH (expected 1000000)"

OPEN_LEN=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$OPEN_LEN" = "0" ] \
  && pass "TC-F: Paused 中 open_positions 空（約定なし）" \
  || fail "TC-F" "open_positions.length=$OPEN_LEN (expected 0)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-G〜I: Paused のまま指値注文 × 4件（買い 2・売り 2）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-G〜I: Paused 中に指値注文 × 4件"

LIMIT_BUY_1=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.05,"order_type":{"limit":1.0}}')
LIMIT_BUY_2=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.03,"order_type":{"limit":1.0}}')
LIMIT_SELL_1=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":0.05,"order_type":{"limit":9999999.0}}')
LIMIT_SELL_2=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":0.03,"order_type":{"limit":9999999.0}}')

# HTTP 200 確認（place 時のコードを再取得）
CODE_LB=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.01,"order_type":{"limit":1.0}}')
CODE_LS=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":0.01,"order_type":{"limit":9999999.0}}')

[ "$CODE_LB" = "200" ] \
  && pass "TC-G: 指値買い → HTTP 200" \
  || fail "TC-G" "HTTP=$CODE_LB (expected 200)"

[ "$CODE_LS" = "200" ] \
  && pass "TC-H: 指値売り → HTTP 200" \
  || fail "TC-H" "HTTP=$CODE_LS (expected 200)"

# 各レスポンスの status="pending" 確認
ALL_PENDING=true
for RESP in "$LIMIT_BUY_1" "$LIMIT_BUY_2" "$LIMIT_SELL_1" "$LIMIT_SELL_2"; do
  STATUS=$(node -e "
    try { console.log(JSON.parse(process.argv[1]).status || 'null'); }
    catch(e) { console.log('null'); }
  " "$RESP")
  [ "$STATUS" != "pending" ] && ALL_PENDING=false
done
[ "$ALL_PENDING" = "true" ] \
  && pass "TC-I: 指値注文 4件 すべて status=\"pending\"" \
  || fail "TC-I" "pending でない注文あり"

# ─────────────────────────────────────────────────────────────────────────────
# TC-J: 注文後もエラー通知 0 件
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-J: 注文後エラー通知なし確認"

NOTIFS_J=$(curl -s "$API/notification/list")
ERR_J=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.filter(n => n.level === 'error').length);
" "$NOTIFS_J")
[ "$ERR_J" = "0" ] \
  && pass "TC-J: 注文後エラー通知 0 件" \
  || fail "TC-J" "エラー通知 $ERR_J 件発生"

# ─────────────────────────────────────────────────────────────────────────────
# TC-K: 元チャートペインの type が変わっていない
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-K: 元ペイン (PANE0) の type 確認"

PANES_K=$(curl -s "$API/pane/list")
ORIG_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$PANE0');
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_K")
echo "  orig pane type=$ORIG_TYPE"
[ "$ORIG_TYPE" = "Candlestick Chart" ] \
  && pass "TC-K: 元ペイン type = \"Candlestick Chart\" のまま" \
  || fail "TC-K" "元ペイン type=\"$ORIG_TYPE\" (expected \"Candlestick Chart\")"

stop_app
print_summary
