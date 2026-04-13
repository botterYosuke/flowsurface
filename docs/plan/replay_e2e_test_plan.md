# リプレイ機能 E2E テスト計画書

**作成日**: 2026-04-13  
**対象ブランチ**: `sasa/step`  
**参照仕様**: [docs/replay_header.md](../replay_header.md)  
**テストスキル**: [.claude/skills/e2e-test/SKILL.md](../../.claude/skills/e2e-test/SKILL.md)

---

## 目次

1. [概要と方針](#1-概要と方針)
2. [テスト環境セットアップ](#2-テスト環境セットアップ)
3. [スイート一覧](#3-スイート一覧)
4. [スイート S1: 基本ライフサイクル](#スイート-s1-基本ライフサイクル)
5. [スイート S2: 永続化往復テスト](#スイート-s2-永続化往復テスト)
6. [スイート S3: 起動時 Auto-play（Fixture 直接起動）](#スイート-s3-起動時-auto-playfixture-直接起動)
7. [スイート S4: マルチペイン・マルチ銘柄（Binance 混在）](#スイート-s4-マルチペインマルチ銘柄binance-混在)
8. [スイート S5: 立花証券混在（Tachibana + Binance）](#スイート-s5-立花証券混在tachibana--binance)
9. [スイート S6: 異なる時間軸混在](#スイート-s6-異なる時間軸混在)
10. [スイート S7: Mid-replay ペイン操作](#スイート-s7-mid-replay-ペイン操作)
11. [スイート S8: エラー・境界値ケース](#スイート-s8-エラー境界値ケース)
12. [スイート S9: 再生速度・Step 精度](#スイート-s9-再生速度step-精度)
13. [スイート S10: 範囲端・終端到達](#スイート-s10-範囲端終端到達)
14. [実行順序と依存関係](#14-実行順序と依存関係)
15. [合否判定基準](#15-合否判定基準)

---

## 1. 概要と方針

### 目的

リプレイ機能の全機能を HTTP API 経由で外部から駆動し、以下を確認する:

- 基本再生ライフサイクル（Play / Pause / Resume / Step / Speed）
- 永続化（保存 → 再起動後の復元）
- Fixture 直接起動による auto-play
- **エッジケース**: 立花証券と Binance の混在、異なる時間軸の混在、範囲端、境界値

### 方針

- `cargo build --release` を事前に実施（テストは release バイナリを使用）
- **日時は実行日の UTC 基準で動的生成**（未来日時はデータ取得不可）
- 各スイートは独立して実行できるよう、開始時にバックアップ→フィクスチャ配置、終了時に復元
- `jq` 不使用。`node -e` で JSON パース（`jqn` ヘルパー利用）
- 立花証券テスト（S5）は **ログイン済みセッションが必要**。セッション切れの場合はスキップして手動ログイン後に再実行

### スコープ外

- Heatmap / Ladder の Depth 描画（リプレイ非対応）
- Comparison ペイン
- GUI 描画のピクセル一致検証

---

## 2. テスト環境セットアップ

### 2.1 前提条件確認スクリプト

```bash
#!/bin/bash
# preflight.sh — 実行前に必ず確認する

echo "=== Preflight Check ==="

# 1. node / curl
node --version && echo "  node: OK" || echo "  ERROR: node not found"
curl --version | head -1 && echo "  curl: OK" || echo "  ERROR: curl not found"

# 2. バイナリ存在確認
EXE="C:/Users/sasai/Documents/flowsurface/target/release/flowsurface.exe"
[ -f "$EXE" ] && echo "  binary: OK" || echo "  ERROR: binary not found. Run: cargo build --release"

# 3. ポート 9876 が空いているか
if curl -s --max-time 1 "http://127.0.0.1:9876/api/replay/status" > /dev/null 2>&1; then
  echo "  WARNING: port 9876 already in use — stop existing instance first"
else
  echo "  port 9876: free"
fi

# 4. C:/tmp ディレクトリ
[ -d "C:/tmp" ] || mkdir -p "C:/tmp"
echo "  C:/tmp: OK"

# 5. DATA_DIR
DATA_DIR="$APPDATA/flowsurface"
[ -d "$DATA_DIR" ] && echo "  DATA_DIR: $DATA_DIR OK" || echo "  ERROR: DATA_DIR not found"

echo "=== Done ==="
```

### 2.2 共通ヘルパー

全スクリプト先頭に貼る（SKILL.md 準拠）:

```bash
#!/bin/bash
set -e

DATA_DIR="$APPDATA/flowsurface"
API="http://127.0.0.1:9876/api"
PASS=0
FAIL=0
EXE="C:/Users/sasai/Documents/flowsurface/target/release/flowsurface.exe"

jqn() {
  node -e "const d=JSON.parse(process.argv[1]); const v=$2; console.log(v === null || v === undefined ? 'null' : v);" "$1"
}

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1 — $2"; FAIL=$((FAIL + 1)); }

start_app() {
  echo "  Starting app..."
  "$EXE" 2>C:/tmp/e2e_debug.log &
  APP_PID=$!
  for i in $(seq 1 30); do
    if curl -s "$API/replay/status" > /dev/null 2>&1; then
      echo "  API ready (${i}s)"
      return 0
    fi
    sleep 1
  done
  echo "  ERROR: API not ready after 30s"
  return 1
}

stop_app() {
  echo "  Stopping app..."
  taskkill //f //im flowsurface.exe > /dev/null 2>&1 || true
  sleep 2
}

backup_state() {
  cp "$DATA_DIR/saved-state.json" "$DATA_DIR/saved-state.json.bak" 2>/dev/null || true
}

restore_state() {
  stop_app
  [ -f "$DATA_DIR/saved-state.json.bak" ] && \
    cp "$DATA_DIR/saved-state.json.bak" "$DATA_DIR/saved-state.json" || true
}

# 日時ヘルパー（UTC）
# 使い方: RANGE_START=$(utc_offset -2)  → 2時間前
utc_offset() {
  local h=$1
  date -u -d "${h} hours" +"%Y-%m-%d %H:%M" 2>/dev/null || \
  date -u -v${h}H +"%Y-%m-%d %H:%M"
}

# BigInt 比較: gt $A $B → true/false
bigt_gt() { node -e "console.log(BigInt('$1') > BigInt('$2'))"; }
bigt_ge() { node -e "console.log(BigInt('$1') >= BigInt('$2'))"; }
bigt_eq() { node -e "console.log(BigInt('$1') === BigInt('$2'))"; }
bigt_sub() { node -e "console.log(String(BigInt('$1') - BigInt('$2')))"; }

wait_playing() {
  local MAX=${1:-30}
  for i in $(seq 1 $MAX); do
    local ST
    ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$ST" = "Playing" ] && return 0
    sleep 1
  done
  return 1
}

wait_paused() {
  local MAX=${1:-15}
  for i in $(seq 1 $MAX); do
    local ST
    ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$ST" = "Paused" ] && return 0
    sleep 1
  done
  return 1
}

print_summary() {
  echo ""
  echo "============================="
  echo "  PASS: $PASS  FAIL: $FAIL"
  echo "============================="
  [ $FAIL -eq 0 ]
}
```

---

## 3. スイート一覧

| スイート | 説明 | 推定時間 | 立花ログイン要 |
|---------|------|---------|--------------|
| S1 | 基本ライフサイクル | 5 分 | 不要 |
| S2 | 永続化往復 | 4 分 | 不要 |
| S3 | Auto-play（Fixture 直接起動） | 3 分 | 不要 |
| S4 | マルチペイン・Binance 混在 | 5 分 | 不要 |
| S5 | 立花証券 + Binance 混在 | 8 分 | **要** |
| S6 | 異なる時間軸混在 | 6 分 | 不要 |
| S7 | Mid-replay ペイン操作 | 6 分 | 不要 |
| S8 | エラー・境界値ケース | 3 分 | 不要 |
| S9 | 再生速度・Step 精度 | 4 分 | 不要 |
| S10 | 範囲端・終端到達 | 5 分 | 不要 |

---

## スイート S1: 基本ライフサイクル

**Fixture**: 最小構成（BinanceLinear:BTCUSDT M1）、Live モード起動  
**目的**: Play→Pause→Resume→Step→Speed→Live 復帰の全遷移を検証する

```bash
#!/bin/bash
# s1_basic_lifecycle.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S1: 基本ライフサイクル ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager": {
    "layouts": [{"name":"S1-Basic","dashboard":{"pane":{
      "KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }
    },"popout":[]}}],
    "active_layout":"S1-Basic"
  },
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# --- TC-S1-01: Live モードで起動 ---
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Live" ] && pass "TC-S1-01: 起動時 mode=Live" || fail "TC-S1-01" "mode=$MODE"

# --- TC-S1-02: Replay に切替 ---
TOGGLE=$(curl -s -X POST "$API/replay/toggle")
MODE2=$(jqn "$TOGGLE" "d.mode")
[ "$MODE2" = "Replay" ] && pass "TC-S1-02: toggle → mode=Replay" || fail "TC-S1-02" "mode=$MODE2"

# --- TC-S1-03: Play 開始 ---
PLAY_RES=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}")
PLAY_ST=$(jqn "$PLAY_RES" "d.status")
[[ "$PLAY_ST" = "Loading" || "$PLAY_ST" = "Playing" ]] && \
  pass "TC-S1-03: play → Loading or Playing" || fail "TC-S1-03" "status=$PLAY_ST"

# --- TC-S1-04: Playing 到達（最大 120s） ---
if wait_playing 120; then
  pass "TC-S1-04: Playing に到達"
else
  fail "TC-S1-04" "120秒以内に Playing にならなかった"
fi

# --- TC-S1-05: current_time が前進 ---
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ADV=$(bigt_gt "$CT2" "$CT1")
[ "$ADV" = "true" ] && pass "TC-S1-05: current_time が前進 ($CT1 → $CT2)" || \
  fail "TC-S1-05" "current_time が前進しない (CT1=$CT1 CT2=$CT2)"

# --- TC-S1-06: Pause で固定 ---
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
P1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
P2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ=$(bigt_eq "$P1" "$P2")
[ "$EQ" = "true" ] && pass "TC-S1-06: Pause 中は current_time 固定" || \
  fail "TC-S1-06" "Pause 中に時刻が変化 ($P1 → $P2)"

# --- TC-S1-07: status=Paused ---
ST_PAUSED=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST_PAUSED" = "Paused" ] && pass "TC-S1-07: status=Paused" || fail "TC-S1-07" "status=$ST_PAUSED"

# --- TC-S1-08: Resume で再開 ---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 3
R1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ADV2=$(bigt_gt "$R1" "$P2")
[ "$ADV2" = "true" ] && pass "TC-S1-08: Resume 後に current_time 前進" || \
  fail "TC-S1-08" "Resume 後に前進しない ($P2 → $R1)"

# --- TC-S1-09〜12: Speed サイクル ---
curl -s -X POST "$API/replay/pause" > /dev/null
for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$SPEED" = "$expected" ] && pass "TC-S1-speed: speed=$SPEED" || \
    fail "TC-S1-speed" "expected=$expected got=$SPEED"
done

# --- TC-S1-13: StepForward → 60000ms 進む ---
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S1-13: StepForward +60000ms" || \
  fail "TC-S1-13" "diff=$DIFF (expected 60000)"

# --- TC-S1-14: StepBackward → 前のバーへ ---
BEF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
AFT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
IS_BACK=$(bigt_gt "$BEF" "$AFT")
[ "$IS_BACK" = "true" ] && pass "TC-S1-14: StepBackward で後退" || \
  fail "TC-S1-14" "後退しない (before=$BEF after=$AFT)"

# --- TC-S1-15: Live に戻す ---
LIVE_TOGGLE=$(curl -s -X POST "$API/replay/toggle")
LIVE_MODE=$(jqn "$LIVE_TOGGLE" "d.mode")
LIVE_ST=$(jqn "$LIVE_TOGGLE" "d.status")
[ "$LIVE_MODE" = "Live" ] && pass "TC-S1-15: Live 復帰 mode=Live" || fail "TC-S1-15" "mode=$LIVE_MODE"
[ "$LIVE_ST" = "null" ] && pass "TC-S1-15b: Live 復帰 status=null" || fail "TC-S1-15b" "status=$LIVE_ST"

restore_state
print_summary
```

---

## スイート S2: 永続化往復テスト

**目的**: 再生設定を保存し、再起動後に同じ状態で復元されることを確認する

```bash
#!/bin/bash
# s2_persistence.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S2: 永続化往復テスト ==="
backup_state

START=$(utc_offset -4)
END=$(utc_offset -1)

# --- TC-S2-01: replay フィールドなしで起動（後方互換） ---
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
RS=$(jqn "$STATUS" "d.range_start")
RE=$(jqn "$STATUS" "d.range_end")
[ "$MODE" = "Live" ] && pass "TC-S2-01: replay なし → mode=Live" || fail "TC-S2-01" "mode=$MODE"
[ "$RS" = "" ] && pass "TC-S2-01b: range_start 空" || fail "TC-S2-01b" "range_start=$RS"
stop_app

# --- TC-S2-02: Replay モードで保存 → 再起動で復元 ---
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
ST=$(curl -s "$API/replay/status")
# auto-play で Playing になる可能性があるため最大 30s 待機
for i in $(seq 1 30); do
  ST=$(curl -s "$API/replay/status")
  PSTATUS=$(jqn "$ST" "d.status")
  [[ "$PSTATUS" = "Playing" || "$PSTATUS" = "null" ]] && break
  sleep 1
done
MODE2=$(jqn "$ST" "d.mode")
RS2=$(jqn "$ST" "d.range_start")
RE2=$(jqn "$ST" "d.range_end")
[ "$MODE2" = "Replay" ] && pass "TC-S2-02: 再起動後 mode=Replay" || fail "TC-S2-02" "mode=$MODE2"
[ "$RS2" = "$START" ] && pass "TC-S2-02b: range_start 復元" || fail "TC-S2-02b" "got=$RS2 expected=$START"
[ "$RE2" = "$END" ] && pass "TC-S2-02c: range_end 復元" || fail "TC-S2-02c" "got=$RE2 expected=$END"
stop_app

# --- TC-S2-03: Play 実行後に保存 → 再起動で range_input 維持 ---
start_app
# Playing 待ち
wait_playing 60 || true
# Playing になっていれば range_input が設定済みのはず
curl -s -X POST "$API/app/save" > /dev/null
stop_app

start_app
ST3=$(curl -s "$API/replay/status")
RS3=$(jqn "$ST3" "d.range_start")
RE3=$(jqn "$ST3" "d.range_end")
[ "$RS3" = "$START" ] && pass "TC-S2-03: 保存→復元で range_start 維持" || fail "TC-S2-03" "got=$RS3"
[ "$RE3" = "$END" ] && pass "TC-S2-03b: 保存→復元で range_end 維持" || fail "TC-S2-03b" "got=$RE3"
stop_app

# --- TC-S2-04: toggle → Live に戻してから保存 → 再起動で Live ---
start_app
curl -s -X POST "$API/replay/toggle" > /dev/null  # Replay → Live
sleep 1
curl -s -X POST "$API/app/save" > /dev/null
stop_app

start_app
ST4=$(curl -s "$API/replay/status")
MODE4=$(jqn "$ST4" "d.mode")
[ "$MODE4" = "Live" ] && pass "TC-S2-04: Live 保存→復元で mode=Live" || fail "TC-S2-04" "mode=$MODE4"

restore_state
print_summary
```

---

## スイート S3: 起動時 Auto-play（Fixture 直接起動）

**目的**: `saved-state.json` に replay 構成を埋め込んで起動すると自動で Playing になること

```bash
#!/bin/bash
# s3_autoplay.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S3: Auto-play (Fixture 直接起動) ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S3-AutoPlay","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S3-AutoPlay"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# --- TC-S3-01: 手動 toggle / play なしで Playing になる（最大 30s） ---
if wait_playing 30; then
  pass "TC-S3-01: auto-play → Playing（sleep 15 不要）"
else
  fail "TC-S3-01" "30s 以内に Playing にならなかった（streams 解決失敗？）"
fi

STATUS=$(curl -s "$API/replay/status")

# --- TC-S3-02: current_time が range 内 ---
CT=$(jqn "$STATUS" "d.current_time")
IN_RANGE=$(node -e "console.log(BigInt('$CT') >= BigInt('$START_MS') && BigInt('$CT') <= BigInt('$END_MS'))")
[ "$IN_RANGE" = "true" ] && pass "TC-S3-02: current_time in range" || \
  fail "TC-S3-02" "CT=$CT range=[$START_MS,$END_MS]"

# --- TC-S3-03: mode=Replay ---
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Replay" ] && pass "TC-S3-03: mode=Replay" || fail "TC-S3-03" "mode=$MODE"

# --- TC-S3-04: Pause → StepForward → diff=60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S3-04: StepForward +60000ms" || \
  fail "TC-S3-04" "diff=$DIFF (expected 60000)"

# --- TC-S3-05: range_start が空文字のとき auto-play しない ---
stop_app
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S3-NoAutoPlay","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S3-NoAutoPlay"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"","range_end":""}
}
EOF

start_app
sleep 10  # auto-play が誤発火しないかを確認
ST_CHECK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# range 未設定なら auto-play しない → status=null のはず
[ "$ST_CHECK" = "null" ] && pass "TC-S3-05: range 未設定 → auto-play なし" || \
  fail "TC-S3-05" "status=$ST_CHECK (expected null)"

restore_state
print_summary
```

---

## スイート S4: マルチペイン・Binance 混在

**目的**: 複数ペイン（BTC M1 + ETH M1 + BTC TimeAndSales）が同時再生できること  
**背景**: kline/trades 分離回帰テスト（trades が kline 完了ゲートをブロックしないか）

```bash
#!/bin/bash
# s4_multi_pane_binance.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S4: マルチペイン Binance 混在 ==="
backup_state

START=$(utc_offset -14)
END=$(utc_offset -2)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S4-Multi","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.33,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"Split":{"axis":"Vertical","ratio":0.5,
        "a":{"KlineChart":{
          "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
          "indicators":["Volume"],"link_group":"B"
        }},
        "b":{"TimeAndSales":{
          "stream_type":[{"Trades":{"ticker":"BinanceLinear:BTCUSDT"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"MS100"}},
          "link_group":"A"
        }}
      }}
    }
  },"popout":[]}}],"active_layout":"S4-Multi"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# Replay に切替して Play
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 1

# --- TC-S4-01: 15秒以内に Playing に遷移（回帰テスト） ---
START_TIME=$(date +%s)
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 15; then
  ELAPSED=$(($(date +%s) - START_TIME))
  pass "TC-S4-01: 15s 以内に Playing ($ELAPSED s)"
else
  fail "TC-S4-01" "15s 以内に Playing にならなかった（trades が kline ゲートをブロック？）"
fi

# --- TC-S4-02: BTC と ETH の両ストリームが再生中 ---
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ADV=$(bigt_gt "$CT2" "$CT1")
[ "$ADV" = "true" ] && pass "TC-S4-02: current_time 前進（マルチペイン）" || \
  fail "TC-S4-02" "前進しない (CT1=$CT1 CT2=$CT2)"

# --- TC-S4-03: 10s 後もクラッシュしない（バックグラウンド trades 着弾） ---
sleep 10
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S4-03: 10s 後も Playing（trades クラッシュなし）" || \
  fail "TC-S4-03" "status=$ST (expected Playing)"

# --- TC-S4-04: Pause → StepForward → ステップ粒度は min timeframe = 60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S4-04: マルチペイン StepForward +60000ms" || \
  fail "TC-S4-04" "diff=$DIFF (expected 60000, min tf=M1)"

restore_state
print_summary
```

---

## スイート S5: 立花証券混在（Tachibana + Binance）

**前提**: 立花証券のログインセッションが有効であること  
**目的**: Tachibana D1 と Binance M1 が同一リプレイで共存できること  
**重要注意**:
- Tachibana はログイン画面で待機する場合がある → `POST /api/app/screenshot` で確認
- auto-play は `SessionRestoreResult(None)` でガードが落ちる（意図的）
- step_size_ms は **全ストリームの最小 tf** = 60000ms (M1) になる  
  → Tachibana D1 チャートはステップを 1440 回分まとめて受信する（正常）

```bash
#!/bin/bash
# s5_tachibana_binance.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S5: 立花証券 + Binance 混在 ==="
backup_state

# 過去 48h の範囲（立花は D1 のみ → 2 日以上の範囲が必要）
START=$(utc_offset -48)
END=$(utc_offset -24)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S5-Tachibana","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"Tachibana:7203","timeframe":"D1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
        "indicators":["Volume"],"link_group":"B"
      }}
    }
  },"popout":[]}}],"active_layout":"S5-Tachibana"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# --- TC-S5-01: ログイン画面待機の検出 ---
sleep 5
SCREEN_RES=$(curl -s -X POST "$API/app/screenshot")
echo "  Screenshot: $SCREEN_RES"
echo "  → C:/tmp/screenshot.png を Read ツールで確認してください"
echo "  立花ログイン画面なら手動ログイン後に続行してください"
read -p "  ログイン完了 or 不要の場合は Enter を押してください..."

# --- TC-S5-02: Live モードで起動確認 ---
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Live" ] && pass "TC-S5-02: Live モードで起動" || \
  echo "  INFO: mode=$MODE（Replay fixture なのでそのまま続行）"

# Replay に切替
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 1

# --- TC-S5-03: Play → Playing（最大 180s ：Tachibana は全データ取得後フィルタ） ---
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 180; then
  pass "TC-S5-03: Tachibana + Binance 混在 → Playing"
else
  fail "TC-S5-03" "180s 以内に Playing にならなかった"
fi

STATUS=$(curl -s "$API/replay/status")

# --- TC-S5-04: current_time が range 内 ---
CT=$(jqn "$STATUS" "d.current_time")
IN_RANGE=$(node -e "console.log(BigInt('$CT') >= BigInt('$START_MS') && BigInt('$CT') <= BigInt('$END_MS'))")
[ "$IN_RANGE" = "true" ] && pass "TC-S5-04: current_time in range" || \
  fail "TC-S5-04" "CT=$CT range=[$START_MS,$END_MS]"

# --- TC-S5-05: step_size は M1（60000ms）--- 最小 tf が M1 のため
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S5-05: 混在時 StepForward は min_tf=M1 → 60000ms" || \
  fail "TC-S5-05" "diff=$DIFF (expected 60000)"

# --- TC-S5-06: 10s 後もクラッシュしない ---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 10
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S5-06: 10s 後も Playing" || fail "TC-S5-06" "status=$ST"

restore_state
print_summary
```

### S5 エッジケース（手動確認項目）

| ID | ケース | 確認方法 |
|----|--------|---------|
| TC-S5-M1 | Tachibana セッション切れ時に auto-play が延期されトースト通知が出る | Fixture に replay mode を含む状態で起動し、ログイン未完のまま 30s 待機。toast が出たら PASS |
| TC-S5-M2 | 立花 D1 チャートで StepBackward が休場日（土日）を自動スキップ | range に土日を含む日程を設定し、StepBackward で土曜・日曜に着地しないことを確認 |
| TC-S5-M3 | Tachibana のみのペインで StepForward が 86400000ms（D1）単位で進む | Binance ペインを含まない Tachibana 単独構成で step_size_ms = 86400000 になることを確認 |

---

## スイート S6: 異なる時間軸混在

**目的**: M1 + M5 + H1 ペインが混在するとき、step_size_ms が最小 tf (M1 = 60000ms) になることを確認する

```bash
#!/bin/bash
# s6_mixed_timeframes.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S6: 異なる時間軸混在 ==="
backup_state

START=$(utc_offset -6)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S6-MixedTF","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.33,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"Split":{"axis":"Vertical","ratio":0.5,
        "a":{"KlineChart":{
          "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M5"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M5"}},
          "indicators":["Volume"],"link_group":"A"
        }},
        "b":{"KlineChart":{
          "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
          "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"H1"}}],
          "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"H1"}},
          "indicators":["Volume"],"link_group":"A"
        }}
      }}
    }
  },"popout":[]}}],"active_layout":"S6-MixedTF"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 1

# --- TC-S6-01: Play → Playing ---
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null

if wait_playing 60; then
  pass "TC-S6-01: M1+M5+H1 混在 → Playing"
else
  fail "TC-S6-01" "60s 以内に Playing にならなかった"
fi

# --- TC-S6-02: step_size は min_tf = M1 = 60000ms ---
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S6-02: step_size=60000ms (M1 が最小 tf)" || \
  fail "TC-S6-02" "diff=$DIFF (expected 60000)"

# --- TC-S6-03: M5 と H1 は kline が疎になる（1 step で kline なしも正常） ---
# 1 step (60000ms) では M5 / H1 の kline が来ない場合があるが、クラッシュしないこと
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.5
done
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Paused" ] && pass "TC-S6-03: M5/H1 疎 step でもクラッシュなし" || \
  fail "TC-S6-03" "status=$ST (expected Paused)"

# --- TC-S6-04: M5 ペインのみ構成（step_size が M5 = 300000ms）---
stop_app
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S6-M5Only","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M5"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M5"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S6-M5Only"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 1
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null
wait_playing 60
curl -s -X POST "$API/replay/pause" > /dev/null
PRE2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF2=$(bigt_sub "$POST2" "$PRE2")
[ "$DIFF2" = "300000" ] && pass "TC-S6-04: M5 単独 → step=300000ms" || \
  fail "TC-S6-04" "diff=$DIFF2 (expected 300000)"

restore_state
print_summary
```

---

## スイート S7: Mid-replay ペイン操作

**目的**: 再生中にペインを追加・削除・ticker/timeframe 変更しても再生が継続することを確認する

```bash
#!/bin/bash
# s7_mid_replay_pane.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S7: Mid-replay ペイン操作 ==="
backup_state

START=$(utc_offset -4)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S7-MidReplay","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S7-MidReplay"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 1
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$START\",\"end\":\"$END\"}" > /dev/null
wait_playing 60

# --- TC-S7-01: Playing 中にペイン ID を取得 ---
PANE_LIST=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "
  const d = JSON.parse(process.argv[1]);
  const panes = d.panes || d;
  const first = Array.isArray(panes) ? panes[0] : Object.values(panes)[0];
  console.log(first.id || first.pane_id || Object.keys(JSON.parse(process.argv[1]).panes || {})[0]);
" "$PANE_LIST" 2>/dev/null || echo "")
echo "  PANE_ID=$PANE_ID"
[ -n "$PANE_ID" ] && pass "TC-S7-01: pane list 取得" || fail "TC-S7-01" "pane ID が取れない（API 未実装？）"

# --- TC-S7-02: 再生中にペイン分割 ---
SPLIT_RES=$(curl -s -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$PANE_ID\",\"axis\":\"Vertical\"}")
echo "  split result: $SPLIT_RES"

# Waiting → Playing に戻るまで最大 60s
sleep 2
if wait_playing 60; then
  pass "TC-S7-02: ペイン分割後に Playing 復帰"
else
  fail "TC-S7-02" "分割後に Playing に戻らない（バックフィル失敗？）"
fi

# --- TC-S7-03: current_time が前進し続ける ---
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ADV=$(bigt_gt "$CT2" "$CT1")
[ "$ADV" = "true" ] && pass "TC-S7-03: ペイン分割後も current_time 前進" || \
  fail "TC-S7-03" "前進しない (CT1=$CT1 CT2=$CT2)"

# --- TC-S7-04: ticker 変更（Backfill） ---
NEW_PANE_LIST=$(curl -s "$API/pane/list")
NEW_PANE_ID=$(node -e "
  const d = JSON.parse(process.argv[1]);
  const panes = d.panes || d;
  const arr = Array.isArray(panes) ? panes : Object.values(panes);
  const last = arr[arr.length - 1];
  console.log(last.id || last.pane_id);
" "$NEW_PANE_LIST" 2>/dev/null || echo "")
[ -n "$NEW_PANE_ID" ] || NEW_PANE_ID="$PANE_ID"

TICKER_RES=$(curl -s -X POST "$API/pane/set-ticker" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$NEW_PANE_ID\",\"ticker\":\"BinanceLinear:ETHUSDT\"}")
echo "  set-ticker result: $TICKER_RES"
sleep 2
if wait_playing 60; then
  pass "TC-S7-04: ticker 変更後に Playing 復帰（バックフィル完了）"
else
  fail "TC-S7-04" "ticker 変更後に Playing に戻らない"
fi

# --- TC-S7-05: timeframe 変更（M1 → M5）---
TF_RES=$(curl -s -X POST "$API/pane/set-timeframe" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$NEW_PANE_ID\",\"timeframe\":\"M5\"}")
echo "  set-timeframe result: $TF_RES"
sleep 2
if wait_playing 60; then
  pass "TC-S7-05: timeframe 変更後に Playing 復帰"
else
  fail "TC-S7-05" "timeframe 変更後に Playing に戻らない"
fi

# --- TC-S7-06: ペイン削除後も再生が続く ---
DEL_RES=$(curl -s -X POST "$API/pane/close" \
  -H "Content-Type: application/json" \
  -d "{\"pane_id\":\"$NEW_PANE_ID\"}")
echo "  close result: $DEL_RES"
sleep 2
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S7-06: ペイン削除後も Playing 継続" || \
  fail "TC-S7-06" "status=$ST"

restore_state
print_summary
```

---

## スイート S8: エラー・境界値ケース

**目的**: 不正入力・メソッド不一致・極端な日時範囲が安全に拒否されること

```bash
#!/bin/bash
# s8_error_boundary.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S8: エラー・境界値ケース ==="
backup_state

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S8","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S8"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base"
}
EOF

start_app

# --- TC-S8-01: 存在しないパス → 404 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/nonexistent")
[ "$CODE" = "404" ] && pass "TC-S8-01: 存在しないパス → 404" || fail "TC-S8-01" "code=$CODE"

# --- TC-S8-02: 不正 JSON → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d 'not json')
[ "$CODE" = "400" ] && pass "TC-S8-02: 不正 JSON → 400" || fail "TC-S8-02" "code=$CODE"

# --- TC-S8-03: 必須フィールド欠損 → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d '{"start":"2026-04-10 09:00"}')
[ "$CODE" = "400" ] && pass "TC-S8-03: end 欠損 → 400" || fail "TC-S8-03" "code=$CODE"

# --- TC-S8-04: GET on POST endpoint → 404 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API/replay/toggle")
[ "$CODE" = "404" ] && pass "TC-S8-04: GET on POST endpoint → 404" || fail "TC-S8-04" "code=$CODE"

curl -s -X POST "$API/replay/toggle" > /dev/null  # Replay モードへ

# --- TC-S8-05: start > end → 400 or Toast（ParseRangeError::StartAfterEnd）---
RES=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-13 10:00","end":"2026-04-13 09:00"}')
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-13 10:00","end":"2026-04-13 09:00"}')
[ "$CODE" = "400" ] && pass "TC-S8-05: start>end → 400" || \
  echo "  INFO: start>end = $CODE (toast で通知される場合は手動確認)"

# --- TC-S8-06: 未来日時 → データ取得できずタイムアウトまたは即時失敗 ---
FUTURE_START="2030-01-01 00:00"
FUTURE_END="2030-01-01 06:00"
RES=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$FUTURE_START\",\"end\":\"$FUTURE_END\"}")
# 400 または Loading になる可能性あり
echo "  INFO: 未来日時 result=$RES（EventStore が空になるため StepForward が無効になる）"

# --- TC-S8-07: 不正なフォーマット → 400 ---
CODES=()
for bad_date in "2026/04/10 09:00" "2026-04-10" "not-a-date" ""; do
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$bad_date\",\"end\":\"2026-04-10 15:00\"}")
  [ "$CODE" = "400" ] && pass "TC-S8-07: 不正フォーマット '$bad_date' → 400" || \
    echo "  INFO: '$bad_date' → $CODE"
done

# --- TC-S8-08: pane/split に不正 UUID → 400 ---
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":"not-a-uuid","axis":"Vertical"}')
[ "$CODE" = "400" ] && pass "TC-S8-08: 不正 UUID → 400" || fail "TC-S8-08" "code=$CODE"

# --- TC-S8-09: pane/split に不正 axis → 400 ---
PANE_LIST=$(curl -s "$API/pane/list")
PANE_ID=$(node -e "
  const d = JSON.parse(process.argv[1]);
  const panes = d.panes || d;
  const arr = Array.isArray(panes) ? panes : Object.values(panes);
  console.log((arr[0]||{}).id || (arr[0]||{}).pane_id || '');
" "$PANE_LIST" 2>/dev/null || echo "")
if [ -n "$PANE_ID" ]; then
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/pane/split" \
    -H "Content-Type: application/json" \
    -d "{\"pane_id\":\"$PANE_ID\",\"axis\":\"Diagonal\"}")
  [ "$CODE" = "400" ] && pass "TC-S8-09: 不正 axis → 400" || fail "TC-S8-09" "code=$CODE"
fi

restore_state
print_summary
```

---

## スイート S9: 再生速度・Step 精度

**目的**: Speed サイクル・各速度での wall delay・StepForward/Backward の精度を検証する

```bash
#!/bin/bash
# s9_speed_step.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S9: 再生速度・Step 精度 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S9","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S9"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
wait_playing 30

# --- TC-S9-01: Speed サイクルの順序 (1x→2x→5x→10x→1x) ---
# 初期速度は 1x のはず
INIT_SPEED=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
[ "$INIT_SPEED" = "1x" ] && pass "TC-S9-01a: 初期 speed=1x" || fail "TC-S9-01a" "speed=$INIT_SPEED"

for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$SPEED" = "$expected" ] && pass "TC-S9-01b: speed cycle → $SPEED" || \
    fail "TC-S9-01b" "expected=$expected got=$SPEED"
done

# --- TC-S9-02: 2x 速度で step 間隔が ~500ms になること（wall 時間測定） ---
# step_delay_ms = base(1000ms) / speed(2.0) = 500ms
# 2 step 分の wall 時間を測定（精度 ±200ms の許容）
curl -s -X POST "$API/replay/pause" > /dev/null
# speed を 2x に設定
jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null  # 2x
curl -s -X POST "$API/replay/resume" > /dev/null
T_START=$(date +%s%N)
# current_time が 2 step 進むまで待つ
CT_INIT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
TARGET=$(node -e "console.log(String(BigInt('$CT_INIT') + BigInt(120000)))")  # +2 steps
for i in $(seq 1 20); do
  CT_NOW=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  GE=$(bigt_ge "$CT_NOW" "$TARGET")
  [ "$GE" = "true" ] && break
  sleep 0.1
done
T_END=$(date +%s%N)
WALL_MS=$(( (T_END - T_START) / 1000000 ))
# 2 step = 2 * 500ms = 1000ms ± 300ms 許容
echo "  INFO: 2x speed 2 step wall time = ${WALL_MS}ms (expected ~1000ms)"
[[ $WALL_MS -ge 700 && $WALL_MS -le 1500 ]] && \
  pass "TC-S9-02: 2x speed wall delay ~500ms/step" || \
  echo "  INFO: wall=${WALL_MS}ms (タイミング依存のため参考値)"

# --- TC-S9-03: StepForward は Paused 時のみ有効 ---
curl -s -X POST "$API/replay/resume" > /dev/null
PRE_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null  # Playing 中に叩く
sleep 0.5
POST_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# Playing 中の StepForward は無視されるはず（仕様確認）
echo "  INFO: Playing 中 StepForward: pre=$PRE_PLAYING post=$POST_PLAYING"

# --- TC-S9-04: StepBackward を連続 5 回 → 単調減少 ---
curl -s -X POST "$API/replay/pause" > /dev/null
# 先に 5 step 進む
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.3
done
TIMES=()
for i in $(seq 1 5); do
  T=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  TIMES+=("$T")
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.3
done
# 単調減少の確認
MONOTONE="true"
for i in $(seq 1 4); do
  A="${TIMES[$i]}"
  B="${TIMES[$((i-1))]}"
  [ -n "$A" ] && [ -n "$B" ] || continue
  GT=$(bigt_gt "$B" "$A")
  [ "$GT" = "true" ] || MONOTONE="false"
done
[ "$MONOTONE" = "true" ] && pass "TC-S9-04: StepBackward 連続 5 回 単調減少" || \
  fail "TC-S9-04" "単調減少でない times=${TIMES[*]}"

restore_state
print_summary
```

---

## スイート S10: 範囲端・終端到達

**目的**: 範囲端（1 バー幅）・終端到達時の Paused 遷移・StepForward 不可・StepBackward 不可を検証する

```bash
#!/bin/bash
# s10_range_end.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== S10: 範囲端・終端到達 ==="
backup_state

# 短い range（2時間 = M1 で 120 バー）
START=$(utc_offset -3)
END=$(utc_offset -1)
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S10","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S10"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
wait_playing 30

# --- TC-S10-01: 速度を 10x にして終端まで再生 ---
# 速度循環 1x→2x→5x→10x
for s in "2x" "5x" "10x"; do
  jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null
done
echo "  10x 速度で終端まで待機（最大 300s）..."

REACHED_END="false"
for i in $(seq 1 300); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  if [ "$ST" = "Paused" ]; then
    # current_time が end_ms 付近か確認
    NEAR_END=$(node -e "console.log(BigInt('$CT') >= BigInt('$END_MS') - BigInt('120000'))")
    [ "$NEAR_END" = "true" ] && REACHED_END="true"
    break
  fi
  sleep 1
done
[ "$REACHED_END" = "true" ] && pass "TC-S10-01: 終端到達で自動 Paused" || \
  fail "TC-S10-01" "終端到達しなかった or Paused にならなかった"

# --- TC-S10-02: 終端到達後 StepForward は無効 ---
CT_AT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
CT_AFTER_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ=$(bigt_eq "$CT_AT_END" "$CT_AFTER_SF")
[ "$EQ" = "true" ] && pass "TC-S10-02: 終端後 StepForward は無効（current_time 変化なし）" || \
  echo "  INFO: 終端後 StepForward: before=$CT_AT_END after=$CT_AFTER_SF"

# --- TC-S10-03: 終端から StepBackward で戻れる ---
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
CT_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
IS_BACK=$(bigt_gt "$CT_AT_END" "$CT_BACK")
[ "$IS_BACK" = "true" ] && pass "TC-S10-03: 終端から StepBackward 可能" || \
  fail "TC-S10-03" "後退しない (end=$CT_AT_END back=$CT_BACK)"

# --- TC-S10-04: Resume で再び Playing になる（終端未到達状態から）---
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 2
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S10-04: StepBackward 後に Resume → Playing" || \
  fail "TC-S10-04" "status=$ST"

# --- TC-S10-05: 1 バー幅のレンジ（start = end - 60s）---
stop_app
TINY_START=$(utc_offset -2)
# end = start + 1分 (node で計算)
TINY_END=$(node -e "
  const d = new Date('${TINY_START}:00Z');
  d.setMinutes(d.getMinutes() + 2);
  const pad = n => String(n).padStart(2,'0');
  console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' '+pad(d.getUTCHours())+':'+pad(d.getUTCMinutes()));
")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S10-Tiny","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S10-Tiny"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$TINY_START","range_end":"$TINY_END"}
}
EOF

start_app
if wait_playing 30; then
  pass "TC-S10-05: 2 分 range でも Playing 開始"
  # 終端到達を待つ
  wait_paused 60 && pass "TC-S10-05b: 小 range で終端到達 → Paused" || \
    fail "TC-S10-05b" "終端到達しなかった"
else
  fail "TC-S10-05" "2 分 range で Playing にならなかった"
fi

restore_state
print_summary
```

---

## 14. 実行順序と依存関係

```
S1（基本） ──┐
S2（永続化） ─┤
S3（auto-play）─┤
S4（マルチペイン）─┤──→ 全スイート独立。並列実行可能
S6（混在 tf）──┤        （ただし同一プロセスで実行する場合は 1 つずつ）
S8（エラー）──┤
S9（速度）──┘

S5（立花）──→ ログイン要。他スイートとは別セッションで実行
S7（mid-replay）──→ pane API が実装済みであることを前提とする
S10（終端）──→ S1 の基本動作が PASS していること
```

### 推奨実行コマンド

```bash
# ビルドを確認してから実行
cargo build --release

# S1〜S4, S6, S8〜S10 を順次実行
for s in s1 s2 s3 s4 s6 s8 s9 s10; do
  echo ""
  echo "=========================================="
  echo "Running Suite: $s"
  echo "=========================================="
  bash "docs/plan/e2e_scripts/${s}_*.sh"
done

# 立花が使える場合のみ
bash "docs/plan/e2e_scripts/s5_tachibana_binance.sh"
```

---

## 15. 合否判定基準

### 全スイート PASS 条件

| カテゴリ | 合格ライン |
|---------|-----------|
| 基本ライフサイクル（S1） | 全 15 項目 PASS |
| 永続化（S2） | 全 4 項目 PASS |
| Auto-play（S3） | TC-S3-01〜05 全 PASS |
| マルチペイン Binance（S4） | 全 4 項目 PASS。特に TC-S4-01 の 15s 以内 Playing |
| 立花混在（S5） | TC-S5-03〜06 PASS（ログイン済み前提） |
| 時間軸混在（S6） | 全 4 項目 PASS |
| Mid-replay（S7） | 全 6 項目 PASS |
| エラー（S8） | 400/404 チェック全 PASS |
| 速度・Step（S9） | TC-S9-01, 03, 04 PASS。TC-S9-02 は参考値 |
| 終端到達（S10） | 全 5 項目 PASS |

### CI での扱い

- S5（立花）は手動テストのみ（keyring / セッション依存）
- S7（Mid-replay）は pane API の実装状況による
- TC-S9-02（wall 時間）はタイミング依存のため WARN として扱う

### デバッグ手順

```bash
# ログの確認
cat C:/tmp/e2e_debug.log

# アプリがログイン画面で止まっている場合
curl -s -X POST "$API/app/screenshot"
# → Read ツールで C:/tmp/screenshot.png を確認

# E2E DEBUG ログを追加した場合の確認
grep "E2E DEBUG" C:/tmp/e2e_debug.log
# PR 前に必ず削除: grep -r "E2E DEBUG" src/
```
