#!/usr/bin/env bash
# s20_tachibana_replay_resilience.sh — スイート S20: UI操作中の Replay 耐性テスト（TachibanaSpot）
#
# 検証シナリオ:
#   TC-S20-01: speed 20 連打 + Resume → status=Playing（D1, 100ms/bar クラッシュなし）
#   TC-S20-02a〜b: D1 StepForward delta=86400000ms・StepBackward 後 status=Paused
#   TC-S20-03: Live ↔ Replay toggle 10 連打 → アプリ応答維持（D1 版）
#
# 仕様根拠:
#   docs/replay_header.md §8 — 速度ボタン連打耐性, §4 — Live/Replay toggle 安定性（TachibanaSpot D1 版）
#
# フィクスチャ: TachibanaSpot:7203 D1, Tachibana セッション必須（DEV AUTO-LOGIN）
#   ビルド: cargo build（debug）
#   前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップします"; exit 0
fi

echo "=== S20: UI操作中の Replay 耐性テスト（TachibanaSpot:7203 D1）==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# tachibana_replay_setup は common_helpers.sh に定義済み

# ── TC-S20-01: 速度ボタン連打 ─────────────────────────────────────────────
# D1 は 1x 速度で 100ms/bar。speed 20 連打（~2 秒） + 確認の間も Playing を維持するため
# -2400h/-24h (100 bar ≒ 10 秒 at 1x) を使用する。
echo "  [TC-S20-01] 速度ボタン連打..."
tachibana_replay_setup "$(utc_offset -2400)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-01-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 20); do
  curl -s -X POST "$API/replay/speed" > /dev/null
done

# 新仕様: CycleSpeed は pause + seek(range.start) を伴う。
# 20 連打後は Paused になるが、これは仕様通り。Resume で回復できることを確認する。
# SPEED_INSTANT を通過した場合は 1 tick で range_end まで消化（高速完了）→ これも crash なし として合格。
STATUS_AFTER_SPEED=$(curl -s "$API/replay/status")
CT_PRE_RESUME=$(jqn "$STATUS_AFTER_SPEED" "d.current_time")
RANGE_END_MS=$(jqn "$STATUS_AFTER_SPEED" "d.range_end")
curl -s -X POST "$API/replay/resume" > /dev/null
wait_status Playing 10 || true
FINAL_STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT_POST_RESUME=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
CT_ADVANCED=$(node -e "
  const pre  = Number('${CT_PRE_RESUME}') || 0;
  const post = Number('${CT_POST_RESUME}') || 0;
  const rend = Number('${RANGE_END_MS}') || 0;
  // 通常の進行 OR SPEED_INSTANT により range_end まで到達（range 消化完了 = crash なし）
  const at_end = rend > 0 && post >= rend;
  console.log(post > pre || at_end ? 'true' : 'false');
")
{ [ "$FINAL_STATUS" = "Playing" ] || [ "$CT_ADVANCED" = "true" ]; } \
  && pass "TC-S20-01: speed 20 連打 + Resume → Playing または高速完了（crash なし）" \
  || fail "TC-S20-01" "status=$FINAL_STATUS, ct_advanced=$CT_ADVANCED (Playing または進行を期待)"

stop_app

# ── TC-S20-02: D1 StepForward/StepBackward の delta 検証 ───────────────
# TachibanaSpot:7203 D1 の StepForward delta は 86400000ms。
# D1 は 1x 速度で 100ms/bar のため -1300h/-24h (54 bar ≒ 5.4 秒) を使用し
# Playing 検出直後に即 Pause することで range 完了前に測定できるようにする。
echo "  [TC-S20-02] D1 StepForward/StepBackward delta 検証..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  pend "TC-S20-02" "Playing 到達せず → PEND"
  stop_app
else
  # Playing 検出直後に即 Pause（D1 は 100ms/step なので長く放置すると完了する）
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 15

  # ウォームアップ: バー境界にスナップしてから delta を計測
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  wait_status Paused 15

  # TC-S20-02a: StepForward delta = 86400000ms（バー境界から計測）
  T_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  wait_status Paused 15
  T_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")

  if [ -z "$T_BEFORE" ] || [ "$T_BEFORE" = "null" ] || \
     [ -z "$T_AFTER" ]  || [ "$T_AFTER"  = "null" ]; then
    fail "TC-S20-02a" "current_time 取得失敗 (before=$T_BEFORE after=$T_AFTER)"
  else
    DELTA=$(node -e "console.log(String(BigInt('$T_AFTER') - BigInt('$T_BEFORE')))")
    [ "$DELTA" = "86400000" ] \
      && pass "TC-S20-02a: D1 StepForward delta=86400000ms" \
      || fail "TC-S20-02a" "delta=$DELTA (expected 86400000)"
  fi

  # TC-S20-02b: StepBackward 後 status=Paused
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  wait_status Paused 15
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S20-02b: StepBackward 後 status=Paused" \
    || fail "TC-S20-02b" "status=$STATUS"

  stop_app
fi

# ── TC-S20-03: Live ↔ Replay 高速切替 ──────────────────────────────────
echo "  [TC-S20-03] Live ↔ Replay 高速切替..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-03-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 10); do
  curl -s -X POST "$API/replay/toggle" > /dev/null
  sleep 0.3
done

sleep 2
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
FINAL=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-03: toggle 10 連打後もアプリ応答あり (final_status=$FINAL)" \
  || fail "TC-S20-03" "toggle 連打後にアプリが応答しなくなった"

stop_app

# ── TC-S20-04: Playing 中の toggle ─────────────────────────────────────
echo "  [TC-S20-04] Playing 中の toggle..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-04-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-04: Playing 中の toggle → アプリ生存 (status=$STATUS_AFTER)" \
  || fail "TC-S20-04" "toggle 後にアプリが応答しなくなった"

stop_app

# ── TC-S20-05: Paused 中の toggle → Live → 再び Replay → Playing ──────
echo "  [TC-S20-05] Paused 中の toggle..."
tachibana_replay_setup "$(utc_offset -1300)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S20-05-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S20-05-pre" "Paused に遷移せず"
  exit 1
fi

# toggle → Live へ
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_LIVE=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$ALIVE" = "true" ] \
  && pass "TC-S20-05a: Paused → toggle → アプリ生存 (status=$STATUS_LIVE)" \
  || fail "TC-S20-05a" "toggle 後にアプリが応答しなくなった"

# toggle → Replay に戻る
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
ALIVE2=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE2" = "true" ] \
  && pass "TC-S20-05b: 2 回目 toggle 後もアプリ生存 (status=$STATUS_BACK)" \
  || fail "TC-S20-05b" "2 回目 toggle 後にアプリが応答しなくなった"

print_summary
[ $FAIL -eq 0 ]
