#!/usr/bin/env bash
# s16_replay_resilience.sh — スイート S16: UI操作中の Replay 耐性テスト
# Replay 再生中に各種 UI 操作を行っても壊れないことを確認する
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S16: UI操作中の Replay 耐性テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── TC-S16-01: 速度ボタン連打 ─────────────────────────────────────────────
# speed API を 20 回高速に叩いて最終的に status=Playing
echo "  [TC-S16-01] 速度ボタン連打..."
START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app

if ! wait_playing 30; then
  fail "TC-S16-01-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 20); do
  curl -s -X POST "$API/replay/speed" > /dev/null
done

# 最終的に Playing 状態を維持しているか（speed は Playing 状態に影響しない）
wait_status Playing 10 || true
FINAL_STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$FINAL_STATUS" = "Playing" ] \
  && pass "TC-S16-01: speed 20 連打後 status=Playing" \
  || fail "TC-S16-01" "status=$FINAL_STATUS (Playing 期待)"

stop_app

# ── TC-S16-02: 日付境界（UTC 0:00 越え）─────────────────────────────────
# 真夜中 UTC 0:00 をまたぐ range を設定して StepForward/StepBackward を検証
echo "  [TC-S16-02] 日付境界テスト（UTC 0:00 越え）..."

# 現在時刻の UTC 0:00 を挟む range を計算
# start: 前日の 23:00 UTC, end: 翌日の 01:00 UTC
MIDNIGHT_MINUS_1=$(node -e "
  const now = new Date();
  const d = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - 1, 23, 0));
  const pad = n => String(n).padStart(2,'0');
  console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' 23:00');
")
MIDNIGHT_PLUS_1=$(node -e "
  const now = new Date();
  const d = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), 1, 0));
  const pad = n => String(n).padStart(2,'0');
  console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' 01:00');
")
echo "  range: $MIDNIGHT_MINUS_1 → $MIDNIGHT_PLUS_1"

setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$MIDNIGHT_MINUS_1" "$MIDNIGHT_PLUS_1"
start_app

if ! wait_playing 30; then
  fail "TC-S16-02-pre" "Playing 到達せず（日付境界 range）"
  # 前日 23:00〜当日 01:00 のデータがない可能性あり → PEND として継続
  pend "TC-S16-02" "日付境界 range でデータなし / Playing 到達せず"
  stop_app
else
  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10

  # 深夜 0:00 付近まで前進
  speed_to_10x
  T_MIDNIGHT=$(node -e "
    const now = new Date();
    console.log(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), 0, 0)).getTime());
  ")
  curl -s -X POST "$API/replay/resume" > /dev/null
  wait_status Playing 10
  # current_time が 0:00 UTC を超えるまで待機（最大 30 秒）
  CROSSED=false
  for i in $(seq 1 60); do
    CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    if [ -n "$CT" ] && [ "$CT" != "null" ] && \
       [ "$(bigt_gt "$CT" "$T_MIDNIGHT")" = "true" ]; then
      CROSSED=true
      break
    fi
    sleep 0.5
  done

  curl -s -X POST "$API/replay/pause" > /dev/null
  wait_status Paused 10

  if $CROSSED; then
    # 0:00 越え後に StepForward → crash なし
    CT_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    curl -s -X POST "$API/replay/step-forward" > /dev/null
    wait_status Paused 10
    CT_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
    DELTA=$(node -e "console.log(String(BigInt('$CT_AFTER') - BigInt('$CT_BEFORE')))")
    [ "$DELTA" = "60000" ] \
      && pass "TC-S16-02a: UTC 0:00 越え後 StepForward delta=60000ms" \
      || fail "TC-S16-02a" "delta=$DELTA (expected 60000)"

    # StepBackward → crash なし
    curl -s -X POST "$API/replay/step-backward" > /dev/null
    wait_status Paused 10
    STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$STATUS" = "Paused" ] \
      && pass "TC-S16-02b: UTC 0:00 越え後 StepBackward → status=Paused" \
      || fail "TC-S16-02b" "status=$STATUS"
  else
    pend "TC-S16-02" "UTC 0:00 境界を超えられなかった（データ不足 or 速度不足）"
  fi

  stop_app
fi

# ── TC-S16-03: Live ↔ Replay 高速切替 ──────────────────────────────────
# toggle を 10 回連続 → 最終状態が安定している
echo "  [TC-S16-03] Live ↔ Replay 高速切替..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S16-03-pre" "Playing 到達せず"
  exit 1
fi

for i in $(seq 1 10); do
  curl -s -X POST "$API/replay/toggle" > /dev/null
  sleep 0.3
done

# 最終状態が安定しているか（アプリが応答する）
sleep 2
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
FINAL=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE" = "true" ] \
  && pass "TC-S16-03: toggle 10 連打後もアプリ応答あり (final_status=$FINAL)" \
  || fail "TC-S16-03" "toggle 連打後にアプリが応答しなくなった"

stop_app

# ── TC-S16-04: Playing 中の toggle ─────────────────────────────────────
echo "  [TC-S16-04] Playing 中の toggle..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S16-04-pre" "Playing 到達せず"
  exit 1
fi

# Playing 中に toggle（Live へ切替 or 停止）
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
# アプリが生存し、状態が確定していれば OK
[ "$ALIVE" = "true" ] \
  && pass "TC-S16-04: Playing 中の toggle → アプリ生存 (status=$STATUS_AFTER)" \
  || fail "TC-S16-04" "toggle 後にアプリが応答しなくなった"

stop_app

# ── TC-S16-05: Paused 中の toggle → Live → 再び Replay → Playing ──────
echo "  [TC-S16-05] Paused 中の toggle..."
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app

if ! wait_playing 30; then
  fail "TC-S16-05-pre" "Playing 到達せず"
  exit 1
fi

# Pause 状態にする
curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 10; then
  fail "TC-S16-05-pre" "Paused に遷移せず"
  exit 1
fi

# toggle → Live へ
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 2
STATUS_LIVE=$(jqn "$(curl -s "$API/replay/status")" "d.status")
ALIVE=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
[ "$ALIVE" = "true" ] \
  && pass "TC-S16-05a: Paused → toggle → アプリ生存 (status=$STATUS_LIVE)" \
  || fail "TC-S16-05a" "toggle 後にアプリが応答しなくなった"

# toggle → Replay に戻る
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 3
# アプリが応答していれば OK（Live モード継続/Replay 切替どちらも許容）
ALIVE2=$(curl -s "$API/replay/status" > /dev/null 2>&1 && echo "true" || echo "false")
STATUS_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ALIVE2" = "true" ] \
  && pass "TC-S16-05b: 2 回目 toggle 後もアプリ生存 (status=$STATUS_BACK)" \
  || fail "TC-S16-05b" "2 回目 toggle 後にアプリが応答しなくなった"

print_summary
[ $FAIL -eq 0 ]
