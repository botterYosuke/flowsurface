#!/bin/bash
# common_helpers.sh — 全 E2E テストスクリプトで source する共通ヘルパー
# Usage: source "$(dirname "$0")/common_helpers.sh"

set -e

DATA_DIR="$APPDATA/flowsurface"
API="http://127.0.0.1:9876/api"
API_BASE="http://127.0.0.1:9876"
PASS=0
FAIL=0
PEND=0
EXE="C:/Users/sasai/Documents/flowsurface/target/release/flowsurface.exe"

jqn() {
  node -e "const d=JSON.parse(process.argv[1]); const v=$2; console.log(v === null || v === undefined ? 'null' : v);" "$1"
}

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1 — $2"; FAIL=$((FAIL + 1)); }
pend() { echo "  PEND: $1 — $2 (API 拡張待ち)"; PEND=$((PEND + 1)); }

start_app() {
  echo "  Starting app..."
  > "$APPDATA/flowsurface/flowsurface-current.log" 2>/dev/null || true
  "$EXE" 2>C:/tmp/e2e_debug.log &
  APP_PID=$!
  for i in $(seq 1 30); do
    if curl -s "$API/replay/status" > /dev/null 2>&1; then
      echo "  API ready (${i}s)"
      return 0
    fi
    sleep 1
  done
  echo "  ERROR: API not ready after 30s"
  return 1
}

stop_app() {
  echo "  Stopping app..."
  taskkill //f //im flowsurface.exe > /dev/null 2>&1 || true
  sleep 2
}

backup_state() {
  cp "$DATA_DIR/saved-state.json" "$DATA_DIR/saved-state.json.bak" 2>/dev/null || true
}

restore_state() {
  stop_app
  [ -f "$DATA_DIR/saved-state.json.bak" ] && \
    cp "$DATA_DIR/saved-state.json.bak" "$DATA_DIR/saved-state.json" || true
}

# 日時ヘルパー（UTC）— node で実装（Windows Git Bash でも動作）
utc_offset() {
  node -e "
    const d = new Date(Date.now() + ($1) * 3600000);
    const pad = n => String(n).padStart(2, '0');
    console.log(
      d.getUTCFullYear() + '-' + pad(d.getUTCMonth()+1) + '-' + pad(d.getUTCDate()) +
      ' ' + pad(d.getUTCHours()) + ':' + pad(d.getUTCMinutes())
    );
  "
}

# BigInt 比較
bigt_gt() { node -e "console.log(BigInt('$1') > BigInt('$2'))"; }
bigt_ge() { node -e "console.log(BigInt('$1') >= BigInt('$2'))"; }
bigt_eq() { node -e "console.log(BigInt('$1') === BigInt('$2'))"; }
bigt_sub() { node -e "console.log(String(BigInt('$1') - BigInt('$2')))"; }

wait_playing() {
  local MAX=${1:-30}
  for i in $(seq 1 $MAX); do
    local ST
    ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$ST" = "Playing" ] && return 0
    sleep 1
  done
  return 1
}

wait_paused() {
  local MAX=${1:-15}
  for i in $(seq 1 $MAX); do
    local ST
    ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$ST" = "Paused" ] && return 0
    sleep 1
  done
  return 1
}

print_summary() {
  echo ""
  echo "============================="
  echo "  PASS: $PASS  FAIL: $FAIL  PEND: $PEND"
  echo "============================="
  [ $FAIL -eq 0 ]
}

# chart-snapshot を取得（要 API 拡張）
chart_snapshot() {
  curl -s "$API/pane/chart-snapshot?pane_id=$1"
}

# current_time_display を取得（要 API 拡張）
status_display() {
  jqn "$(curl -s "$API/replay/status")" "d.current_time_display"
}

# トースト一覧を取得
list_notifications() {
  curl -s "$API/notification/list"
}

# トーストに body 部分一致で検索
has_notification() {
  local needle=$1
  local n
  n=$(list_notifications)
  node -e "
    const d=JSON.parse(process.argv[1]);
    const items=d.notifications||[];
    const hit=items.some(t=>(t.body||'').includes(process.argv[2])||(t.title||'').includes(process.argv[2]));
    console.log(hit);
  " "$n" "$needle"
}

# ── S5〜S14 向け追加ヘルパー ──────────────────────────────────────────────

# API ラッパー（フルパス /api/... を受け取る）
api_get()      { curl -s "$API_BASE$1"; }
api_post()     {
  if [ -n "${2:-}" ]; then
    curl -s -X POST -H "Content-Type: application/json" -d "$2" "$API_BASE$1"
  else
    curl -s -X POST "$API_BASE$1"
  fi
}
# HTTP ステータスコードのみ返す POST ラッパー
api_post_code() {
  curl -s -o /dev/null -w "%{http_code}" \
    -X POST -H "Content-Type: application/json" \
    -d "${2:-{}}" "$API_BASE$1"
}

# status が want になるまでポーリング（最大 timeout 秒）
wait_status() {
  local want=$1 timeout=${2:-10}
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local s
    s=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$s" = "$want" ] && return 0
    sleep 0.5
  done
  return 1
}

# current_time が ref より大きくなるまでポーリング。成功時は新しい値を stdout へ出力
wait_for_time_advance() {
  local ref=$1 timeout=${2:-30}
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local t
    t=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    if [ -n "$t" ] && [ "$t" != "null" ] && \
       node -e "process.exit(BigInt('$t') > BigInt('$ref') ? 0 : 1)" 2>/dev/null; then
      echo "$t"; return 0
    fi
    sleep 0.5
  done
  return 1
}

# ペイン数が want になるまでポーリング（pane/list の .panes 配列長を確認）
wait_for_pane_count() {
  local want=$1 timeout=${2:-10}
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local c
    c=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
      "$(curl -s "$API/pane/list")")
    [ "$c" = "$want" ] && return 0
    sleep 0.5
  done
  return 1
}

# 指定ペインの streams_ready=true になるまでポーリング
wait_for_streams_ready() {
  local pane_id="$1" timeout=${2:-30}
  local end=$((SECONDS + timeout))
  while [ $SECONDS -lt $end ]; do
    local panes ready
    panes=$(curl -s "$API/pane/list")
    ready=$(node -e "
      const ps = (JSON.parse(process.argv[1]).panes || []);
      const p = ps.find(x => x.id === '$pane_id');
      console.log(p && p.streams_ready ? 'true' : 'false');
    " "$panes")
    [ "$ready" = "true" ] && return 0
    sleep 1
  done
  return 1
}

# 速度を 1x→10x に上げる（speed は 1x→2x→5x→10x→1x のサイクル）
speed_to_10x() {
  curl -s -X POST "$API/replay/speed" > /dev/null
  curl -s -X POST "$API/replay/speed" > /dev/null
  curl -s -X POST "$API/replay/speed" > /dev/null
}

# 単一ペイン saved-state.json を書き込む
# 引数: ticker timeframe start_utc end_utc
setup_single_pane() {
  local ticker=$1 timeframe=$2 start=$3 end=$4
  local name="Test-${timeframe}"
  cat > "$DATA_DIR/saved-state.json" <<HEREDOC
{
  "layout_manager":{"layouts":[{"name":"$name","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"$ticker","timeframe":"$timeframe"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"$timeframe"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"$name"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$start","range_end":"$end"}
}
HEREDOC
}

# ステップサイズ定数
STEP_M1=60000
STEP_M5=300000
STEP_H1=3600000
STEP_D1=86400000

# 前進差分が期待ステップ境界内に収まるか
advance_within() {
  local pre=$1 post=$2 step=$3 max_bars=$4
  node -e "
    const d = BigInt('$post') - BigInt('$pre');
    const s = BigInt('$step');
    const max = BigInt('$max_bars');
    if (d < 0n) { console.log('false'); process.exit(0); }
    if (d % s !== 0n) { console.log('false'); process.exit(0); }
    const bars = d / s;
    console.log(bars >= 1n && bars <= max ? 'true' : 'false');
  "
}

# current_time がバー境界値か
is_bar_boundary() {
  local ct=$1 step=$2
  node -e "console.log(BigInt('$ct') % BigInt('$step') === 0n)"
}

# current_time が [start_time, end_time] 区間内
ct_in_range() {
  local ct=$1 st=$2 et=$3
  node -e "console.log(BigInt('$ct') >= BigInt('$st') && BigInt('$ct') <= BigInt('$et'))"
}
