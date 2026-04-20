#!/usr/bin/env bash
# s35_virtual_portfolio.sh — S35: 仮想ポートフォリオのライフサイクル検証
#
# 検証シナリオ:
#   A-G: 初期ポートフォリオスナップショットのスキーマと値を検証
#        (cash=1000000, unrealized_pnl=0, realized_pnl=0,
#         total_equity=cash, open_positions=[], closed_positions=[])
#   H-I: 成行注文を 2 件 place 後もポートフォリオは変化しない
#        (現状 Trades EventStore 未統合のため約定なし → cash/positions 不変)
#   J:   PEND — StepBackward による仮想エンジンリセット（未実装）
#        docs/order_windows.md §未実装: "SeekBackward 時のエンジンリセット"
#   K-L: Live → Replay 遷移でエンジンが reset() される
#        (toggle → toggle 後に portfolio が初期値に戻ることを確認)
#
# 仕様根拠:
#   docs/replay_header.md §11.2 PortfolioSnapshot スキーマ
#   docs/order_windows.md §仮想約定エンジン §main.rs の拡張
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S35: 仮想ポートフォリオ ライフサイクル検証 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

setup_single_pane "$E2E_TICKER" "M1" "$START" "$END"

start_app
headless_play

if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "auto-play で Playing に到達せず（60s タイムアウト）"
  print_summary; exit 1
fi
echo "  REPLAY Playing 到達"

# Paused にして状態を安定させてから検証する
api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

# ─────────────────────────────────────────────────────────────────────────────
# TC-A〜G: 初期ポートフォリオスナップショット
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A〜G: 初期ポートフォリオスナップショット"

PORTFOLIO=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio: $PORTFOLIO"

# TC-A: HTTP 200 かつ JSON が返る
CODE_A=$(curl -s -o /dev/null -w "%{http_code}" "$API_BASE/api/replay/portfolio")
[ "$CODE_A" = "200" ] \
  && pass "TC-A: GET /api/replay/portfolio → HTTP 200" \
  || fail "TC-A" "HTTP=$CODE_A (expected 200)"

# TC-B: 初期 cash = 1000000.0
CASH=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$CASH" = "1000000" ] \
  && pass "TC-B: 初期 cash = 1000000.0" \
  || fail "TC-B" "cash=$CASH (expected 1000000)"

# TC-C: 初期 unrealized_pnl = 0
UNREALIZED=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).unrealized_pnl)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$UNREALIZED" = "0" ] \
  && pass "TC-C: 初期 unrealized_pnl = 0" \
  || fail "TC-C" "unrealized_pnl=$UNREALIZED (expected 0)"

# TC-D: 初期 realized_pnl = 0
REALIZED=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).realized_pnl)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$REALIZED" = "0" ] \
  && pass "TC-D: 初期 realized_pnl = 0" \
  || fail "TC-D" "realized_pnl=$REALIZED (expected 0)"

# TC-E: total_equity = cash（ポジションなし時）
EQUITY=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    console.log(String(d.total_equity));
  } catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$EQUITY" = "1000000" ] \
  && pass "TC-E: 初期 total_equity = cash (1000000)" \
  || fail "TC-E" "total_equity=$EQUITY (expected 1000000)"

# TC-F: 初期 open_positions = []
OPEN_LEN=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$OPEN_LEN" = "0" ] \
  && pass "TC-F: 初期 open_positions = [] (length=0)" \
  || fail "TC-F" "open_positions.length=$OPEN_LEN (expected 0)"

# TC-G: 初期 closed_positions = []
CLOSED_LEN=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).closed_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
[ "$CLOSED_LEN" = "0" ] \
  && pass "TC-G: 初期 closed_positions = [] (length=0)" \
  || fail "TC-G" "closed_positions.length=$CLOSED_LEN (expected 0)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-H〜I: 注文 place 後のポートフォリオ
# Trades EventStore が未統合（docs/replay_header.md §13.2 既知制限 #1）のため
# on_tick に trade が来ず、市場注文は Pending のまま約定しない。
# → cash / open_positions は初期値のまま変化しないことを確認する。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-H〜I: 注文 place 後のポートフォリオ（約定なし確認）"

# 成行買い 2 件 place
api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"buy\",\"qty\":0.1,\"order_type\":\"market\"}" > /dev/null
api_post /api/replay/order \
  "{\"ticker\":\"$(order_symbol)\",\"side\":\"sell\",\"qty\":0.05,\"order_type\":\"market\"}" > /dev/null

# Paused のまま少し待ってからポートフォリオを確認（tick は来ない）
sleep 1
PORTFOLIO_AFTER=$(curl -s "$API_BASE/api/replay/portfolio")
echo "  portfolio after orders: $PORTFOLIO_AFTER"

# TC-H: Paused 中は約定しないため cash は不変
CASH_AFTER=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_AFTER")
[ "$CASH_AFTER" = "1000000" ] \
  && pass "TC-H: Paused 中に成行注文を place しても cash は不変 ($CASH_AFTER)" \
  || fail "TC-H" "cash=$CASH_AFTER (expected 1000000 — Paused なので約定しないはず)"

# TC-I: Paused 中は約定しないため open_positions は空のまま
OPEN_AFTER=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_AFTER")
[ "$OPEN_AFTER" = "0" ] \
  && pass "TC-I: Paused 中 open_positions は空のまま (length=0)" \
  || fail "TC-I" "open_positions.length=$OPEN_AFTER (expected 0)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-J: PEND — StepBackward によるエンジンリセット（未実装）
# docs/order_windows.md §未実装: "SeekBackward 時のエンジンリセット"
# 現状は Live↔Replay 遷移時のみ reset()。StepBackward は engine を reset しない。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-J: StepBackward によるポートフォリオリセット（実装待ち）"
pend "TC-J" "StepBackward 後のエンジンリセットは未実装 (docs/order_windows.md §未実装)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-K〜L: Live → Replay 遷移でエンジンが reset() される
# docs/order_windows.md §main.rs の拡張:
#   "Live → Replay 遷移: VirtualExchangeEngine::new(1_000_000.0) で初期化（既存なら reset()）"
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-K〜L: Live → Replay 遷移でエンジンリセット"

if is_headless; then
  pend "TC-K" "headless は Live/Replay toggle 非対応"
  pend "TC-L" "headless は Live/Replay toggle 非対応"
else
  # Replay → Live → Replay とトグルし、エンジンが reset() されることを確認
  api_post /api/replay/toggle > /dev/null  # → Live
  LIVE_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
  echo "  toggle 後のモード: $LIVE_MODE"

  api_post /api/replay/toggle > /dev/null  # → Replay (engine.reset() が呼ばれる)
  REPLAY_MODE=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
  echo "  再 toggle 後のモード: $REPLAY_MODE"

  [ "$REPLAY_MODE" = "Replay" ] \
    && pass "TC-K: Live → Replay 再遷移成功 (mode=$REPLAY_MODE)" \
    || fail "TC-K" "mode=$REPLAY_MODE (expected Replay)"

  # reset() 後はポートフォリオが初期値に戻るはず
  PORTFOLIO_RESET=$(curl -s "$API_BASE/api/replay/portfolio")
  echo "  portfolio after reset: $PORTFOLIO_RESET"

  CASH_RESET=$(node -e "
    try { console.log(String(JSON.parse(process.argv[1]).cash)); }
    catch(e) { console.log('null'); }
  " "$PORTFOLIO_RESET")
  OPEN_RESET=$(node -e "
    try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
    catch(e) { console.log('null'); }
  " "$PORTFOLIO_RESET")

  [ "$CASH_RESET" = "1000000" ] && [ "$OPEN_RESET" = "0" ] \
    && pass "TC-L: Live→Replay 遷移後 portfolio リセット (cash=$CASH_RESET, open_positions=[])" \
    || fail "TC-L" "cash=$CASH_RESET open_positions=$OPEN_RESET (expected cash=1000000, open=0)"
fi

stop_app
print_summary
