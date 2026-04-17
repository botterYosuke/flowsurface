#!/usr/bin/env bash
# s15_chart_snapshot.sh — スイート S15: chart-snapshot API テスト
#
# 検証シナリオ:
#   TC-S15-01: oldest_ts ≤ start_time かつ差分 ≤ 301 bars（PRE_START_HISTORY_BARS=300 確認）
#   TC-S15-02: StepForward 後 bar_count 増加または同数
#   TC-S15-03: StepBackward 後も snapshot 取得可能（クラッシュなし）
#   TC-S15-04: 存在しない pane_id → {"error":"..."} + アプリ生存
#   TC-S15-05: Live モード中の snapshot 取得後もアプリ応答あり
#
# 仕様根拠:
#   docs/replay_header.md §9.2 — GET /api/pane/chart-snapshot レスポンスフォーマット
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
#   注: chart-snapshot API 未実装環境では全 TC が PENDING
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S15: chart-snapshot API テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app

if ! wait_playing 30; then
  diagnose_playing_failure
  fail "TC-S15-precond" "Playing 到達せず"
  exit 1
fi

# Pause してからペイン ID を取得
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S15-precond" "Paused に遷移せず"
  exit 1
fi

PANES=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE_ID" ]; then
  fail "TC-S15-precond" "ペイン ID 取得失敗"
  exit 1
fi
echo "  PANE_ID=$PANE_ID"

# TC-S15-01: PRE_START_HISTORY_BARS=300 の検証
# oldest_ts ≤ start_time かつ start_time - oldest_ts ≤ 300 bars 分 (60000ms × 301)
# bar_count は再生中に range 内バーが積み重なるため上限は設けない
SNAP=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  snapshot response: $SNAP"
BAR_COUNT=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP")
OLDEST_TS=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.oldest_ts !== undefined ? String(d.oldest_ts) : 'null');" "$SNAP")
START_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
echo "  bar_count=$BAR_COUNT oldest_ts=$OLDEST_TS start_time=$START_T"
if node -e "
  const oldest = BigInt(process.argv[1]);
  const start  = BigInt(process.argv[2]);
  const STEP   = 60000n;            // M1
  const MAX_PRE = 301n * STEP;      // PRE_START_HISTORY_BARS=300 + 1 bar tolerance
  const ok = oldest <= start && (start - oldest) <= MAX_PRE;
  process.exit(ok ? 0 : 1);
" "$OLDEST_TS" "$START_T" 2>/dev/null; then
  pass "TC-S15-01: oldest_ts=$OLDEST_TS ≤ start=$START_T かつ差分 ≤ 301 bars (PRE_START_HISTORY_BARS 確認)"
else
  fail "TC-S15-01" "oldest_ts=$OLDEST_TS start=$START_T bar_count=$BAR_COUNT (pre-start history バー数異常)"
fi

# TC-S15-02: StepForward 後 bar_count が増加または同数（リグレッションなし）
BAR_BEFORE="$BAR_COUNT"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 10
sleep 0.5
SNAP2=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
BAR_AFTER=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP2")
echo "  bar_count after StepForward: $BAR_BEFORE → $BAR_AFTER"
if node -e "process.exit(Number(process.argv[1]) >= Number(process.argv[2]) ? 0 : 1);" \
     "$BAR_AFTER" "$BAR_BEFORE" 2>/dev/null; then
  pass "TC-S15-02: StepForward 後 bar_count=$BAR_AFTER >= before=$BAR_BEFORE"
else
  fail "TC-S15-02" "bar_count=$BAR_AFTER < before=$BAR_BEFORE（バー減少の異常）"
fi

# TC-S15-03: StepBackward 後も snapshot 取得可能（クラッシュしない）
# 少し前進してから StepBackward（start 境界クランプを避けるため）
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
done
wait_status Paused 10 || true
curl -s -X POST "$API/replay/step-backward" > /dev/null
wait_status Paused 10
sleep 0.3
SNAP3=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
HAS_BAR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.bar_count !== undefined && !d.error ? 'true' : 'false');
" "$SNAP3")
BAR3=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count);" "$SNAP3")
[ "$HAS_BAR" = "true" ] \
  && pass "TC-S15-03: StepBackward 後 snapshot 取得成功 (bar_count=$BAR3)" \
  || fail "TC-S15-03" "snapshot 異常レスポンス: $SNAP3"

# TC-S15-04: 存在しないペイン ID に対する snapshot → {"error":"..."} かつクラッシュなし
FAKE_ID="00000000-0000-0000-0000-deadbeef0000"
SNAP_FAKE=$(curl -s "$API/pane/chart-snapshot?pane_id=$FAKE_ID")
HAS_ERROR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.error ? 'true' : 'false');
" "$SNAP_FAKE")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HAS_ERROR" = "true" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S15-04: 不正 pane_id → error 応答 & アプリ生存確認 (resp=$SNAP_FAKE)" \
  || fail "TC-S15-04" "has_error=$HAS_ERROR alive=$ALIVE resp=$SNAP_FAKE"

# TC-S15-05: Live モードで snapshot を取得してもクラッシュしない
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
SNAP_LIVE=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  Live mode snapshot: $SNAP_LIVE"
# アプリがまだ応答しているかを確認（クラッシュ検出）
if curl -s "$API/replay/status" > /dev/null 2>&1; then
  pass "TC-S15-05: Live モード中の snapshot 取得後もアプリ応答あり"
else
  fail "TC-S15-05" "Live モード中の snapshot 取得後にアプリが応答しなくなった"
fi
# Replay モードに戻す
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2

print_summary
[ $FAIL -eq 0 ]
