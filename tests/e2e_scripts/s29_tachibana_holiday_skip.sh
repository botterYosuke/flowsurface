#!/usr/bin/env bash
# s29_tachibana_holiday_skip.sh — S29: Tachibana D1 StepBackward が休場日（土日）をスキップすること
#
# 検証シナリオ（仕様 §10.1「離散ステップ」休場日スキップ）:
#
#   Tachibana の EventStore には土日祝の kline が存在しない。
#   StepBackward は klines_in(0..current_time) の最大 time を検索するため、
#   取引日（月〜金）にのみ landing する = 自然に休場日スキップが実現される。
#
#   TC-A: Paused 状態から StepForward で 2025-01-10 (金) 付近まで進める
#   TC-B: 金曜から StepForward 1 回 → 2025-01-11 (土, kline なし) に current_time が移動
#   TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) に戻る（休場日スキップ）
#   TC-D: 金曜から StepBackward → 2025-01-09 (木) に戻る（通常ステップ）
#   TC-E: StepBackward 連続 5 回 → 毎回取引日に着地すること（土日に止まらない）
#
# 前提条件:
#   - cargo build (debug) ビルドが必要
#   - DEV_USER_ID / DEV_PASSWORD 環境変数でセッション確立
#   - セッションなしの場合は全テストを SKIP して exit 0
#
# フィクスチャ: TachibanaSpot:7203 D1, 2025-01-07 00:00 〜 2025-01-15 00:00 (UTC)
#   取引日: 01-07(火), 01-08(水), 01-09(木), 01-10(金), [01-11 土, 01-12 日], 01-13(月), 01-14(火), 01-15(水)
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# debug ビルドを使用（Tachibana テスト標準）
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

echo "=== S29: Tachibana D1 StepBackward 休場日スキップ ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── セッション環境変数チェック ────────────────────────────────────────────────
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana セッション不要環境ではスキップ"
  echo ""
  echo "============================="
  echo "  PASS: 0  FAIL: 0  PEND: 0  (skipped)"
  echo "============================="
  exit 0
fi

# ── テスト対象日付の定数（UTC ms）────────────────────────────────────────────
# 2025-01-07 (火) 〜 2025-01-15 (水) のレンジ
RANGE_START="2025-01-07 00:00"
RANGE_END="2025-01-15 00:00"

# 各取引日のタイムスタンプ (UTC ms)
MS_JAN07=$(node -e "console.log(new Date('2025-01-07T00:00:00Z').getTime())")  # 火
MS_JAN08=$(node -e "console.log(new Date('2025-01-08T00:00:00Z').getTime())")  # 水
MS_JAN09=$(node -e "console.log(new Date('2025-01-09T00:00:00Z').getTime())")  # 木
MS_JAN10=$(node -e "console.log(new Date('2025-01-10T00:00:00Z').getTime())")  # 金
MS_JAN11=$(node -e "console.log(new Date('2025-01-11T00:00:00Z').getTime())")  # 土（休場）
MS_JAN12=$(node -e "console.log(new Date('2025-01-12T00:00:00Z').getTime())")  # 日（休場）
MS_JAN13=$(node -e "console.log(new Date('2025-01-13T00:00:00Z').getTime())")  # 月

echo "  レンジ: $RANGE_START → $RANGE_END"
echo "  取引日 ms: 01-07=$MS_JAN07, 01-09=$MS_JAN09, 01-10=$MS_JAN10, 01-11=$MS_JAN11, 01-13=$MS_JAN13"

# 2 日以内の許容誤差（D1 の bar snap によるずれを許容）
is_near_ms() {
  local ct="$1" target="$2"
  node -e "
    const ct   = BigInt('$ct');
    const tgt  = BigInt('$target');
    const tol  = BigInt('172800000'); // 2 days
    const diff = ct > tgt ? ct - tgt : tgt - ct;
    console.log(diff <= tol ? 'true' : 'false');
  "
}

# current_time が取引日かどうか確認（土曜 / 日曜 でないこと）
is_trading_day() {
  local ct="$1"
  node -e "
    const d = new Date(Number('$ct'));
    const dow = d.getUTCDay(); // 0=日, 6=土
    console.log(dow !== 0 && dow !== 6 ? 'true' : 'false');
  "
}

# ── フィクスチャ: Tachibana D1 (Live 起動 → セッション確立後に toggle + play) ─
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S29-TachibanaHoliday","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S29-TachibanaHoliday"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# Tachibana セッション確立まで待機（最大 120 秒）
echo "  Tachibana セッション待機..."
if ! wait_tachibana_session 120; then
  echo "  ERROR: Tachibana セッション未確立（120s タイムアウト）"
  fail "precond" "Tachibana セッション確立失敗"
  print_summary
  exit 1
fi
echo "  Tachibana セッション確立"

# ペイン ID 取得
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" \
  "$(curl -s "$API/pane/list")")
if [ -z "$PANE_ID" ]; then
  fail "precond" "ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE_ID=$PANE_ID"

# Replay モードへ toggle
curl -s -X POST "$API/replay/toggle" > /dev/null
echo "  Replay モードに切替"

# Play 発火（固定レンジ）
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$RANGE_START\",\"end\":\"$RANGE_END\"}" > /dev/null
echo "  Play 送信"

# Playing に到達するまで待機（最大 180 秒: D1 データ全件フェッチを考慮）
echo "  Playing 待機（最大 180s）..."
if ! wait_status "Playing" 180; then
  diagnose_playing_failure
  fail "precond" "Playing に到達せず"
  print_summary
  exit 1
fi
echo "  Playing 到達"

# Pause して step 操作を開始
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 0.5

CT_INIT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  Pause 後 current_time=$CT_INIT (expected ≈ $MS_JAN07)"

# ─────────────────────────────────────────────────────────────────────────────
# TC-A: StepForward ×3 で 2025-01-10 (金) 付近まで進める
#
#  初期 current_time = 2025-01-07 (火)
#  StepForward: +1D × 3 → 2025-01-08 → 2025-01-09 → 2025-01-10
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-A: StepForward ×3 で 2025-01-10 (金) 付近まで前進"

for i in 1 2 3; do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
done

CT_A=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  3 回 StepForward 後 current_time=$CT_A"

# 2025-01-10 (金) の ±2 日以内に着地しているか確認
IS_NEAR_FRI=$(is_near_ms "$CT_A" "$MS_JAN10")
[ "$IS_NEAR_FRI" = "true" ] \
  && pass "TC-A: StepForward ×3 後 current_time ≈ 2025-01-10 ($CT_A)" \
  || fail "TC-A" "current_time=$CT_A は 2025-01-10 ($MS_JAN10) から 2 日以上離れている"

# ─────────────────────────────────────────────────────────────────────────────
# TC-B: 金曜から StepForward 1 回 → 2025-01-11 (土) に current_time が移動
#
#  StepForward (Paused) は current_time + step_size (86400000ms) を機械的に足すため、
#  土曜のタイムスタンプに移動する（kline はないが current_time は変わる）。
#  この「土曜 current_time」が TC-C の前提となる。
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-B: StepForward 1 回 → 土曜 current_time"

CT_BEFORE_B=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 0.3
CT_B=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  StepForward 前=$CT_BEFORE_B → 後=$CT_B"

# current_time が前進したことを確認
ADVANCED_B=$(node -e "console.log(BigInt('$CT_B') > BigInt('$CT_BEFORE_B'))")
[ "$ADVANCED_B" = "true" ] \
  && pass "TC-B: StepForward で current_time が前進 ($CT_BEFORE_B → $CT_B)" \
  || fail "TC-B" "StepForward で current_time が変化しない"

# ─────────────────────────────────────────────────────────────────────────────
# TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) に戻る
#
#  StepBackward は klines_in(0..current_time) の最大 time を検索する。
#  EventStore に土曜の kline がないため、前の取引日（金曜）の kline.time が返る。
#  = 休場日スキップの本質的な動作
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) にスキップ"

CT_BEFORE_C=$CT_B  # 土曜（TC-B 後）
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 0.5
CT_C=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  StepBackward 前=$CT_BEFORE_C → 後=$CT_C"

# current_time が後退したことを確認
BACKWARD_C=$(node -e "console.log(BigInt('$CT_C') < BigInt('$CT_BEFORE_C'))")
[ "$BACKWARD_C" = "true" ] \
  && pass "TC-C1: StepBackward で current_time が後退 ($CT_BEFORE_C → $CT_C)" \
  || fail "TC-C1" "StepBackward で current_time が変化しない"

# 2025-01-10 (金) の ±2 日以内に着地しているか（土日曜に止まっていないか）
IS_NEAR_FRI_C=$(is_near_ms "$CT_C" "$MS_JAN10")
[ "$IS_NEAR_FRI_C" = "true" ] \
  && pass "TC-C2: StepBackward が土曜をスキップし 2025-01-10 (金) 付近に着地 ($CT_C)" \
  || fail "TC-C2" "current_time=$CT_C は 2025-01-10 ($MS_JAN10) から 2 日以上離れている（休場日スキップ不成立の疑い）"

# 取引日であることを確認（土曜・日曜でないこと）
IS_TRADING_C=$(is_trading_day "$CT_C")
[ "$IS_TRADING_C" = "true" ] \
  && pass "TC-C3: StepBackward 後 current_time は取引日（土日でない）" \
  || fail "TC-C3" "current_time=$CT_C が土曜または日曜 — 休場日スキップ失敗"

# ─────────────────────────────────────────────────────────────────────────────
# TC-D: 金曜 current_time から StepBackward → 2025-01-09 (木) に戻る（通常ステップ）
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-D: 金曜 current_time から StepBackward → 2025-01-09 (木)"

CT_BEFORE_D=$CT_C  # 金曜
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 0.5
CT_D=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
echo "  StepBackward 前=$CT_BEFORE_D → 後=$CT_D"

BACKWARD_D=$(node -e "console.log(BigInt('$CT_D') < BigInt('$CT_BEFORE_D'))")
[ "$BACKWARD_D" = "true" ] \
  && pass "TC-D1: 通常 StepBackward で current_time が後退" \
  || fail "TC-D1" "StepBackward で current_time が変化しない"

IS_NEAR_THU=$(is_near_ms "$CT_D" "$MS_JAN09")
[ "$IS_NEAR_THU" = "true" ] \
  && pass "TC-D2: 金曜 → StepBackward → 2025-01-09 (木) 付近 ($CT_D)" \
  || fail "TC-D2" "current_time=$CT_D は 2025-01-09 ($MS_JAN09) から 2 日以上離れている"

# ─────────────────────────────────────────────────────────────────────────────
# TC-E: StepBackward 連続 5 回 → 毎回取引日に着地すること
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo "── TC-E: StepBackward 連続 5 回 → 毎回取引日に着地"

# TC-A の位置（2025-01-10 付近）に戻してから実施
# StepForward で進めて再度 2025-01-10 付近へ
# 現在: 2025-01-09 (TC-D 後) → StepForward 1 回で 2025-01-10 付近に戻す
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 0.3

ALL_TRADING="true"
PREV_CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")

for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.4
  CT_STEP=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")

  # 後退していることを確認
  IS_BACK=$(node -e "console.log(BigInt('$CT_STEP') < BigInt('$PREV_CT'))")
  if [ "$IS_BACK" != "true" ]; then
    echo "  TC-E[$i]: 後退なし ($PREV_CT → $CT_STEP) — start_time に到達した可能性あり"
    break
  fi

  # 取引日に着地しているか確認
  IS_TD=$(is_trading_day "$CT_STEP")
  DOW=$(node -e "const d=new Date(Number('$CT_STEP')); const days=['日','月','火','水','木','金','土']; console.log(days[d.getUTCDay()])")
  echo "  TC-E[$i]: current_time=$CT_STEP ($DOW) trading_day=$IS_TD"

  if [ "$IS_TD" != "true" ]; then
    ALL_TRADING="false"
  fi
  PREV_CT=$CT_STEP
done

[ "$ALL_TRADING" = "true" ] \
  && pass "TC-E: StepBackward 5 回全て取引日に着地（土日スキップ確認）" \
  || fail "TC-E" "StepBackward が土曜または日曜に止まった（休場日スキップ失敗）"

stop_app
print_summary
