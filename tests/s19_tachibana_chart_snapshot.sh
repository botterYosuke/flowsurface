#!/usr/bin/env bash
# s19_tachibana_chart_snapshot.sh — スイート S19: chart-snapshot API テスト（TachibanaSpot）
#
# 検証シナリオ:
#   TC-S19-01: Play 後 bar_count が 1〜301（PRE_START_HISTORY_BARS=300 確認）
#   TC-S19-02: StepForward 後 bar_count 増加または同数（D1 step = 86400000ms）
#   TC-S19-03: StepBackward 後も snapshot 取得可能（クラッシュなし）
#   TC-S19-04: 存在しない pane_id → {"error":"..."} + アプリ生存
#   TC-S19-05: Live モード中の snapshot 取得後もアプリ応答あり
#
# 仕様根拠:
#   docs/replay_header.md §9.2 — GET /api/pane/chart-snapshot（TachibanaSpot D1 版）
#
# フィクスチャ: TachibanaSpot:7203 D1, UTC[-96h, -24h]（4 日レンジ）
#   ビルド: cargo build（debug）
#   前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み
#   注: chart-snapshot API 未実装環境では全 TC が PENDING
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップします"; exit 0
fi

echo "=== S19: chart-snapshot API テスト（TachibanaSpot:7203 D1）==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# tachibana_replay_setup は common_helpers.sh に定義済み

START=$(utc_offset -96)
END=$(utc_offset -24)
tachibana_replay_setup "$START" "$END"

if ! wait_playing 120; then
  fail "TC-S19-precond" "Playing 到達せず（120 秒タイムアウト）"
  exit 1
fi

# ペイン ID を取得して streams_ready を待機してから Pause
PANES=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE_ID" ]; then
  fail "TC-S19-precond" "ペイン ID 取得失敗"
  exit 1
fi
echo "  PANE_ID=$PANE_ID"

# streams_ready=true になるまで待機（最大 60 秒）
echo "  waiting for streams_ready..."
if ! wait_for_streams_ready "$PANE_ID" 60; then
  echo "  WARN: streams_ready タイムアウト（継続）"
fi

# 少し再生させてから Pause（バーが蓄積されるのを待つ）
sleep 2
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S19-precond" "Paused に遷移せず"
  exit 1
fi
sleep 0.5

# TC-S19-01: Paused 直後のバー本数が 1 ≤ bar_count ≤ 301
SNAP=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  snapshot response: $SNAP"
BAR_COUNT=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP")
echo "  bar_count=$BAR_COUNT"
if node -e "
  const n = Number(process.argv[1]);
  process.exit((Number.isFinite(n) && n >= 1 && n <= 301) ? 0 : 1);
" "$BAR_COUNT" 2>/dev/null; then
  pass "TC-S19-01: Play 後 bar_count=$BAR_COUNT (1 ≤ N ≤ 301, PRE_START_HISTORY_BARS 確認)"
else
  fail "TC-S19-01" "bar_count=$BAR_COUNT (想定: 1..301)"
fi

# TC-S19-02: StepForward 後 bar_count が増加または同数（D1 step = 86400000ms）
BAR_BEFORE="$BAR_COUNT"
curl -s -X POST "$API/replay/step-forward" > /dev/null
wait_status Paused 15
sleep 0.5
SNAP2=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
BAR_AFTER=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count !== undefined ? String(d.bar_count) : 'null');" "$SNAP2")
echo "  bar_count after StepForward: $BAR_BEFORE → $BAR_AFTER"
if node -e "process.exit(Number(process.argv[1]) >= Number(process.argv[2]) ? 0 : 1);" \
     "$BAR_AFTER" "$BAR_BEFORE" 2>/dev/null; then
  pass "TC-S19-02: StepForward 後 bar_count=$BAR_AFTER >= before=$BAR_BEFORE"
else
  fail "TC-S19-02" "bar_count=$BAR_AFTER < before=$BAR_BEFORE（バー減少の異常）"
fi

# TC-S19-03: StepBackward 後も snapshot 取得可能（クラッシュしない）
# 少し前進してから StepBackward（start 境界クランプを避けるため）
for i in $(seq 1 3); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.5
done
wait_status Paused 15 || true
curl -s -X POST "$API/replay/step-backward" > /dev/null
wait_status Paused 15
sleep 0.3
SNAP3=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
HAS_BAR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.bar_count !== undefined && !d.error ? 'true' : 'false');
" "$SNAP3")
BAR3=$(node -e "const d=JSON.parse(process.argv[1]); console.log(d.bar_count);" "$SNAP3")
[ "$HAS_BAR" = "true" ] \
  && pass "TC-S19-03: StepBackward 後 snapshot 取得成功 (bar_count=$BAR3)" \
  || fail "TC-S19-03" "snapshot 異常レスポンス: $SNAP3"

# TC-S19-04: 存在しないペイン ID に対する snapshot → {"error":"..."} かつクラッシュなし
FAKE_ID="00000000-0000-0000-0000-deadbeef0000"
SNAP_FAKE=$(curl -s "$API/pane/chart-snapshot?pane_id=$FAKE_ID")
HAS_ERROR=$(node -e "
  const d=JSON.parse(process.argv[1]);
  console.log(d.error ? 'true' : 'false');
" "$SNAP_FAKE")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$HAS_ERROR" = "true" ] && [ "$ALIVE" = "true" ] \
  && pass "TC-S19-04: 不正 pane_id → error 応答 & アプリ生存確認 (resp=$SNAP_FAKE)" \
  || fail "TC-S19-04" "has_error=$HAS_ERROR alive=$ALIVE resp=$SNAP_FAKE"

# TC-S19-05: Live モードで snapshot を取得してもクラッシュしない
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
SNAP_LIVE=$(curl -s "$API/pane/chart-snapshot?pane_id=$PANE_ID")
echo "  Live mode snapshot: $SNAP_LIVE"
if curl -s "$API/replay/status" > /dev/null 2>&1; then
  pass "TC-S19-05: Live モード中の snapshot 取得後もアプリ応答あり"
else
  fail "TC-S19-05" "Live モード中の snapshot 取得後にアプリが応答しなくなった"
fi
# Replay モードに戻す
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2

print_summary
[ $FAIL -eq 0 ]
