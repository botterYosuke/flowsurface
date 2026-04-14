#!/usr/bin/env bash
# s5_tachibana_mixed.sh — スイート S5: 立花証券 + Binance 混在 Replay
# ビルド要件: cargo build（debug）
# 前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "ERROR: DEV_USER_ID/DEV_PASSWORD not set" && exit 1
fi

echo "=== S5: 立花証券 + Binance 混在 Replay ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -4)
END=$(utc_offset -2)

# Live モードで起動（DEV AUTO-LOGIN が Tachibana セッションを確立）
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S5","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":[],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
        "indicators":[],"link_group":"A"
      }}
    }
  },"popout":[]}}],"active_layout":"S5"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# TC-S5-01: DEV AUTO-LOGIN で Tachibana セッションが確立されるまで待機
echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
if wait_tachibana_session 120; then
  pass "TC-S5-01: Tachibana セッション確立 (session=present)"
else
  fail "TC-S5-01" "Tachibana セッションが 120s 以内に確立されなかった"
  exit 1
fi

# TC-S5-02: Replay に切替 + Manual Play → Playing 到達
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 60; then
  pass "TC-S5-02: Replay Playing 到達（Binance M1 + Tachibana D1 混在）"
else
  fail "TC-S5-02" "Playing に到達せず（60 秒タイムアウト）"
  exit 1
fi

# TC-S5-03/04: 両ペインの streams_ready 確認
for i in $(seq 1 30); do
  PANES=$(curl -s "$API/pane/list")
  BTC_READY=$(node -e "
    const ps = (JSON.parse(process.argv[1]).panes || []);
    const p = ps.find(x => x.ticker && x.ticker.includes('BTCUSDT'));
    console.log(p && p.streams_ready ? 'true' : 'false');
  " "$PANES")
  TACH_READY=$(node -e "
    const ps = (JSON.parse(process.argv[1]).panes || []);
    const p = ps.find(x => x.ticker && x.ticker.includes('7203'));
    console.log(p && p.streams_ready ? 'true' : 'false');
  " "$PANES")
  [ "$BTC_READY" = "true" ] && [ "$TACH_READY" = "true" ] && break
  sleep 2
done

[ "$BTC_READY" = "true" ] \
  && pass "TC-S5-03: Binance BTCUSDT streams_ready=true" \
  || fail "TC-S5-03" "Binance streams_ready=$BTC_READY"
[ "$TACH_READY" = "true" ] \
  && pass "TC-S5-04: Tachibana 7203 streams_ready=true" \
  || fail "TC-S5-04" "Tachibana streams_ready=$TACH_READY"

# TC-S5-05: 10x 速度で current_time 前進をポーリング確認
speed_to_10x
T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if T2=$(wait_for_time_advance "$T1" 15); then
  pass "TC-S5-05: current_time 前進 ($T1 → $T2)"
else
  fail "TC-S5-05" "15 秒待機しても current_time が変化しなかった"
fi

# TC-S5-06: M1+D1 混在での StepForward
# 最小 TF は M1(60000ms)。D1 は最小 TF ではないので delta = 60000ms
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S5-06-pre" "Paused に遷移せず"
else
  T_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 1
  wait_status Paused 10 || true
  T_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if [ -z "$T_BEFORE" ] || [ "$T_BEFORE" = "null" ] || \
     [ -z "$T_AFTER" ]  || [ "$T_AFTER"  = "null" ]; then
    fail "TC-S5-06" "current_time 取得失敗 (before=$T_BEFORE after=$T_AFTER)"
  else
    DELTA=$(node -e "console.log(String(BigInt('$T_AFTER') - BigInt('$T_BEFORE')))")
    # M1+D1 混在 → step_size = min(M1, D1) = M1 = 60000ms
    [ "$DELTA" = "60000" ] \
      && pass "TC-S5-06: M1+D1 混在 StepForward delta=60000ms（M1 が最小 TF）" \
      || fail "TC-S5-06" "delta=$DELTA (expected 60000ms: M1 is min TF in M1+D1 mix)"
  fi
fi

print_summary
[ $FAIL -eq 0 ]
