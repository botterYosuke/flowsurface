#!/usr/bin/env bash
# s31_replay_end_restart.sh — S31: 混合データ（Tachibana D1 + ETHUSDT M1）終端到達後 ▶ で先頭から再スタート
# ビルド要件: cargo build（debug ビルド）— inject-* エンドポイントは debug_assertions でのみ有効
#
# 修正された不具合:
#   ▶ ボタンが終端（now_ms >= range.end）で Paused のとき Resume ではなく
#   Play（先頭からの再スタート）を送るべきだったが、修正前は Resume を送っていた。
#   → 終端到達後に ▶ を押しても current_time が end_time のまま動かなかった。
#
#   API テストでは /api/replay/play を再呼び出しすることで同じコードパスを検証する。
#   混合データ（Tachibana D1 + ETHUSDT M1）でも再スタートが正常に動作することを確認する。
#
# 検証シナリオ:
#   TC-A: Play → 10x 加速 → 終端到達 (Paused @ end_time)
#   TC-B: Play 再呼び出し → レスポンスが Loading かつ再スタート開始
#   TC-C: Play レスポンスの current_time が start_time 付近であること（先頭からの再開）
#
# フィクスチャ: TachibanaSpot:7203 D1 + BinanceLinear:ETHUSDT M1（2 ペイン構成）
#              Tachibana 日次履歴を inject して D1 klines が存在する状態
#
# 前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数が設定済みであること
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S31: 混合データ 終端到達後 ▶ で先頭から再スタート ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── レンジ計算（過去 4h〜過去 2h、UTC）─────────────────────────────────────
START=$(utc_offset -4)
END=$(utc_offset -2)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e   "console.log(new Date('${END}:00Z').getTime())")
MID_MS=$(node -e   "console.log(Date.now() - 3*3600*1000)")

echo "  range: $START → $END"
echo "  start_ms=$START_MS end_ms=$END_MS"

# ── フィクスチャ（Live モード起動）──────────────────────────────────────────
# Auto-play を避けるため Live モードで起動し、セッション注入後に手動 Play する
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S31","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
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
  },"popout":[]}}],"active_layout":"S31"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana セッション不要環境ではスキップ"
  exit 0
fi

start_app

# ── Tachibana セッション確認 ─────────────────────────────────────────────────
AUTH=$(curl -s "$API/auth/tachibana/status" 2>/dev/null || echo '{}')
SESSION=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.session||'none');}catch(e){console.log('none');}" "$AUTH")
echo "  Tachibana session: $SESSION"

if [ "$SESSION" = "none" ]; then
  echo "  SKIP: Tachibana セッションなし — このテストはキーリングのセッションが必要です"
  print_summary
  exit 0
fi

# ── ETHUSDT M1 の streams_ready を待機 ────────────────────────────────────────
echo "  ETHUSDT M1 stream ready 待機（最大 30 秒）..."
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  ETH_RDY=$(node -e "
    try{const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.ticker&&x.ticker.includes('ETHUSDT'));
    process.stdout.write(p&&p.streams_ready?'true':'false');}catch(e){process.stdout.write('false');}
  " "$PLIST")
  [ "$ETH_RDY" = "true" ] && echo "  ETHUSDT M1 stream ready (${i}s)" && break
  sleep 1
done

# Replay に切替 + Play
curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

# Playing 到達待機（最大 60 秒）
echo "  Playing 待機..."
if ! wait_status "Playing" 60; then
  diagnose_playing_failure
  fail "precond" "Playing に到達せず"
  print_summary
  exit 1
fi
echo "  Playing 到達"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: 10x 加速 → 終端到達確認
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: 10x 加速 → 終端到達（Paused @ end_time）"

speed_to_10x
echo "  10x 加速完了、終端まで待機（最大 120 秒）..."

REACHED_END="false"
CT_AT_END=""
for i in $(seq 1 120); do
  STATUS_JSON=$(curl -s "$API/replay/status")
  CT=$(jqn "$STATUS_JSON" "d.current_time")
  ST=$(jqn "$STATUS_JSON" "d.status")
  if [ "$ST" = "Paused" ] && [ "$CT" != "null" ] && [ -n "$CT" ]; then
    NEAR_END=$(node -e "console.log(BigInt('$CT') >= BigInt('$END_MS') - BigInt('120000'))")
    if [ "$NEAR_END" = "true" ]; then
      REACHED_END="true"
      CT_AT_END="$CT"
      break
    fi
  fi
  sleep 1
done

if [ "$REACHED_END" = "true" ]; then
  pass "TC-A: 終端到達確認 (current_time=$CT_AT_END, status=Paused)"
else
  LAST_STATUS=$(curl -s "$API/replay/status")
  fail "TC-A: 終端到達しなかった" \
    "status=$(jqn "$LAST_STATUS" "d.status") current_time=$(jqn "$LAST_STATUS" "d.current_time")"
  print_summary
  exit 1
fi

# current_time が start_time とは異なることを確認（テストの前提確認）
DIFFERS=$(node -e "console.log(BigInt('$CT_AT_END') !== BigInt('$START_MS'))")
if [ "$DIFFERS" != "true" ]; then
  echo "  [SKIP] current_time が既に start_time と一致 — レンジが小さすぎてテスト不成立"
  print_summary
  exit 0
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Play 再呼び出し → レスポンスが Loading かつ再スタート開始
#
# 注意: 10x 加速状態では Loading→Playing→Paused が 1 秒以内に完了するため、
#       sleep 後のポーリングでは状態を捕捉できない。
#       Play API レスポンス（初期 Loading 状態を直接返す）を検証する。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: 終端到達後 Play 再呼び出し → レスポンスで再スタートを確認"

PLAY_RESP=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}")
echo "  play response: $PLAY_RESP"

RESP_STATUS=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.status||'none');}catch(e){console.log('parse_error');}" "$PLAY_RESP")
RESP_CT=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.current_time!=null?String(d.current_time):'null');}catch(e){console.log('null');}" "$PLAY_RESP")

if [ "$RESP_STATUS" = "Loading" ] || [ "$RESP_STATUS" = "Playing" ]; then
  pass "TC-B: 終端後 Play レスポンス status=$RESP_STATUS（再スタート開始）"
else
  fail "TC-B" "play レスポンス status=$RESP_STATUS (expected Loading or Playing) — $PLAY_RESP"
  print_summary
  exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Play レスポンスの current_time が start_time 付近であること（先頭から再スタート）
#
# Play レスポンスには Loading 状態の初期 current_time（= start_time）が含まれる。
# end_time 付近のままなら「修正前の挙動（Resume を送っていた）」が残っている。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: Play レスポンスの current_time が start_time 付近か確認"
echo "  resp.current_time=$RESP_CT start_time=$START_MS end_time=$END_MS"

if [ "$RESP_CT" = "null" ] || [ -z "$RESP_CT" ]; then
  fail "TC-C" "play レスポンスに current_time がない — $PLAY_RESP"
else
  # start_time から 5 分（300000ms）以内 かつ end_time から 1 分以上離れていること
  IS_NEAR_START=$(node -e "
    const ct = BigInt('$RESP_CT');
    const st = BigInt('$START_MS');
    const et = BigInt('$END_MS');
    const nearStart = ct >= st && ct <= st + BigInt('300000');
    const farFromEnd = et - ct > BigInt('60000');
    console.log(nearStart && farFromEnd ? 'true' : 'false');
  ")
  if [ "$IS_NEAR_START" = "true" ]; then
    pass "TC-C: 再スタート後 current_time が start_time 付近 (ct=$RESP_CT st=$START_MS) — 先頭から再開を確認"
  else
    fail "TC-C" \
      "current_time=$RESP_CT は start_time=$START_MS 付近でない — end_time=$END_MS 付近のまま？ (修正前の挙動)"
  fi
fi

print_summary
