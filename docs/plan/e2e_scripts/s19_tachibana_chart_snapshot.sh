#!/usr/bin/env bash
# s19_tachibana_chart_snapshot.sh — スイート S19: chart-snapshot API テスト（TachibanaSpot）
# TachibanaSpot:7203 D1 を使った chart-snapshot API の動作確認
# ビルド要件: cargo build --release --features e2e-mock
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S19: chart-snapshot API テスト（TachibanaSpot:7203 D1）==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

MASTER='{"records":[{"sIssueCode":"7203","sIssueNameEizi":"Toyota Motor","sCLMID":"CLMIssueMstKabu"}]}'

# inject_daily_history: start/end の範囲内に日次アライン D1 kline を注入する
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
    // 注入するバーが 0 本なら range 外なので前日分も追加
    if (klines.length === 0) {
      klines.push({time: first - day, open: 3000, high: 3100, low: 2900, close: 3050, volume: 500000});
      klines.push({time: first,       open: 3050, high: 3150, low: 2950, close: 3100, volume: 600000});
    }
    console.log(JSON.stringify({issue_code: '7203', klines}));
  " "$start" "$end")
  curl -s -X POST -H "Content-Type: application/json" -d "$body" \
    "$API/test/tachibana/inject-daily-history" > /dev/null
}

# tachibana_replay_setup: Live モードで起動 → inject → streams_ready 待機 → replay 開始
# streams_ready が true になってから toggle+play することで step_size_ms が D1 に設定される。
# inject-master 直後は UpdateMetadata タスクが非同期で実行されるため 2〜3 秒待ちが必要。
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
  # inject-master 後に UpdateMetadata タスクが非同期実行され ticker_info が解決されるまで
  # 約 2 秒かかる（[e2e-live] has_ticker_info=false → true のログ参照）。
  # 解決前に toggle+play すると prepare_replay() の ready_iter() が空を返し
  # step_size_ms が fallback の 6000ms になってしまう。
  sleep 4
  curl -s -X POST "$API/replay/toggle" > /dev/null
  curl -s -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$start\",\"end\":\"$end\"}" > /dev/null
}

START=$(utc_offset -96)
END=$(utc_offset -24)
tachibana_replay_setup "$START" "$END"

if ! wait_playing 60; then
  fail "TC-S19-precond" "Playing 到達せず（60 秒タイムアウト）"
  exit 1
fi

# 前提確認: chart-snapshot API が実装済みか確認（未実装なら全 TC PENDING）
PROBE=$(curl -s -o /dev/null -w "%{http_code}" "$API/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000000" || echo "000")
if [ "$PROBE" = "404" ]; then
  pend "TC-S19-*" "GET /api/pane/chart-snapshot 未実装 → S19 全 TC を PENDING"
  print_summary
  exit 0
fi
echo "  chart-snapshot API 確認 (probe=$PROBE)"

# ペイン ID を取得して streams_ready を待機してから Pause
PANES=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE_ID" ]; then
  fail "TC-S19-precond" "ペイン ID 取得失敗"
  exit 1
fi
echo "  PANE_ID=$PANE_ID"

# streams_ready=true になるまで待機（最大 60 秒）
echo "  waiting for streams_ready..."
if ! wait_for_streams_ready "$PANE_ID" 60; then
  echo "  WARN: streams_ready タイムアウト（継続）"
fi

# 少し再生させてから Pause（バーが蓄積されるのを待つ）
sleep 2
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S19-precond" "Paused に遷移せず"
  exit 1
fi
sleep 0.5

# TC-S19-01: Paused 直後のバー本数が 1 ≤ bar_count ≤ 301
SNAP=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  snapshot response: $SNAP"
BAR_COUNT=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP")
echo "  bar_count=$BAR_COUNT"
if node -e "
  const n = Number(process.argv[1]);
  process.exit((Number.isFinite(n) && n >= 1 && n <= 301) ? 0 : 1);
" "$BAR_COUNT" 2>/dev/null; then
  pass "TC-S19-01: Play 後 bar_count=$BAR_COUNT (1 ≤ N ≤ 301, PRE_START_HISTORY_BARS 確認)"
else
  fail "TC-S19-01" "bar_count=$BAR_COUNT (想定: 1..301)"
fi

# TC-S19-02: StepForward 後 bar_count が増加または同数（D1 step = 86400000ms）
BAR_BEFORE="$BAR_COUNT"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 15
sleep 0.5
SNAP2=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
BAR_AFTER=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP2")
echo "  bar_count after StepForward: $BAR_BEFORE → $BAR_AFTER"
if node -e "process.exit(Number(process.argv[1]) >= Number(process.argv[2]) ? 0 : 1);" \
     "$BAR_AFTER" "$BAR_BEFORE" 2>/dev/null; then
  pass "TC-S19-02: StepForward 後 bar_count=$BAR_AFTER >= before=$BAR_BEFORE"
else
  fail "TC-S19-02" "bar_count=$BAR_AFTER < before=$BAR_BEFORE（バー減少の異常）"
fi

# TC-S19-03: StepBackward 後も snapshot 取得可能（クラッシュしない）
# 少し前進してから StepBackward（start 境界クランプを避けるため）
for i in $(seq 1 3); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.5
done
wait_status Paused 15 || true
curl -s -X POST "$API/replay/step-backward" > /dev/null
wait_status Paused 15
sleep 0.3
SNAP3=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
HAS_BAR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.bar_count !== undefined && !d.error ? 'true' : 'false');
" "$SNAP3")
BAR3=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count);" "$SNAP3")
[ "$HAS_BAR" = "true" ] \
  && pass "TC-S19-03: StepBackward 後 snapshot 取得成功 (bar_count=$BAR3)" \
  || fail "TC-S19-03" "snapshot 異常レスポンス: $SNAP3"

# TC-S19-04: 存在しないペイン ID に対する snapshot → {"error":"..."} かつクラッシュなし
FAKE_ID="00000000-0000-0000-0000-deadbeef0000"
SNAP_FAKE=$(curl -s "$API/pane/chart-snapshot?pane_id=$FAKE_ID")
HAS_ERROR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.error ? 'true' : 'false');
" "$SNAP_FAKE")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HAS_ERROR" = "true" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S19-04: 不正 pane_id → error 応答 & アプリ生存確認 (resp=$SNAP_FAKE)" \
  || fail "TC-S19-04" "has_error=$HAS_ERROR alive=$ALIVE resp=$SNAP_FAKE"

# TC-S19-05: Live モードで snapshot を取得してもクラッシュしない
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
SNAP_LIVE=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  Live mode snapshot: $SNAP_LIVE"
if curl -s "$API/replay/status" > /dev/null 2>&1; then
  pass "TC-S19-05: Live モード中の snapshot 取得後もアプリ応答あり"
else
  fail "TC-S19-05" "Live モード中の snapshot 取得後にアプリが応答しなくなった"
fi
# Replay モードに戻す
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2

print_summary
[ $FAIL -eq 0 ]
