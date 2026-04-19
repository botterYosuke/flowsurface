#!/usr/bin/env bash
# s5_tachibana_mixed.sh — スイート S5: 立花証券 + Binance 混在 Replay
#
# 検証シナリオ:
#   TC-S5-*: inject-master + inject-daily-history でモックデータ注入 → Playing 到達
#   TC-S5-07: M1+D1 混在 step_size = M1 = 60000ms（D1 は最小 TF でない）
#
# 仕様根拠:
#   docs/replay_header.md §7 — マルチストリーム同期
#   e2e-mock feature — inject エンドポイント（inject-master / inject-daily-history）
#
# フィクスチャ: TachibanaSpot:7203 D1 + BinanceLinear:BTCUSDT M1（2ペイン）
#   ビルド: cargo build --release --features e2e-mock
#   注: inject-master は {"records":[{Tachibana フィールド名}]} 形式
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S5: 立花証券 + Binance 混在 Replay ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -4)
END=$(utc_offset -2)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e   "console.log(new Date('${END}:00Z').getTime())")
# 3 時間前のタイムスタンプ（range 内の D1 モック kline に使用）
MID_MS=$(node -e "console.log(Date.now() - 3*3600*1000)")

# Live モードで起動（auto-play 問題を回避するため replay 設定なし）
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

# inject-session が 404 の場合は e2e-mock feature なし → PEND
_probe=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/test/tachibana/inject-session" 2>/dev/null || echo "000")
if [ "$_probe" = "404" ]; then
  echo "  PEND: inject-session エンドポイント未実装（HTTP 404）— e2e-mock feature が必要"
  exit 0
fi

# TC-S5-01: Tachibana セッション注入 → session=present
# 上記プローブで inject-session は既に呼ばれているため、ステータスのみ確認
STATUS=$(curl -s "$API/auth/tachibana/status")
SESSION=$(jqn "$STATUS" "d.session")
[ "$SESSION" = "present" ] \
  && pass "TC-S5-01: Tachibana セッション注入成功 (session=present)" \
  || fail "TC-S5-01" "session=$SESSION (expected present)"

# TC-S5-02: 銘柄マスター注入（Tachibana フィールド形式）
MASTER=$(cat <<'MEOF'
{"records":[{"sIssueCode":"7203","sIssueNameEizi":"Toyota Motor","sCLMID":"CLMIssueMstKabu"}]}
MEOF
)
RES=$(curl -s -X POST "$API/test/tachibana/inject-master" \
  -H "Content-Type: application/json" -d "$MASTER")
M_OK=$(jqn "$RES" "d.ok")
HAS_NOT_FOUND=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.error&&d.error.includes('Not Found')?'true':'false');}catch(e){console.log('false');}" "$RES")
if [ "$M_OK" = "true" ]; then
  pass "TC-S5-02: inject-master 成功 (ok=true)"
elif [ "$HAS_NOT_FOUND" = "true" ]; then
  pend "TC-S5-02" "inject-master エンドポイント未実装（404）— e2e-mock feature が必要"
else
  fail "TC-S5-02" "inject-master 失敗: $RES"
fi

# inject-daily-history: replay 範囲内のモック D1 kline を注入
DAILY_BODY=$(node -e "
  const t = $MID_MS;
  console.log(JSON.stringify({
    issue_code: '7203',
    klines: [
      {time: t - 86400000, open: 3000, high: 3100, low: 2900, close: 3050, volume: 500000},
      {time: t,            open: 3050, high: 3150, low: 2950, close: 3100, volume: 600000}
    ]
  }));
")
DH_RES=$(curl -s -X POST "$API/test/tachibana/inject-daily-history" \
  -H "Content-Type: application/json" -d "$DAILY_BODY")
DH_OK=$(jqn "$DH_RES" "d.ok")
[ "$DH_OK" = "true" ] \
  && echo "  inject-daily-history OK (count=$(jqn "$DH_RES" "d.count"))" \
  || echo "  WARN: inject-daily-history: $DH_RES"

# Binance ストリームが Ready になるまで待つ（prepare_replay が M1 stream を登録できるように）
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  BTC_RDY=$(node -e "try{const ps=(JSON.parse(process.argv[1]).panes||[]);const p=ps.find(x=>x.ticker&&x.ticker.includes('BTCUSDT'));process.stdout.write(p&&p.streams_ready?'true':'false');}catch(e){process.stdout.write('false');}" "$PLIST")
  [ "$BTC_RDY" = "true" ] && echo "  BTC stream ready (${i}s)" && break
  sleep 1
done

# TC-S5-03: Replay に切替 + Manual Play → Playing 到達
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 60; then
  pass "TC-S5-03: Replay Playing 到達（Binance M1 + Tachibana D1 混在）"
else
  fail "TC-S5-03" "Playing に到達せず（60 秒タイムアウト）"
  exit 1
fi

# TC-S5-04/05: 両ペインの streams_ready 確認
# 少し待って streams を解決させる
for i in $(seq 1 20); do
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
  sleep 1
done

[ "$BTC_READY" = "true" ] \
  && pass "TC-S5-04: Binance BTCUSDT streams_ready=true" \
  || fail "TC-S5-04" "Binance streams_ready=$BTC_READY"
[ "$TACH_READY" = "true" ] \
  && pass "TC-S5-05: Tachibana 7203 streams_ready=true" \
  || fail "TC-S5-05" "Tachibana streams_ready=$TACH_READY"

# TC-S5-06: 10x 速度で current_time 前進をポーリング確認
speed_to_10x
T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if T2=$(wait_for_time_advance "$T1" 15); then
  pass "TC-S5-06: current_time 前進 ($T1 → $T2)"
else
  fail "TC-S5-06" "15 秒待機しても current_time が変化しなかった"
fi

# TC-S5-07: M1+D1 混在での StepForward
# 最小 TF は M1(60000ms)。D1 は最小 TF ではないので delta = 60000ms
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S5-07-pre" "Paused に遷移せず"
else
  T_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 1
  wait_status Paused 10 || true
  T_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  if [ -z "$T_BEFORE" ] || [ "$T_BEFORE" = "null" ] || \
     [ -z "$T_AFTER" ]  || [ "$T_AFTER"  = "null" ]; then
    fail "TC-S5-07" "current_time 取得失敗 (before=$T_BEFORE after=$T_AFTER)"
  else
    DELTA=$(node -e "console.log(String(BigInt('$T_AFTER') - BigInt('$T_BEFORE')))")
    # M1+D1 混在 → step_size = min(M1, D1) = M1 = 60000ms
    [ "$DELTA" = "60000" ] \
      && pass "TC-S5-07: M1+D1 混在 StepForward delta=60000ms（M1 が最小 TF）" \
      || fail "TC-S5-07" "delta=$DELTA (expected 60000ms: M1 is min TF in M1+D1 mix)"
  fi
fi

print_summary
[ $FAIL -eq 0 ]
