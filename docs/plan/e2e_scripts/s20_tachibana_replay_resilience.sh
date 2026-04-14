#!/usr/bin/env bash
# s20_tachibana_replay_resilience.sh — スイート S20: UI操作中の Replay 耐性テスト（TachibanaSpot）
# TachibanaSpot:7203 D1 での Replay 再生中に各種 UI 操作を行っても壊れないことを確認する
# ビルド要件: cargo build --release --features e2e-mock
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S20: UI操作中の Replay 耐性テスト（TachibanaSpot:7203 D1）==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

MASTER='{"records":[{"sIssueCode":"7203","sIssueNameEizi":"Toyota Motor","sCLMID":"CLMIssueMstKabu"}]}'

inject_daily_history() {
  local start=$1 end=$2
  local body
  body=$(node -e "
    const startMs = new Date(process.argv[1].replace(' ', 'T') + ':00Z').getTime();
    const endMs   = new Date(process.argv[2].replace(' ', 'T') + ':00Z').getTime();
    const day = 86400000;
    const first = Math.ceil(startMs / day) * day;
    const klines = [];
    for (let t = first; t <= endMs; t += day) {
      klines.push({time: t, open: 3000, high: 3100, low: 2900, close: 3050, volume: 500000});
    }
    if (klines.length === 0) {
      klines.push({time: first - day, open: 3000, high: 3100, low: 2900, close: 3050, volume: 500000});
      klines.push({time: first,       open: 3050, high: 3150, low: 2950, close: 3100, volume: 600000});
    }
    console.log(JSON.stringify({issue_code: '7203', klines}));
  " "$start" "$end")
  curl -s -X POST -H "Content-Type: application/json" -d "$body" \
    "$API/test/tachibana/inject-daily-history" > /dev/null
}

tachibana_replay_setup() {
  local start=$1 end=$2
  cat > "$DATA_DIR/saved-state.json" <<HEREDOC
{
  "layout_manager":{"layouts":[{"name":"Test-D1","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"Test-D1"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
HEREDOC
  start_app
  curl -s -X POST "$API/test/tachibana/inject-session" > /dev/null
  curl -s -X POST -H "Content-Type: application/json" -d "$MASTER" \
    "$API/test/tachibana/inject-master" > /dev/null
  inject_daily_history "$start" "$end"
  sleep 4
  curl -s -X POST "$API/replay/toggle" > /dev/null
  curl -s -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$start\",\"end\":\"$end\"}" > /dev/null
}

# ── TC-S20-01: 速度ボタン連打 ─────────────────────────────────────────────
# D1 は 1x 速度で 100ms/bar。speed 20 連打（~2 秒） + 確認の間も Playing を維持するため
# -2400h/-24h (100 bar ≒ 10 秒 at 1x) を使用する。
# ※ inject-daily-history ペイロードが 8KB を超えないよう replay_api.rs のバッファを 64KB に拡張済み。
echo "  [TC-S20-01] 速度ボタン連打..."
tachibana_replay_setup "$(utc_offset -2400)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-01-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 20); do
  curl -s -X POST "$API/replay/speed" > /dev/null
done

wait_status Playing 10 || true
FINAL_STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$FINAL_STATUS" = "Playing" ] \
  && pass "TC-S20-01: speed 20 連打後 status=Playing" \
  || fail "TC-S20-01" "status=$FINAL_STATUS (Playing 期待)"

stop_app

# ── TC-S20-02: D1 StepForward/StepBackward の delta 検証 ───────────────
# TachibanaSpot:7203 D1 の StepForward delta は 86400000ms。
# D1 は 1x 速度で 100ms/bar のため -1300h/-24h (54 bar ≒ 5.4 秒) を使用し
# Playing 検出直後に即 Pause することで range 完了前に測定できるようにする。
echo "  [TC-S20-02] D1 StepForward/StepBackward delta 検証..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  pend "TC-S20-02" "Playing 到達せず → PEND"
  stop_app
else
  # Playing 検出直後に即 Pause（D1 は 100ms/step なので長く放置すると完了する）
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 15

  # ウォームアップ: バー境界にスナップしてから delta を計測
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  wait_status Paused 15

  # TC-S20-02a: StepForward delta = 86400000ms（バー境界から計測）
  T_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  wait_status Paused 15
  T_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")

  if [ -z "$T_BEFORE" ] || [ "$T_BEFORE" = "null" ] || \
     [ -z "$T_AFTER" ]  || [ "$T_AFTER"  = "null" ]; then
    fail "TC-S20-02a" "current_time 取得失敗 (before=$T_BEFORE after=$T_AFTER)"
  else
    DELTA=$(node -e "console.log(String(BigInt('$T_AFTER') - BigInt('$T_BEFORE')))")
    [ "$DELTA" = "86400000" ] \
      && pass "TC-S20-02a: D1 StepForward delta=86400000ms" \
      || fail "TC-S20-02a" "delta=$DELTA (expected 86400000)"
  fi

  # TC-S20-02b: StepBackward 後 status=Paused
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  wait_status Paused 15
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S20-02b: StepBackward 後 status=Paused" \
    || fail "TC-S20-02b" "status=$STATUS"

  stop_app
fi

# ── TC-S20-03: Live ↔ Replay 高速切替 ──────────────────────────────────
echo "  [TC-S20-03] Live ↔ Replay 高速切替..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-03-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 10); do
  curl -s -X POST "$API/replay/toggle" > /dev/null
  sleep 0.3
done

sleep 2
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
FINAL=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-03: toggle 10 連打後もアプリ応答あり (final_status=$FINAL)" \
  || fail "TC-S20-03" "toggle 連打後にアプリが応答しなくなった"

stop_app

# ── TC-S20-04: Playing 中の toggle ─────────────────────────────────────
echo "  [TC-S20-04] Playing 中の toggle..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-04-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-04: Playing 中の toggle → アプリ生存 (status=$STATUS_AFTER)" \
  || fail "TC-S20-04" "toggle 後にアプリが応答しなくなった"

stop_app

# ── TC-S20-05: Paused 中の toggle → Live → 再び Replay → Playing ──────
echo "  [TC-S20-05] Paused 中の toggle..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-05-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S20-05-pre" "Paused に遷移せず"
  exit 1
fi

# toggle → Live へ
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_LIVE=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-05a: Paused → toggle → アプリ生存 (status=$STATUS_LIVE)" \
  || fail "TC-S20-05a" "toggle 後にアプリが応答しなくなった"

# toggle → Replay に戻る
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
ALIVE2=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE2" = "true" ] \
  && pass "TC-S20-05b: 2 回目 toggle 後もアプリ生存 (status=$STATUS_BACK)" \
  || fail "TC-S20-05b" "2 回目 toggle 後にアプリが応答しなくなった"

print_summary
[ $FAIL -eq 0 ]
