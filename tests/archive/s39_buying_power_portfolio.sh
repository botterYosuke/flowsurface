#!/usr/bin/env bash
# s39_buying_power_portfolio.sh — S39: BuyingPower パネル × portfolio.cash 整合性テスト
#
# 検証シナリオ:
#   A-B: BuyingPower パネルを開く → ペイン数 2、type = "Buying Power"
#   C:   初期 portfolio.cash = 1000000
#   D-E: Paused で成行買い × 3件 place → HTTP 200
#   F:   Paused 中 portfolio.cash 変化なし（約定しないため）
#   G:   Paused 中 open_positions 空
#   H:   BuyingPower パネル共存中のエラー通知 0 件
#   I-J: Live → Replay 遷移でエンジンリセット → cash = 1000000 に戻る
#   K:   リセット後 open_positions 空
#
# 仕様根拠:
#   docs/plan/e2e_order_panels_replay.md §S39
#   docs/order_windows.md §仮想約定エンジン §既知制限（Trades EventStore 未統合）
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S39: BuyingPower × portfolio.cash 整合性テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "$(primary_ticker)" "M1" "$START" "$END"
echo "  fixture: $(primary_ticker) M1, replay $START → $END"

start_app

if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "REPLAY Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
echo "  REPLAY Playing 到達"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A〜B: BuyingPower パネルを開く
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A〜B: BuyingPower パネルを開く"

PANES_INIT=$(curl -s "$API/pane/list")
PANE0=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  console.log(ps[0] ? ps[0].id : '');
" "$PANES_INIT")

api_post /api/sidebar/open-order-pane '{"kind":"BuyingPower"}' > /dev/null

if wait_for_pane_count 2 15; then
  pass "TC-A: BuyingPower パネルを開く → ペイン数 2"
else
  ACTUAL=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-A" "15s 以内にペイン数 2 にならなかった (actual=$ACTUAL)"
  print_summary; exit 1
fi

PANES_A=$(curl -s "$API/pane/list")
BP_TYPE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? (p.type || 'null') : 'not_found');
" "$PANES_A")
echo "  BuyingPower pane type=$BP_TYPE"
[ "$BP_TYPE" = "Buying Power" ] \
  && pass "TC-B: 新ペイン type = \"Buying Power\"" \
  || fail "TC-B" "type=\"$BP_TYPE\" (expected \"Buying Power\")"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Paused にして初期 portfolio.cash を確認
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: 初期 portfolio.cash 確認"

api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

PORTFOLIO_INIT=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio: $PORTFOLIO_INIT"

CASH_INIT=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_INIT")
[ "$CASH_INIT" = "1000000" ] \
  && pass "TC-C: 初期 portfolio.cash = 1000000" \
  || fail "TC-C" "cash=$CASH_INIT (expected 1000000)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D〜E: Paused で成行買い × 3件
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D〜E: Paused で成行買い × 3件 place"

CODE_1=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}')
CODE_2=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.2,"order_type":"market"}')
CODE_3=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.05,"order_type":"market"}')

[ "$CODE_1" = "200" ] && [ "$CODE_2" = "200" ] && [ "$CODE_3" = "200" ] \
  && pass "TC-D: 成行買い 3件 → すべて HTTP 200" \
  || fail "TC-D" "HTTP コード: $CODE_1 / $CODE_2 / $CODE_3 (expected 200/200/200)"

# Paused のまま 1s 待機（tick は来ない）
sleep 1

PORTFOLIO_AFTER=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio after orders: $PORTFOLIO_AFTER"

RESP_3=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.03,"order_type":"market"}')
STATUS_3=$(node -e "
  try { console.log(JSON.parse(process.argv[1]).status || 'null'); }
  catch(e) { console.log('null'); }
" "$RESP_3")
[ "$STATUS_3" = "pending" ] \
  && pass "TC-E: 成行買い status = \"pending\"" \
  || fail "TC-E" "status=$STATUS_3 (expected pending)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-F〜G: Paused 中 cash 不変・open_positions 空
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F〜G: Paused 中 portfolio 確認"

CASH_AFTER=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_AFTER")
[ "$CASH_AFTER" = "1000000" ] \
  && pass "TC-F: Paused 中 cash 不変 (cash=$CASH_AFTER)" \
  || fail "TC-F" "cash=$CASH_AFTER (expected 1000000 — Paused なので約定しないはず)"

OPEN_AFTER=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_AFTER")
[ "$OPEN_AFTER" = "0" ] \
  && pass "TC-G: Paused 中 open_positions 空 (length=0)" \
  || fail "TC-G" "open_positions.length=$OPEN_AFTER (expected 0)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-H: BuyingPower パネル共存中のエラー通知 0 件
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-H: エラー通知なし確認"

NOTIFS=$(curl -s "$API/notification/list")
ERR_COUNT=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.filter(n => n.level === 'error').length);
" "$NOTIFS")
[ "$ERR_COUNT" = "0" ] \
  && pass "TC-H: BuyingPower パネル共存中のエラー通知 0 件" \
  || fail "TC-H" "エラー通知 $ERR_COUNT 件発生"

# ─────────────────────────────────────────────────────────────────────────────
# TC-I〜K: Live → Replay 再遷移でエンジンリセット → cash = 1000000
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-I〜K: Live → Replay 再遷移でリセット確認"

api_post /api/replay/toggle > /dev/null  # → Live
LIVE_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
echo "  toggle 後のモード: $LIVE_MODE"

api_post /api/replay/toggle > /dev/null  # → Replay (engine.reset() が呼ばれる)
REPLAY_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
echo "  再 toggle 後のモード: $REPLAY_MODE"

[ "$REPLAY_MODE" = "Replay" ] \
  && pass "TC-I: Live → Replay 再遷移成功 (mode=$REPLAY_MODE)" \
  || fail "TC-I" "mode=$REPLAY_MODE (expected Replay)"

PORTFOLIO_RESET=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio after reset: $PORTFOLIO_RESET"

CASH_RESET=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_RESET")
[ "$CASH_RESET" = "1000000" ] \
  && pass "TC-J: リセット後 cash = 1000000" \
  || fail "TC-J" "cash=$CASH_RESET (expected 1000000)"

OPEN_RESET=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_RESET")
[ "$OPEN_RESET" = "0" ] \
  && pass "TC-K: リセット後 open_positions 空 (length=0)" \
  || fail "TC-K" "open_positions.length=$OPEN_RESET (expected 0)"

stop_app
print_summary
