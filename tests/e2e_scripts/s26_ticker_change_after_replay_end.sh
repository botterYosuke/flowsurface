#!/usr/bin/env bash
# s26_ticker_change_after_replay_end.sh — S26: リプレイ終了後の銘柄変更で current_time がリセットされること
#
# 検証シナリオ:
#   TC-A: リプレイ終了（Paused @ end_time）→ 銘柄変更 → current_time が start_time に戻る
#   TC-B: 銘柄変更後のステータスは Paused のまま
#   TC-C: Resume → Playing に遷移できる（リセット後の再生が正常）
#
# 再現する不具合（修正前）:
#   Task::chain() により ReloadKlineStream（clock.seek(start)）が kline_fetch_task 完了待ち
#   になり、Tachibana セッションなしでは無限にブロックされ current_time がリセットされない。
#   → current_time が end_time のまま固定される。
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S26: リプレイ終了後の銘柄変更で current_time がリセットされること ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── ヘルパー ─────────────────────────────────────────────────────────────────

get_pane_id() {
  local panes
  panes=$(curl -s "$API/pane/list")
  node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$panes"
}

get_status_field() {
  local field="$1"
  jqn "$(curl -s "$API/replay/status")" "d.$field"
}

# ── フィクスチャ ──────────────────────────────────────────────────────────────
# BinanceLinear:BTCUSDT M1、過去 15 分のレンジ（10x 加速で ~1秒以内に終端到達）
START=$(utc_offset -0.5)
END=$(utc_offset -0.25)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

echo "  range: $START → $END (${START_MS} → ${END_MS})"

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S26","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S26"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# Playing 到達待機（最大 60 秒）
if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "auto-play で Playing に到達せず"
  print_summary
  exit 1
fi
echo "  Playing 到達"

# ── 10x 加速して終端まで再生 ─────────────────────────────────────────────────
speed_to_10x
echo "  10x 加速完了、終端まで待機..."

# Paused + current_time ≈ end_time になるまでポーリング（最大 120 秒）
REACHED_END="false"
for i in $(seq 1 120); do
  STATUS=$(curl -s "$API/replay/status")
  CT=$(jqn "$STATUS" "d.current_time")
  ST=$(jqn "$STATUS" "d.status")
  if [ "$ST" = "Paused" ] && [ "$CT" != "null" ]; then
    # end_time の 2 分以内（120000 ms）なら終端到達とみなす
    NEAR_END=$(node -e "console.log(BigInt('$CT') >= BigInt('$END_MS') - BigInt('120000'))")
    if [ "$NEAR_END" = "true" ]; then
      REACHED_END="true"
      break
    fi
  fi
  sleep 1
done

if [ "$REACHED_END" != "true" ]; then
  LAST_STATUS=$(curl -s "$API/replay/status")
  fail "precond" "終端到達しなかった: status=$(jqn "$LAST_STATUS" "d.status") current_time=$(jqn "$LAST_STATUS" "d.current_time")"
  print_summary
  exit 1
fi

# 終端到達時の情報を取得
FINAL_STATUS=$(curl -s "$API/replay/status")
CT_AT_END=$(jqn "$FINAL_STATUS" "d.current_time")
START_TIME_MS=$(jqn "$FINAL_STATUS" "d.start_time")

echo "  終端到達: current_time=$CT_AT_END start_time=$START_TIME_MS end_time=$END_MS"

# 前提確認: current_time が end_time 近くにある（start_time とは異なる）
CT_DIFFERS_FROM_START=$(node -e "console.log(BigInt('$CT_AT_END') !== BigInt('$START_TIME_MS'))")
if [ "$CT_DIFFERS_FROM_START" != "true" ]; then
  echo "  [SKIP] current_time が既に start_time と一致 — レンジが小さすぎてテスト不成立"
  print_summary
  exit 0
fi

# ペイン ID 取得
PANE_ID=$(get_pane_id)
if [ -z "$PANE_ID" ]; then
  fail "precond" "ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE_ID=$PANE_ID"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: リプレイ終了後に銘柄変更 → current_time が start_time に戻る
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: リプレイ終了後に銘柄変更 → current_time が start_time に戻る"

api_post /api/pane/set-ticker "{\"pane_id\":\"$PANE_ID\",\"ticker\":\"BinanceLinear:ETHUSDT\"}" > /dev/null
sleep 2

CT_AFTER_CHANGE=$(get_status_field "current_time")
echo "  銘柄変更後 current_time=$CT_AFTER_CHANGE (start_time=$START_TIME_MS)"

# current_time が start_time と一致するか確認
IS_RESET=$(node -e "
  const ct = BigInt('$CT_AFTER_CHANGE');
  const st = BigInt('$START_TIME_MS');
  // start_time の ±1 バー（60秒）以内なら OK（bar スナップによるずれを許容）
  const diff = ct > st ? ct - st : st - ct;
  console.log(diff <= BigInt('60000') ? 'true' : 'false');
")

if [ "$IS_RESET" = "true" ]; then
  pass "TC-A: 銘柄変更後 current_time が start_time 付近にリセットされた (ct=$CT_AFTER_CHANGE st=$START_TIME_MS)"
else
  fail "TC-A: current_time がリセットされない" \
    "current_time=$CT_AFTER_CHANGE start_time=$START_TIME_MS end_time=$END_MS (修正前の挙動: end_time 付近のまま)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: 銘柄変更後も Paused のまま（自動再生されない）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: 銘柄変更後 status=Paused のまま"

ST_AFTER=$(get_status_field "status")
[ "$ST_AFTER" = "Paused" ] \
  && pass "TC-B: 銘柄変更後 status=Paused" \
  || fail "TC-B" "status=$ST_AFTER (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Resume → Playing に遷移できる
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: Paused → Resume → Playing"

api_post /api/replay/resume > /dev/null
if wait_status "Playing" 30; then
  pass "TC-C: リセット後 Resume → Playing 到達"
else
  fail "TC-C" "status=$(get_status_field "status") (expected Playing)"
fi

stop_app
print_summary
