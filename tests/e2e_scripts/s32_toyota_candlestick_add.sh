#!/usr/bin/env bash
# s32_toyota_candlestick_add.sh — S32: TOYOTA candlestick チャート追加テスト
#
# シナリオ:
#   saved-state.json サンプル（BinanceLinear:BTCUSDT M1、Replay 2025-04-15 04:49 〜 2026-04-15 06:49）
#   で起動後、TOYOTA（TachibanaSpot:7203）D1 candlestick チャートを追加し、
#   以下の期待動作を検証する:
#
#     1. TOYOTA の 1d チャートが追加される（pane split + set-ticker + set-timeframe）
#     2. REPLAY が start 時間に戻って再開される
#        - current_time == start_time（clock.seek(range.start) が発火）
#        - status = Paused
#        - [Tachibana セッションあり時] Resume → Playing に遷移
#
# ビルド要件:
#   通常ビルド  : cargo build --release
#   e2e-mock ビルド: cargo build --release --features e2e-mock  （inject-session エンドポイント有効）
#
# Tachibana セッションなし時の動作:
#   - TC-S32-05/06/07 は実行（セッション不要の検証）
#   - TC-S32-08〜10 は PEND（streams_ready / Playing チェック）
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S32: TOYOTA candlestick チャート追加テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ: saved-state.json サンプルと同等の構成 ─────────────────────
# range_start = "2025-04-15 04:49" が clock.seek のターゲット（固定）
RANGE_START="2025-04-15 04:49"
RANGE_END="2026-04-15 06:49"

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"Test-M1","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"Test-M1"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$RANGE_START","range_end":"$RANGE_END"}
}
EOF

echo "  fixture: BTCUSDT M1, replay $RANGE_START → $RANGE_END"

# ── アプリ起動（auto-play 付き）───────────────────────────────────────────────
start_app

# ── TC-S32-01: auto-play → Playing 到達 ─────────────────────────────────────
echo ""
echo "── TC-S32-01: auto-play → Playing 到達"
if wait_playing 120; then
  pass "TC-S32-01: auto-play → Playing 到達"
else
  fail "TC-S32-01" "Playing 未到達（120 秒タイムアウト）: status=$(jqn "$(curl -s "$API/replay/status")" "d.status")"
  print_summary
  exit 1
fi

# start_time を API から取得（基準値として使用）
STATUS_RESP=$(curl -s "$API/replay/status")
START_TIME_MS=$(jqn "$STATUS_RESP" "d.start_time")
echo "  start_time_ms=$START_TIME_MS  (= ${RANGE_START} UTC)"

# ── 初期ペイン ID 取得 ──────────────────────────────────────────────────────
PANES=$(curl -s "$API/pane/list")
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE0" ]; then
  fail "TC-S32-precond" "初期ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE0=$PANE0"

# ── Tachibana セッション確認（inject-session 試行 → keyring フォールバック）──
echo ""
echo "── Tachibana セッション確認"
TACH_SESSION="none"

# e2e-mock ビルドなら inject-session が使える
INJECT_RESP=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/test/tachibana/inject-session" 2>/dev/null || echo "000")
if [ "$INJECT_RESP" = "200" ]; then
  TACH_STATUS=$(curl -s "$API/auth/tachibana/status" 2>/dev/null || echo '{}')
  TACH_SESSION=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.session||'none');}catch(e){console.log('none');}" "$TACH_STATUS")
  echo "  inject-session 成功 → session=$TACH_SESSION"
else
  # keyring からの実セッションを確認
  TACH_STATUS=$(curl -s "$API/auth/tachibana/status" 2>/dev/null || echo '{}')
  TACH_SESSION=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.session||'none');}catch(e){console.log('none');}" "$TACH_STATUS")
  echo "  inject-session 利用不可 (HTTP=$INJECT_RESP) → session=$TACH_SESSION"
fi

if [ "$TACH_SESSION" = "none" ]; then
  echo "  INFO: Tachibana セッションなし — TC-S32-08〜10 は PEND"
fi

# ── TC-S32-02: ペイン split → pane count = 2 ─────────────────────────────────
echo ""
echo "── TC-S32-02: ペイン split → pane count = 2"
curl -s -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null

if wait_for_pane_count 2 10; then
  pass "TC-S32-02: split 後 pane count = 2"
else
  fail "TC-S32-02" "10 秒以内に pane count が 2 にならなかった"
  print_summary
  exit 1
fi

# 新ペイン ID 取得
PANES=$(curl -s "$API/pane/list")
NEW_PANE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? p.id : '');
" "$PANES")
echo "  NEW_PANE=$NEW_PANE"
if [ -z "$NEW_PANE" ]; then
  fail "TC-S32-02b" "新ペイン ID 取得失敗"
  print_summary
  exit 1
fi

# ── TC-S32-03: 新ペインに set-ticker TachibanaSpot:7203 ─────────────────────
echo ""
echo "── TC-S32-03: 新ペインに set-ticker TachibanaSpot:7203"
SET_TICKER_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$NEW_PANE\",\"ticker\":\"TachibanaSpot:7203\"}")
[ "$SET_TICKER_CODE" = "200" ] \
  && pass "TC-S32-03: set-ticker TachibanaSpot:7203 → HTTP 200" \
  || fail "TC-S32-03" "HTTP=$SET_TICKER_CODE (expected 200)"

# ── TC-S32-04: 新ペインに set-timeframe D1 ───────────────────────────────────
echo ""
echo "── TC-S32-04: 新ペインに set-timeframe D1"
SET_TF_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST "$API/pane/set-timeframe" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$NEW_PANE\",\"timeframe\":\"D1\"}")
[ "$SET_TF_CODE" = "200" ] \
  && pass "TC-S32-04: set-timeframe D1 → HTTP 200" \
  || fail "TC-S32-04" "HTTP=$SET_TF_CODE (expected 200)"

# ticker/timeframe 変更が反映されるまで少し待機
sleep 1

# ── TC-S32-05: current_time == start_time（clock.seek が発火）───────────────
echo ""
echo "── TC-S32-05: current_time == start_time（Replay が start に戻る）"
STATUS_AFTER=$(curl -s "$API/replay/status")
CT=$(jqn "$STATUS_AFTER" "d.current_time")
ST=$(jqn "$STATUS_AFTER" "d.start_time")
echo "  current_time=$CT  start_time=$ST"

if [ -n "$CT" ] && [ "$CT" != "null" ] && [ -n "$ST" ] && [ "$ST" != "null" ]; then
  if node -e "process.exit(BigInt('$CT') === BigInt('$ST') ? 0 : 1)" 2>/dev/null; then
    pass "TC-S32-05: current_time == start_time (clock.seek が正しく発火)"
  else
    fail "TC-S32-05" "current_time=$CT != start_time=$ST (expected clock.seek(range.start))"
  fi
else
  fail "TC-S32-05" "current_time または start_time が null (CT=$CT, ST=$ST)"
fi

# ── TC-S32-06: status = Paused（自動再生しない）─────────────────────────────
echo ""
echo "── TC-S32-06: ticker 変更後 status = Paused"
STATUS_STR=$(jqn "$STATUS_AFTER" "d.status")
[ "$STATUS_STR" = "Paused" ] \
  && pass "TC-S32-06: status = Paused（自動再生なし）" \
  || fail "TC-S32-06" "status=$STATUS_STR (expected Paused)"

# ── TC-S32-07: 新ペインの ticker/timeframe が正しく設定されている ──────────
echo ""
echo "── TC-S32-07: 新ペインの ticker/timeframe 確認"
PANES_AFTER=$(curl -s "$API/pane/list")
NEW_TICKER=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$NEW_PANE');
  console.log(p ? (p.ticker || 'null') : 'not_found');
" "$PANES_AFTER")
NEW_TF=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$NEW_PANE');
  console.log(p ? (p.timeframe || 'null') : 'not_found');
" "$PANES_AFTER")
echo "  new pane ticker=$NEW_TICKER  timeframe=$NEW_TF"

# pane/list は ticker を正規化して返す（"Tachibana:7203" 形式）
# set-ticker で "TachibanaSpot:7203" を渡しても表示は "Tachibana:7203" になる
if echo "$NEW_TICKER" | grep -q "7203"; then
  pass "TC-S32-07a: 新ペイン ticker に 7203 が含まれる (=$NEW_TICKER)"
else
  fail "TC-S32-07a" "ticker=$NEW_TICKER (expected to contain '7203')"
fi

[ "$NEW_TF" = "D1" ] \
  && pass "TC-S32-07b: 新ペイン timeframe = D1" \
  || fail "TC-S32-07b" "timeframe=$NEW_TF (expected D1)"

# ── TC-S32-08〜10: Tachibana セッションあり時のみ実行 ───────────────────────
echo ""
if [ "$TACH_SESSION" = "none" ]; then
  pend "TC-S32-08: 新ペイン streams_ready = true" "Tachibana セッションなし"
  pend "TC-S32-09: Resume → Playing" "Tachibana セッションなし"
  pend "TC-S32-10: current_time 前進（再生継続）" "Tachibana セッションなし"
else
  # TC-S32-08: streams_ready = true（TOYOTA D1 データロード完了）
  echo "── TC-S32-08: 新ペイン streams_ready = true を待機（Tachibana D1）"
  if wait_for_streams_ready "$NEW_PANE" 120; then
    pass "TC-S32-08: TachibanaSpot:7203 D1 streams_ready = true"
  else
    fail "TC-S32-08" "streams_ready タイムアウト（120 秒）— Tachibana D1 データロード失敗"
  fi

  # TC-S32-09: Resume → Playing
  echo ""
  echo "── TC-S32-09: Resume → Playing"
  api_post /api/replay/resume > /dev/null
  if wait_status "Playing" 30; then
    pass "TC-S32-09: Resume → Playing 到達"
  else
    fail "TC-S32-09" "status=$(jqn "$(curl -s "$API/replay/status")" "d.status") (expected Playing)"
  fi

  # TC-S32-10: current_time が前進（再生が正常動作）
  echo ""
  echo "── TC-S32-10: current_time が前進"
  T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if T2=$(wait_for_time_advance "$T1" 15); then
    pass "TC-S32-10: current_time 前進 ($T1 → $T2)"
  else
    fail "TC-S32-10" "15 秒待機しても current_time が変化しなかった"
  fi
fi

print_summary
[ $FAIL -eq 0 ]
