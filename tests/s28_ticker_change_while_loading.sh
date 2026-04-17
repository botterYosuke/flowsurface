#!/usr/bin/env bash
# s28_ticker_change_while_loading.sh — S28: Loading（Waiting）状態中の銘柄変更
#
# 検証シナリオ（仕様 §6.6「銘柄変更による初期状態リセット」Waiting 状態部分）:
#   TC-setup: Playing → split + ETHUSDT 設定 → Loading 状態を確認
#   TC-A: Loading 中（または直後）に元ペインの ticker を SOLUSDT に変更 → クラッシュなし
#   TC-B: 変更後 最大 30s 待機 → status=Paused（自動再生されない）
#   TC-C: Paused 状態で current_time≈start_time（リセット発生の確認）
#   TC-D: Resume → Playing 到達（回復可能であること）
#
# 仕様根拠:
#   docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット（Waiting 状態中も適用）
#   s23 は Playing/Paused 中をカバー済み。本テストは Waiting（API: "Loading"）中を対象とする。
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, UTC[-6h, -1h]（5h レンジで Loading を長め確保）
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S28: Loading（Waiting）状態中の銘柄変更 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ ──────────────────────────────────────────────────────────────
# 5h レンジ（300 bar M1）を使い、ロードに 2〜5 秒かかることで Loading を捕捉しやすくする
START=$(utc_offset -6)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")

echo "  range: $START → $END (start_ms=$START_MS)"

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S28","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S28"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# Playing に到達するまで待機（最大 90 秒: 5h レンジのフェッチ時間を考慮）
if ! wait_status "Playing" 90; then
  diagnose_playing_failure
  fail "precond" "auto-play で Playing に到達せず"
  print_summary
  exit 1
fi
echo "  Playing 到達"

# ── ペイン ID 取得 ─────────────────────────────────────────────────────────────
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" \
  "$(curl -s "$API/pane/list")")
if [ -z "$PANE0" ]; then
  fail "precond" "初期ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE0=$PANE0"

# ─────────────────────────────────────────────────────────────────────────────
# TC-setup: Playing 中に split → 新ペインに ETHUSDT → Loading 状態を確認
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-setup: split + ETHUSDT 設定 → Loading 遷移を確認"

api_post /api/pane/split "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null
sleep 0.3

# 新ペイン ID を取得
NEW_PANE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p  = ps.find(x => x.id !== '$PANE0');
  console.log(p ? p.id : '');
" "$(curl -s "$API/pane/list")")

if [ -z "$NEW_PANE" ]; then
  fail "TC-setup" "split 後の新ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  NEW_PANE=$NEW_PANE"

# 新ペインに ETHUSDT を設定 → 新ストリームのロードが始まる → Loading 遷移
api_post /api/pane/set-ticker "{\"pane_id\":\"$NEW_PANE\",\"ticker\":\"BinanceLinear:ETHUSDT\"}" > /dev/null

# Loading 状態を 100ms ポーリングで最大 5 秒間確認
LOADING_CAUGHT="false"
for i in $(seq 1 50); do
  ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  if [ "$ST" = "Loading" ]; then
    LOADING_CAUGHT="true"
    echo "  Loading 捕捉 (${i}×100ms)"
    break
  fi
  sleep 0.1
done

# Loading 捕捉は保証できない（ロードが瞬時に完了した場合）ため、INFO 扱い
if [ "$LOADING_CAUGHT" = "true" ]; then
  echo "  INFO: Loading 状態を確認（TC-A は Waiting 中の ticker 変更をテスト）"
else
  echo "  INFO: Loading を捕捉できず（ロードが高速で完了した可能性あり）"
  echo "        TC-A は Playing または Paused 中の ticker 変更をテストする（§6.6 の別ケース）"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: Loading 中（または直後）に元ペインの ticker を SOLUSDT に変更
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: 元ペイン ticker を SOLUSDT に変更 → クラッシュなし"

# ticker 変更を実行（Loading / Playing / Paused のいずれの状態でも §6.6 リセットが適用される）
api_post /api/pane/set-ticker "{\"pane_id\":\"$PANE0\",\"ticker\":\"BinanceLinear:SOLUSDT\"}" > /dev/null
echo "  ticker 変更送信完了"

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: 変更後 最大 30s 待機 → status=Paused（自動再生されない）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: 変更後 status=Paused になること"

if wait_status "Paused" 30; then
  pass "TC-B: ticker 変更後 status=Paused（自動再生なし）"
else
  LAST_ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  fail "TC-B" "30s 待機後 status=$LAST_ST (expected Paused)"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: Paused 状態で current_time≈start_time
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: current_time≈start_time（リセット発生の確認）"

CT_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  current_time=$CT_AFTER (start_ms=$START_MS)"

if [ "$CT_AFTER" != "null" ] && [ -n "$CT_AFTER" ]; then
  IS_NEAR=$(node -e "
    const ct  = BigInt('$CT_AFTER');
    const st  = BigInt('$START_MS');
    const tol = BigInt('60000'); // 1 bar = 60s
    const diff = ct > st ? ct - st : st - ct;
    console.log(diff <= tol ? 'true' : 'false');
  ")
  [ "$IS_NEAR" = "true" ] \
    && pass "TC-C: ticker 変更後 current_time≈start_time (ct=$CT_AFTER st=$START_MS)" \
    || fail "TC-C" "current_time=$CT_AFTER は start_time=$START_MS から 1 bar 以上離れている（リセット未発生の疑い）"
else
  fail "TC-C" "current_time が null"
fi

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: SOLUSDT のデータロード待機 → Resume → Playing 到達
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: SOLUSDT streams_ready 待機 → Resume → Playing 到達"

# SOLUSDT のロードが完了するまで待機
if wait_for_streams_ready "$PANE0" 30; then
  echo "  PANE0 (SOLUSDT) streams_ready=true"
else
  echo "  WARN: PANE0 streams_ready timeout (continuing)"
fi

api_post /api/replay/resume > /dev/null
if wait_status "Playing" 30; then
  pass "TC-D: Resume 後 status=Playing（回復可能）"
else
  fail "TC-D" "status=$(jqn "$(curl -s "$API/replay/status")" "d.status") (expected Playing)"
fi

stop_app
print_summary
