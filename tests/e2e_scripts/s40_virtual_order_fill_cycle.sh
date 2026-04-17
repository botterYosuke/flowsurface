#!/usr/bin/env bash
# s40_virtual_order_fill_cycle.sh — S40: 仮想取引フルサイクル検証
#
# 検証シナリオ:
#   A:   REPLAY Playing 到達
#   B:   成行買い 1.0 BTC → HTTP 200, order_id 返却
#   C:   step-forward を最大 5 回 → open_positions.length >= 1（約定確認）
#   D:   portfolio.cash < 1_000_000（買い約定コスト分が差し引かれていること）
#   E:   成行売り 1.0 BTC → HTTP 200, order_id 返却
#   F:   step-forward を最大 5 回 → open_positions.length == 0（Long クローズ確認）
#   G:   closed_positions.length == 1（クローズ済みポジションが存在）
#   H:   realized_pnl != 0（PnL が確定している）
#   I:   total_equity == cash + unrealized_pnl（スナップショット整合性）
#
# 仕様根拠:
#   docs/replay_header.md §11.2 — 仮想約定エンジン API
#   docs/order_windows.md §仮想約定エンジン §main.rs の拡張
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
#
# 前提条件:
#   DEV_USER_ID / DEV_PASSWORD 環境変数設定済み（未設定時は SKIP）
#   ビルド: cargo build（debug_assertions が必要 — auto-login は #[cfg(debug_assertions)] のみ有効）
#   注意: step-forward が trades を含むかはリプレイデータに依存する。
#         trades が空の step が続く可能性があるため、TC-C / TC-F は最大 5 回リトライする。
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S40: 仮想取引フルサイクル検証 ==="

# ── DEV 認証情報チェック ────────────────────────────────────────────────────────
# 自動ログインは #[cfg(debug_assertions)] のみ有効。
# DEV_USER_ID / DEV_PASSWORD が未設定の環境（CI / 本番バイナリ）では SKIP する。
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — 自動ログインが無効です"
  echo "  (cargo build でビルドした debug バイナリ + 環境変数設定済みの環境で実行してください)"
  exit 0
fi

backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S40","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S40"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# ── Tachibana 自動ログイン確認 ─────────────────────────────────────────────────
# DEV_USER_ID / DEV_PASSWORD により LoginScreen::new() が自動送信する。
# セッションが確立されるまで最大 60 秒待機する。
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

# Paused にして状態を安定させてから注文を入れる
api_post /api/replay/pause > /dev/null
wait_status "Paused" 10

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: 成行買い 1.0 BTC → HTTP 200, order_id 返却
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: 成行買い 1.0 BTC"

BUY_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":1.0,"order_type":"market"}')
echo "  buy response: $BUY_RESP"

BUY_CODE=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"buy","qty":0.001,"order_type":"market"}')
[ "$BUY_CODE" = "200" ] \
  && pass "TC-B1: POST /api/replay/order (成行買い) → HTTP 200" \
  || fail "TC-B1" "HTTP=$BUY_CODE (expected 200)"

BUY_ID=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    const id = d.order_id;
    console.log(typeof id === 'string' && id.length > 0 ? id : 'null');
  } catch(e) { console.log('null'); }
" "$BUY_RESP")
[ "$BUY_ID" != "null" ] \
  && pass "TC-B2: order_id が返却された ($BUY_ID)" \
  || fail "TC-B2" "order_id が null (response=$BUY_RESP)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: step-forward を最大 5 回 → open_positions.length >= 1
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: step-forward で約定待ち（最大 5 回）"

OPEN=0
for i in $(seq 1 5); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.5
  PORTFOLIO=$(curl -s "$API_BASE/api/replay/portfolio")
  OPEN=$(node -e "
    try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
    catch(e) { console.log('0'); }
  " "$PORTFOLIO")
  echo "  step $i: open_positions.length=$OPEN"
  [ "$OPEN" -ge 1 ] && break
done

[ "$OPEN" -ge 1 ] \
  && pass "TC-C: 買い約定後 open_positions.length=$OPEN (>= 1)" \
  || fail "TC-C" "5 回 step-forward 後も open_positions=0 (約定せず)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: portfolio.cash < 1_000_000（購入コスト分が差し引かれている）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: cash が購入コスト分だけ減少していること"

CASH=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO")
echo "  cash after buy fill: $CASH"

CASH_DECREASED=$(node -e "
  try {
    const c = parseFloat(process.argv[1]);
    console.log(isNaN(c) ? 'false' : (c < 1000000 ? 'true' : 'false'));
  } catch(e) { console.log('false'); }
" "$CASH")
[ "$CASH_DECREASED" = "true" ] \
  && pass "TC-D: cash < 1_000_000 (cash=$CASH — 購入コスト分が差し引かれた)" \
  || fail "TC-D" "cash=$CASH (expected < 1000000 — 購入コストが差し引かれていない)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E: 成行売り 1.0 BTC → HTTP 200, order_id 返却
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E: 成行売り 1.0 BTC"

SELL_RESP=$(api_post /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":1.0,"order_type":"market"}')
echo "  sell response: $SELL_RESP"

SELL_CODE=$(api_post_code /api/replay/order \
  '{"ticker":"BTCUSDT","side":"sell","qty":0.001,"order_type":"market"}')
[ "$SELL_CODE" = "200" ] \
  && pass "TC-E1: POST /api/replay/order (成行売り) → HTTP 200" \
  || fail "TC-E1" "HTTP=$SELL_CODE (expected 200)"

SELL_ID=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    const id = d.order_id;
    console.log(typeof id === 'string' && id.length > 0 ? id : 'null');
  } catch(e) { console.log('null'); }
" "$SELL_RESP")
[ "$SELL_ID" != "null" ] \
  && pass "TC-E2: sell order_id が返却された ($SELL_ID)" \
  || fail "TC-E2" "sell order_id が null (response=$SELL_RESP)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-F: step-forward を最大 5 回 → open_positions.length == 0
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F: step-forward で Long クローズ待ち（最大 5 回）"

OPEN_AFTER=99
for i in $(seq 1 5); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.5
  PORTFOLIO_FINAL=$(curl -s "$API_BASE/api/replay/portfolio")
  OPEN_AFTER=$(node -e "
    try { console.log(String(JSON.parse(process.argv[1]).open_positions.length)); }
    catch(e) { console.log('99'); }
  " "$PORTFOLIO_FINAL")
  echo "  step $i: open_positions.length=$OPEN_AFTER"
  [ "$OPEN_AFTER" -eq 0 ] && break
done

[ "$OPEN_AFTER" -eq 0 ] \
  && pass "TC-F: 売り約定後 open_positions.length=0 (Long がクローズされた)" \
  || fail "TC-F" "5 回 step-forward 後も open_positions=$OPEN_AFTER (クローズされず)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-G: closed_positions.length == 1
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-G: closed_positions.length == 1"

CLOSED=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).closed_positions.length)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_FINAL")
[ "$CLOSED" = "1" ] \
  && pass "TC-G: closed_positions.length=1 (ポジションがクローズ済みに移動)" \
  || fail "TC-G" "closed_positions.length=$CLOSED (expected 1)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-H: realized_pnl != 0（PnL が確定している）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-H: realized_pnl が確定していること"

REALIZED=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).realized_pnl)); }
  catch(e) { console.log('null'); }
" "$PORTFOLIO_FINAL")
echo "  realized_pnl: $REALIZED"

REALIZED_NONZERO=$(node -e "
  try {
    const r = parseFloat(process.argv[1]);
    console.log(!isNaN(r) && r !== 0 ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$REALIZED")
[ "$REALIZED_NONZERO" = "true" ] \
  && pass "TC-H: realized_pnl=$REALIZED (≠ 0 — PnL が確定した)" \
  || fail "TC-H" "realized_pnl=$REALIZED (expected != 0)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-I: total_equity == cash + unrealized_pnl（スナップショット整合性）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-I: total_equity の整合性確認"

EQUITY_OK=$(node -e "
  try {
    const d = JSON.parse(process.argv[1]);
    const diff = Math.abs(d.total_equity - (d.cash + d.unrealized_pnl));
    console.log(diff < 0.01 ? 'true' : 'false');
  } catch(e) { console.log('false'); }
" "$PORTFOLIO_FINAL")
FINAL_CASH=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).cash)); }
  catch(e) { console.log('?'); }
" "$PORTFOLIO_FINAL")
FINAL_EQUITY=$(node -e "
  try { console.log(String(JSON.parse(process.argv[1]).total_equity)); }
  catch(e) { console.log('?'); }
" "$PORTFOLIO_FINAL")
[ "$EQUITY_OK" = "true" ] \
  && pass "TC-I: total_equity=$FINAL_EQUITY = cash($FINAL_CASH) + unrealized_pnl (整合)" \
  || fail "TC-I" "total_equity と cash+unrealized_pnl が不一致 (portfolio=$PORTFOLIO_FINAL)"

stop_app
print_summary
