#!/usr/bin/env bash
# s21_tachibana_error_boundary.sh — スイート S21: クラッシュ・エラー境界テスト（TachibanaSpot）
# TachibanaSpot:7203 D1 でアプリがクラッシュせず、エラーが適切に処理されることを確認する
# ビルド要件: cargo build（デバッグビルド）
# 前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "ERROR: DEV_USER_ID/DEV_PASSWORD not set" && exit 1
fi

echo "=== S21: クラッシュ・エラー境界テスト（TachibanaSpot:7203 D1）==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

FAKE_UUID="ffffffff-ffff-ffff-ffff-ffffffffffff"

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
  # DEV AUTO-LOGIN で Tachibana セッションが確立されるまで待機
  echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
  if ! wait_tachibana_session 120; then
    echo "  ERROR: Tachibana session not established after 120s"
    return 1
  fi
  echo "  Tachibana session established"
  # ペインの D1 kline データがフェッチ完了するまで待機
  local pane_id
  pane_id=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" \
    "$(curl -s "$API/pane/list")")
  if [ -n "$pane_id" ]; then
    echo "  waiting for D1 klines (streams_ready)..."
    wait_for_streams_ready "$pane_id" 120 || echo "  WARN: streams_ready timeout (continuing)"
  fi
  curl -s -X POST "$API/replay/toggle" > /dev/null
  curl -s -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$start\",\"end\":\"$end\"}" > /dev/null
}

# ── TC-S21-01〜03: 不正 pane_id に対する各エンドポイント ────────────────
echo "  [TC-S21-01/03] 不正 pane_id テスト..."
tachibana_replay_setup "$(utc_offset -96)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S21-precond" "Playing 到達せず"
  exit 1
fi

# TC-S21-01: pane/split に不正 UUID
HTTP_SPLIT=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"axis\":\"Vertical\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_SPLIT" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S21-01: pane/split 不正 UUID → HTTP=$HTTP_SPLIT & アプリ生存" \
  || fail "TC-S21-01" "HTTP=$HTTP_SPLIT alive=$ALIVE"

# TC-S21-02: pane/close に不正 UUID
HTTP_CLOSE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/close" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_CLOSE" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S21-02: pane/close 不正 UUID → HTTP=$HTTP_CLOSE & アプリ生存" \
  || fail "TC-S21-02" "HTTP=$HTTP_CLOSE alive=$ALIVE"

# TC-S21-03: pane/set-ticker に不正 UUID（別の Tachibana 銘柄）
HTTP_TICKER=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$FAKE_UUID\",\"ticker\":\"TachibanaSpot:6758\"}")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HTTP_TICKER" = "200" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S21-03: pane/set-ticker 不正 UUID → HTTP=$HTTP_TICKER & アプリ生存" \
  || fail "TC-S21-03" "HTTP=$HTTP_TICKER alive=$ALIVE"

stop_app

# ── TC-S21-04: 空 range (start == end) ─────────────────────────────────
echo "  [TC-S21-04] 空 range (start == end)..."
SAME_TIME=$(utc_offset -24)
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
echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
if ! wait_tachibana_session 120; then
  fail "TC-S21-04-pre" "Tachibana セッション確立せず（120 秒タイムアウト）"
  exit 1
fi
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$SAME_TIME\",\"end\":\"$SAME_TIME\"}" > /dev/null

sleep 5
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S21-04: 空 range でもアプリ生存 (status=$STATUS)" \
  || fail "TC-S21-04" "空 range でアプリがクラッシュした"

stop_app

# ── TC-S21-05: 未来の range (現在時刻 + 24h 先) ─────────────────────────
echo "  [TC-S21-05] 未来 range テスト..."
FUTURE_START=$(utc_offset 24)
FUTURE_END=$(utc_offset 48)
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
echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
if ! wait_tachibana_session 120; then
  fail "TC-S21-05-pre" "Tachibana セッション確立せず（120 秒タイムアウト）"
  exit 1
fi
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$FUTURE_START\",\"end\":\"$FUTURE_END\"}" > /dev/null

sleep 10
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S21-05: 未来 range でもアプリ生存 (status=$STATUS)" \
  || fail "TC-S21-05" "未来 range でアプリがクラッシュした"

stop_app

# ── TC-S21-06: StepForward 連打 50 回 (Paused 状態) ─────────────────────
echo "  [TC-S21-06] StepForward 連打 50 回..."
tachibana_replay_setup "$(utc_offset -96)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S21-06-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S21-06-pre" "Paused に遷移せず"
  exit 1
fi

CRASH=false
for i in $(seq 1 50); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    break
  fi
done
wait_status Paused 15 || true
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")

if ! $CRASH && [ "$ALIVE" = "true" ] && [ "$STATUS" = "Paused" ]; then
  pass "TC-S21-06: StepForward 50 連打 → crash なし, status=Paused"
else
  fail "TC-S21-06" "crash=$CRASH alive=$ALIVE status=$STATUS"
fi

stop_app

# ── TC-S21-07: split 上限テスト ──────────────────────────────────────────
echo "  [TC-S21-07] split 上限テスト..."
tachibana_replay_setup "$(utc_offset -96)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S21-07-pre" "Playing 到達せず"
  exit 1
fi

SPLIT_COUNT=0
LAST_HTTP=""
for i in $(seq 1 10); do
  PANES=$(curl -s "$API/pane/list")
  FIRST_PANE=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
  if [ -z "$FIRST_PANE" ]; then
    break
  fi
  LAST_HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$FIRST_PANE\",\"axis\":\"Vertical\"}")
  SPLIT_COUNT=$((SPLIT_COUNT + 1))
  sleep 0.5
  if [ "$LAST_HTTP" != "200" ]; then
    break
  fi
done

ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
PANE_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
  "$(curl -s "$API/pane/list")")
[ "$ALIVE" = "true" ] \
  && pass "TC-S21-07: split $SPLIT_COUNT 回後 (HTTP=$LAST_HTTP) クラッシュなし (panes=$PANE_COUNT)" \
  || fail "TC-S21-07" "split 繰り返し後にアプリがクラッシュした"

print_summary
[ $FAIL -eq 0 ]
