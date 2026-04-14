#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

MASTER='{"records":[{"sIssueCode":"7203","sIssueNameEizi":"Toyota Motor","sCLMID":"CLMIssueMstKabu"}]}'

inject_daily_history() {
  local start=$1 end=$2
  local startMs endMs first
  startMs=$(node -e "console.log(new Date(process.argv[1].replace(' ','T')+':00Z').getTime());" "$start")
  endMs=$(node -e "console.log(new Date(process.argv[1].replace(' ','T')+':00Z').getTime());" "$end")
  first=$(node -e "const d=86400000; console.log(String(Math.ceil(Number(process.argv[1])/d)*d));" "$startMs")
  echo "  inject range: $start → $end (first_day_boundary=$(node -e "console.log(new Date(Number(process.argv[1])).toISOString());" "$first"))"
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
      klines.push({time: first, open: 3050, high: 3150, low: 2950, close: 3100, volume: 600000});
    }
    process.stderr.write(JSON.stringify({issue_code: '7203', klines}));
    console.log('  klines count: ' + klines.length + ' (first: ' + new Date(klines[0].time).toISOString() + ', last: ' + new Date(klines[klines.length-1].time).toISOString() + ')');
  " "$start" "$end" 2>/tmp/diag_inject_body.json)
  echo "$body"
  curl -s -X POST -H "Content-Type: application/json" -d "$(cat /tmp/diag_inject_body.json)" \
    "$API/test/tachibana/inject-daily-history"
}

stop_app
START=$(utc_offset -96); END=$(utc_offset -24)
echo "Replay range: $START → $END"

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
curl -s -X POST -H "Content-Type: application/json" -d "$MASTER" "$API/test/tachibana/inject-master" > /dev/null
inject_daily_history "$START" "$END"
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

echo "  Waiting for Playing..."
wait_playing 60

PANES=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
echo "  PANE_ID=$PANE_ID"

wait_for_streams_ready "$PANE_ID" 60 && echo "  streams_ready=true" || echo "  WARN: streams_ready timeout, continuing"

sleep 2
curl -s -X POST "$API/replay/pause" > /dev/null
wait_status Paused 15
STATUS0=$(curl -s "$API/replay/status")
CT0=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.current_time||'null');" "$STATUS0")
echo "  After pause: status=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.status||'null');" "$STATUS0") ct=$CT0 ($(node -e "try{console.log(new Date(Number('$CT0')).toISOString());}catch(e){console.log('?');}" 2>/dev/null))"

echo "  --- warmup step ---"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 15; sleep 0.3
STATUS1=$(curl -s "$API/replay/status")
CT1=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.current_time||'null');" "$STATUS1")
D01=$(node -e "if('$CT1'!='null'&&'$CT0'!='null') console.log(String(BigInt('$CT1')-BigInt('$CT0'))); else console.log('N/A');" 2>/dev/null)
echo "  ct=$CT1 ($(node -e "try{console.log(new Date(Number('$CT1')).toISOString());}catch(e){console.log('?');}" 2>/dev/null)) delta_warmup=$D01"

echo "  --- measure step 1 ---"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 15; sleep 0.3
STATUS2=$(curl -s "$API/replay/status")
CT2=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.current_time||'null');" "$STATUS2")
D12=$(node -e "if('$CT2'!='null'&&'$CT1'!='null') console.log(String(BigInt('$CT2')-BigInt('$CT1'))); else console.log('N/A');" 2>/dev/null)
echo "  ct=$CT2 ($(node -e "try{console.log(new Date(Number('$CT2')).toISOString());}catch(e){console.log('?');}" 2>/dev/null)) delta_step1=$D12"

echo "  --- measure step 2 ---"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 15; sleep 0.3
STATUS3=$(curl -s "$API/replay/status")
CT3=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.current_time||'null');" "$STATUS3")
D23=$(node -e "if('$CT3'!='null'&&'$CT2'!='null') console.log(String(BigInt('$CT3')-BigInt('$CT2'))); else console.log('N/A');" 2>/dev/null)
echo "  ct=$CT3 ($(node -e "try{console.log(new Date(Number('$CT3')).toISOString());}catch(e){console.log('?');}" 2>/dev/null)) delta_step2=$D23"

stop_app
