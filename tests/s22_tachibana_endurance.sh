#!/usr/bin/env bash
# s22_tachibana_endurance.sh — スイート S22: 耐久テスト（TachibanaSpot）
#
# 検証シナリオ:
#   TC-S22-01: 長期 range (100 bar 相当) を 10x 速度で完走 → Paused 到達
#   TC-S22-02-fwd/bwd: StepForward × N + StepBackward × N → crash なし（D1 版）
#
# 仕様根拠:
#   TachibanaSpot D1 での長時間再生・高速操作でのメモリリーク・デッドロック検証
#
# フィクスチャ: TachibanaSpot:7203 D1, Tachibana セッション必須（DEV AUTO-LOGIN）
#   ビルド: cargo build（debug）
#   前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み
#   警告: 完走に 15〜30 分かかる
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップします"; exit 0
fi

echo "=== S22: 耐久テスト（TachibanaSpot:7203 D1）==="
echo "  警告: このスクリプトは完走に 15〜30 分かかる"
backup_state
trap 'stop_app; restore_state' EXIT ERR

tachibana_replay_setup() {
  local start=$1 end=$2
  cat > "$DATA_DIR/saved-state.json" <<HEREDOC
{
  "layout_manager":{"layouts":[{"name":"Test-D1","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"Test-D1"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
HEREDOC
  start_app
  # DEV AUTO-LOGIN で Tachibana セッションが確立されるまで待機
  echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
  if ! wait_tachibana_session 120; then
    echo "  ERROR: Tachibana session not established after 120s"
    return 1
  fi
  echo "  Tachibana session established"
  # ペインの D1 kline データがフェッチ完了するまで待機
  local pane_id
  pane_id=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" \
    "$(curl -s "$API/pane/list")")
  if [ -n "$pane_id" ]; then
    echo "  waiting for D1 klines (streams_ready)..."
    wait_for_streams_ready "$pane_id" 120 || echo "  WARN: streams_ready timeout (continuing)"
  fi
  curl -s -X POST "$API/replay/toggle" > /dev/null
  curl -s -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$start\",\"end\":\"$end\"}" > /dev/null
}

# ── TC-S22-01: 60 日 range を 10x 速度で再生し終了 → Paused ───────────────
# -1440h(-24h) ≈ 59 calendar days ≈ 42 trading bars
# wait_playing は speed_to_10x より前（1x 速度）で実行される
# 42 bars × 100ms/bar (1x) = 4200ms → 1s ポーリングで確実に検出可能
echo "  [TC-S22-01] 10x 速度 60 日 range 完走テスト..."
START_LONG=$(utc_offset -1440)
END_LONG=$(utc_offset -24)
tachibana_replay_setup "$START_LONG" "$END_LONG"

if ! wait_playing 60; then
  fail "TC-S22-01-pre" "Playing 到達せず"
  exit 1
fi

speed_to_10x
echo "  10x 速度で再生中（最大 180 秒待機）..."
if wait_status Paused 180; then
  pass "TC-S22-01: 60 日 range 10x 完走 → Paused 到達"
else
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  fail "TC-S22-01" "180 秒経過後も status=$STATUS（Paused 未到達）"
fi

stop_app

# ── TC-S22-02: D1 Step 100 回（各方向 50 回）→ crash なし ────────────────
# StepForward 50 回 = 50 × 86400000ms ≒ 50 日分の range が必要
# range: -1300h (-54 日) 〜 -24h（十分な空間を確保）
echo "  [TC-S22-02] D1 Step 100 回耐久テスト（forward × 50 + backward × 50）..."
START_WIDE=$(utc_offset -1300)
END_WIDE=$(utc_offset -24)
tachibana_replay_setup "$START_WIDE" "$END_WIDE"

if ! wait_playing 60; then
  fail "TC-S22-02-pre" "Playing 到達せず"
  exit 1
fi

curl -s -X POST "$API/replay/pause" > /dev/null
if ! wait_status Paused 15; then
  fail "TC-S22-02-pre" "Paused に遷移せず"
  exit 1
fi

echo "  StepForward × 50..."
CRASH=false
for i in $(seq 1 50); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    echo "  CRASH detected at forward step #$i"
    break
  fi
  if [ $((i % 10)) -eq 0 ]; then
    echo "    forward step $i/50..."
  fi
done

if $CRASH; then
  fail "TC-S22-02-fwd" "StepForward 連打中にアプリがクラッシュした"
else
  wait_status Paused 15 || true
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S22-02-fwd: StepForward 50 回完了 → status=Paused" \
    || fail "TC-S22-02-fwd" "status=$STATUS (Paused 期待)"
fi

echo "  StepBackward × 50..."
CRASH=false
for i in $(seq 1 50); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.3
  if ! curl -s "$API/replay/status" > /dev/null 2>&1; then
    CRASH=true
    echo "  CRASH detected at backward step #$i"
    break
  fi
  if [ $((i % 10)) -eq 0 ]; then
    echo "    backward step $i/50..."
  fi
done

if $CRASH; then
  fail "TC-S22-02-bwd" "StepBackward 連打中にアプリがクラッシュした"
else
  wait_status Paused 15 || true
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Paused" ] \
    && pass "TC-S22-02-bwd: StepBackward 50 回完了 → status=Paused" \
    || fail "TC-S22-02-bwd" "status=$STATUS (Paused 期待)"
fi

stop_app

# ── TC-S22-03: ペイン CRUD サイクル 20 回（Playing 中）→ Playing 維持 ───
# 20 CRUD サイクル × ~3 秒/cycle ≒ 60 秒かかるため -18000h/-24h (750 bars ≒ 75 秒 at 1x) を使用
echo "  [TC-S22-03] Playing 中 split→close × 20 サイクル..."
tachibana_replay_setup "$(utc_offset -18000)" "$(utc_offset -24)"

if ! wait_playing 60; then
  fail "TC-S22-03-pre" "Playing 到達せず"
  exit 1
fi

CRUD_FAIL=false
for i in $(seq 1 20); do
  PANES=$(curl -s "$API/pane/list")
  PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
  if [ -z "$PANE0" ]; then
    fail "TC-S22-03-$i" "ペイン ID 取得失敗"
    CRUD_FAIL=true
    break
  fi

  curl -s -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}" > /dev/null
  if ! wait_for_pane_count 2 10; then
    fail "TC-S22-03-$i" "split 後ペイン数が 2 にならなかった"
    CRUD_FAIL=true
    break
  fi

  PANES=$(curl -s "$API/pane/list")
  NEW_PANE=$(node -e "
    const ps=(JSON.parse(process.argv[1]).panes||[]);
    const p=ps.find(x=>x.id!=='$PANE0');
    console.log(p?p.id:'');
  " "$PANES")
  if [ -z "$NEW_PANE" ]; then
    fail "TC-S22-03-$i" "新ペイン ID 取得失敗"
    CRUD_FAIL=true
    break
  fi

  curl -s -X POST "$API/pane/close" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$NEW_PANE\"}" > /dev/null
  if ! wait_for_pane_count 1 10; then
    fail "TC-S22-03-$i" "close 後ペイン数が 1 にならなかった"
    CRUD_FAIL=true
    break
  fi

  if [ $((i % 5)) -eq 0 ]; then
    STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    if [ "$STATUS" != "Playing" ]; then
      fail "TC-S22-03-$i" "CRUD サイクル $i 回後 status=$STATUS (Playing 期待)"
      CRUD_FAIL=true
      break
    fi
    echo "    cycle $i/20: status=Playing OK"
  fi
done

if ! $CRUD_FAIL; then
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$STATUS" = "Playing" ] \
    && pass "TC-S22-03: CRUD 20 サイクル完了 → status=Playing 維持" \
    || fail "TC-S22-03" "20 サイクル後 status=$STATUS (Playing 期待)"
fi

print_summary
[ $FAIL -eq 0 ]
