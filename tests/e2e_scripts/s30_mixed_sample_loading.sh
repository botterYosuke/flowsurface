#!/usr/bin/env bash
# s30_mixed_sample_loading.sh — S30: Tachibana D1 + ETHUSDT M1 混在起動時の Loading 解消
# ビルド要件: cargo build（debug ビルド）— inject-* エンドポイントは debug_assertions でのみ有効
#
# 修正された不具合 (2 件):
#   (1) Play 時に D1 ストリームの load_range が M1 の step_size で計算されていたため
#       D1 klines が range 外になり空で返っていた → compute_load_range を各 TF で計算するよう修正
#   (2) KlinesLoadCompleted で空 klines が返ったとき on_klines_loaded を呼ばず
#       ストリームがロード済みになれなかった → status が "Loading" に固定された
#
# 検証シナリオ:
#   TC-A: Tachibana D1 + ETHUSDT M1 の 2 ペイン構成で Play → Playing に遷移すること
#         （修正前: D1 ストリームが load_range 不正により Loading に固定されていた）
#   TC-B: Playing 後 current_time が前進すること（再生が正常動作している）
#   TC-C: 両ペインの streams_ready=true になること
#
# 注意: inject-master / inject-daily-history エンドポイントは現在未実装のため、
#       実際の Tachibana セッション（keyring）を使用する。
#       セッションが存在しない環境では TC-A は SKIP される。
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S30: Tachibana D1 + ETHUSDT M1 混在起動時の Loading 解消 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── レンジ計算（過去 4h〜過去 2h、UTC）─────────────────────────────────────
START=$(utc_offset -4)
END=$(utc_offset -2)
MID_MS=$(node -e "console.log(Date.now() - 3*3600*1000)")
echo "  range: $START → $END"

# ── フィクスチャ（Live モード起動）──────────────────────────────────────────
# auto-play を避けるため Live モードで起動し、stream ready 確認後に手動 Play する
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S30","dashboard":{"pane":{
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
  },"popout":[]}}],"active_layout":"S30"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# ── Tachibana セッション確認 ─────────────────────────────────────────────────
AUTH=$(curl -s "$API/auth/tachibana/status" 2>/dev/null || echo '{}')
SESSION=$(node -e "try{const d=JSON.parse(process.argv[1]);console.log(d.session||'none');}catch(e){console.log('none');}" "$AUTH")
echo "  Tachibana session: $SESSION"

if [ "$SESSION" = "none" ]; then
  echo "  SKIP: Tachibana セッションなし — このテストはキーリングのセッションが必要です"
  echo "  (inject-session が利用できない環境では Tachibana ストリームはテストできません)"
  echo ""
  print_summary
  exit 0
fi

# ── ETHUSDT M1 の streams_ready を待機（Binance WebSocket 接続後に利用可能）──
# s5 パターン: stream が Ready になってから replay play を発火する
echo "  ETHUSDT M1 stream ready 待機（最大 30 秒）..."
ETH_PANE_READY="false"
for i in $(seq 1 30); do
  PLIST=$(curl -s "$API/pane/list" 2>/dev/null || echo '{}')
  ETH_RDY=$(node -e "
    try {
      const ps=(JSON.parse(process.argv[1]).panes||[]);
      const p=ps.find(x=>x.ticker&&x.ticker.includes('ETHUSDT'));
      process.stdout.write(p&&p.streams_ready?'true':'false');
    } catch(e){process.stdout.write('false');}
  " "$PLIST")
  if [ "$ETH_RDY" = "true" ]; then
    echo "  ETHUSDT M1 stream ready (${i}s)"
    ETH_PANE_READY="true"
    break
  fi
  sleep 1
done

if [ "$ETH_PANE_READY" != "true" ]; then
  echo "  WARN: ETHUSDT M1 stream が 30 秒で ready にならなかった — Live 接続待ちのまま続行"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Tachibana D1 + ETHUSDT M1 の 2 ペイン構成で Play → Playing に遷移
#
# 修正前: D1 ストリームの load_range が min_timeframe(=M1=60000ms) で計算されていたため、
#         300 * 60000 ms = 5 時間しか遡らず、D1 バーが範囲外になっていた。
#         → D1 klines が空で返り、ストリームがロード済みにならず Loading に固定。
# 修正後: D1 ストリームは D1(=86400000ms) で計算 = 300 日分遡る → D1 data が取得される。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: Tachibana D1 + ETHUSDT M1 混在 Play → Playing に遷移"

curl -s -X POST "$API/replay/toggle" > /dev/null
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

echo "  Play 開始、Loading → Playing を待機（最大 120 秒）..."

if wait_playing 120; then
  pass "TC-A: Loading が解消され Playing に遷移（D1 load_range 修正が有効）"
else
  LAST_ST=$(jqn "$(curl -s "$API/replay/status" 2>/dev/null || echo '{}')" "d.status")
  fail "TC-A: Playing に到達しなかった（120 秒タイムアウト）" \
    "status=$LAST_ST — 修正前: D1 stream が load_range 不正により Loading に固定される"
  print_summary
  exit 1
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: Playing 後 current_time が前進すること
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: current_time が前進すること（再生が正常動作）"

T1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
if T2=$(wait_for_time_advance "$T1" 15); then
  pass "TC-B: current_time が前進 ($T1 → $T2)"
else
  fail "TC-B" "15 秒待機しても current_time が変化しなかった"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: 両ペインの streams_ready=true 確認
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: 両ペインの streams_ready 確認"

for i in $(seq 1 20); do
  PANES=$(curl -s "$API/pane/list")
  TACH_READY=$(node -e "
    const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.ticker&&x.ticker.includes('7203'));
    console.log(p&&p.streams_ready?'true':'false');
  " "$PANES")
  ETH_READY=$(node -e "
    const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.ticker&&x.ticker.includes('ETHUSDT'));
    console.log(p&&p.streams_ready?'true':'false');
  " "$PANES")
  [ "$TACH_READY" = "true" ] && [ "$ETH_READY" = "true" ] && break
  sleep 1
done

[ "$TACH_READY" = "true" ] \
  && pass "TC-C1: Tachibana D1 (7203) streams_ready=true" \
  || fail "TC-C1" "Tachibana streams_ready=$TACH_READY"
[ "$ETH_READY" = "true" ] \
  && pass "TC-C2: BinanceLinear ETHUSDT M1 streams_ready=true" \
  || fail "TC-C2" "ETHUSDT streams_ready=$ETH_READY"

print_summary
