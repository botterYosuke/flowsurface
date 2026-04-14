#!/usr/bin/env bash
# s23_mid_replay_ticker_change.sh — S23: mid-replay 銘柄・timeframe 変更時の自動再生防止
#
# 検証シナリオ:
#   A: Play → 銘柄変更 → status = Paused
#   B: Play → timeframe 変更 → status = Paused
#   C: Play → 銘柄変更 → データロード待機 → status = Paused（自動再生されない）
#   D: Play → 銘柄変更 → Resume → status = Playing
#   E: Pause → 銘柄変更 → status = Paused のまま
#   F: Play のみ（通常フロー）→ Loading → Playing（回帰）
#   G: Play → 銘柄変更 → 別銘柄に再変更 → Resume → status = Playing
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S23: mid-replay 銘柄・timeframe 変更時の自動再生防止 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── ヘルパー ─────────────────────────────────────────────────────────────────

get_pane_id() {
  local panes
  panes=$(curl -s "$API/pane/list")
  node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$panes"
}

get_status() {
  jqn "$(curl -s "$API/replay/status")" "d.status"
}

set_ticker() {
  local pane_id="$1" ticker="$2"
  api_post /api/pane/set-ticker "{\"pane_id\":\"$pane_id\",\"ticker\":\"$ticker\"}" > /dev/null
}

set_timeframe() {
  local pane_id="$1" tf="$2"
  api_post /api/pane/set-timeframe "{\"pane_id\":\"$pane_id\",\"timeframe\":\"$tf\"}" > /dev/null
}

# status が want になるまで最大 timeout 秒ポーリング（成功 = 0, 失敗 = 1）
poll_status() {
  local want="$1" timeout="${2:-15}"
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local s
    s=$(get_status)
    [ "$s" = "$want" ] && return 0
    sleep 0.5
  done
  return 1
}

# ── フィクスチャ: BinanceLinear BTCUSDT M1, 過去 3h〜1h（autoplay）─────────

START=$(utc_offset -3)
END=$(utc_offset -1)

write_fixture() {
  cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S23","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S23"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF
}

# ─────────────────────────────────────────────────────────────────────────────
# TC-F: 通常 Play フロー（回帰）— autoplay 起動後に Loading → Playing に遷移する
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-F: 通常 Play フロー（回帰）"
write_fixture
start_app

# API 準備直後の mode を確認する。
# Play 発火前は status フィールドが存在しない（null）ため mode だけ確認する。
INIT_RESP=$(curl -s "$API/replay/status")
INIT_MODE=$(jqn "$INIT_RESP" "d.mode")
echo "  F: 起動直後 mode=$INIT_MODE"
# Replay モード（または Loading/Playing への遷移中）であれば OK
[ "$INIT_MODE" = "Replay" ] || [ "$(get_status)" = "Loading" ] || [ "$(get_status)" = "Playing" ] \
  && pass "TC-F1: autoplay 起動後 mode=Replay (status フィールドは Play 発火後に現れる)" \
  || fail "TC-F1" "mode=$INIT_MODE (expected Replay)"

# Playing に到達するまで待機（最大 60 秒）
if wait_status "Playing" 60; then
  pass "TC-F2: 通常 Play フロー → Playing 到達"
else
  fail "TC-F2" "Playing 未到達（timeout）: status=$(get_status)"
  print_summary
  exit 1
fi

PANE_ID=$(get_pane_id)
echo "  PANE_ID=$PANE_ID"
if [ -z "$PANE_ID" ]; then
  fail "precond" "ペイン ID 取得失敗"
  print_summary
  exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Playing 中に銘柄変更 → 即座に Paused
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: Playing 中に銘柄変更 → 即座に Paused"
set_ticker "$PANE_ID" "BinanceLinear:ETHUSDT"
sleep 0.5

ST=$(get_status)
[ "$ST" = "Paused" ] \
  && pass "TC-A: 銘柄変更後 status=Paused" \
  || fail "TC-A" "status=$ST (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: データロード完了後も Paused のまま（自動再生されない）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: データロード後も Paused のまま"
# streams_ready=true になるまで待機 → ロード完了を確認
if wait_for_streams_ready "$PANE_ID" 30; then
  echo "  C: streams_ready=true 確認"
  ST=$(get_status)
  [ "$ST" = "Paused" ] \
    && pass "TC-C: streams_ready 後も status=Paused（自動再生なし）" \
    || fail "TC-C" "status=$ST (expected Paused — 自動再生が発生した)"
else
  # タイムアウト時は 3 秒待機して確認（最低限の保証）
  echo "  C: streams_ready 未到達、3 秒待機して status を確認"
  sleep 3
  ST=$(get_status)
  [ "$ST" = "Paused" ] \
    && pass "TC-C: 3 秒後も status=Paused（自動再生なし）" \
    || fail "TC-C" "status=$ST (expected Paused — 自動再生が発生した)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: Paused 状態から Resume → Playing
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: Paused → Resume → Playing"
api_post /api/replay/resume > /dev/null
if wait_status "Playing" 30; then
  pass "TC-D: Resume → Playing 到達"
else
  fail "TC-D" "status=$(get_status) (expected Playing)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Playing 中に timeframe 変更 → Paused
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: Playing 中に timeframe 変更 → Paused"
set_timeframe "$PANE_ID" "M5"
sleep 0.5

ST=$(get_status)
[ "$ST" = "Paused" ] \
  && pass "TC-B: timeframe 変更後 status=Paused" \
  || fail "TC-B" "status=$ST (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E: Paused 中に銘柄変更 → Paused のまま
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E: Paused 中に銘柄変更 → Paused のまま"
# 現在 Paused（TC-B の結果）
set_ticker "$PANE_ID" "BinanceLinear:BTCUSDT"
sleep 1

ST=$(get_status)
[ "$ST" = "Paused" ] \
  && pass "TC-E: Paused 中に銘柄変更後も status=Paused" \
  || fail "TC-E" "status=$ST (expected Paused)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-G: 連続銘柄変更（Playing → 2 回変更 → Resume → Playing）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-G: 連続銘柄変更後 Resume → Playing"

# まず Playing に戻す
api_post /api/replay/resume > /dev/null
if ! wait_status "Playing" 30; then
  fail "TC-G-pre" "Playing 到達失敗（前提条件） status=$(get_status)"
  print_summary
  exit 1
fi

# Playing 中に 2 回連続で銘柄変更（1 回目: ETHUSDT、間髪入れず 2 回目: BTCUSDT）
set_ticker "$PANE_ID" "BinanceLinear:ETHUSDT"
sleep 0.3
set_ticker "$PANE_ID" "BinanceLinear:BTCUSDT"
sleep 0.5

# 連続変更後は Paused のはず
ST_AFTER=$(get_status)
echo "  G: 連続変更後 status=$ST_AFTER"

# Resume → Playing
api_post /api/replay/resume > /dev/null
if wait_status "Playing" 30; then
  pass "TC-G: 連続銘柄変更後 Resume → Playing 到達"
else
  fail "TC-G" "status=$(get_status) (expected Playing)"
fi

stop_app

print_summary
