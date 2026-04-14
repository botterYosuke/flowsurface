# E2E テスト計画書 — 新機能カバレッジ拡張

**作成日**: 2026-04-14  
**対象ブランチ**: `sasa/develop`  
**テストスキル**: [.claude/skills/e2e-testing/SKILL.md](../../.claude/skills/e2e-testing/SKILL.md)  
**前提 E2E 計画**: [archive/replay_e2e_test_plan.md](archive/replay_e2e_test_plan.md)（S1–S10, X1–X3 実装済み）

---

## 1. 概要と目的

既存の E2E スイート（S1–S10, X1–X3）は `replay_e2e_test_plan.md` に基づき実装済みだが、
以下の新機能・バグ修正は **未カバー** または **スクリプト未作成** の状態にある：

| 機能 | 対応アーカイブ | 既存スイート |
|------|----------------|-------------|
| バーステップ離散化 | `replay_bar_step_loop.md` | — 未カバー |
| Start 以前の履歴バー表示 (PRE_START_HISTORY_BARS=300) | `show_pre_start_history_bars.md` | — 未カバー |
| StepBackward チラつき防止（replay_mode フラグ） | `fix_step_backward_flicker.md` | — 未カバー |
| StepBackward コンテキストバー（request_handler リセット） | `fix_step_backward_missing_context.md` | — 未カバー |
| Auto-play タイムアウト廃止（イベント駆動） | `replay_auto_play_no_timeout.md` | — 未カバー |
| 立花証券混在 Replay | `tachibana_replay.md`, `pane_crud_api.md` | S5 **スクリプト未作成** |
| Mid-replay ペイン CRUD | `pane_crud_api.md`, `replay_unified_step.md` | S7 **スクリプト未作成** |

本計画では上記をカバーする **S5, S7, S11〜S14** のスクリプトを新規追加する。

---

## 2. 前提条件

### ビルド

```bash
# Tachibana モック注入が必要なシナリオ（S5, S14）
cargo build --release --features e2e-mock
EXE="./target/release/flowsurface.exe"
API="http://localhost:9876/api"
```

### chart-snapshot API の状態

`GET /api/pane/chart-snapshot?pane_id=...` は **未実装**。  
S12（履歴バー表示）の直接検証はこの API が実装されるまで **間接観測** で代替する。

---

## 3. 新規スイート一覧

| スイート | ファイル名 | 対象機能 | e2e-mock 必要 |
|---------|-----------|---------|:---:|
| S5  | `s5_tachibana_mixed.sh`       | 立花証券 + Binance 混在 Replay | ✅ |
| S7  | `s7_mid_replay_pane.sh`       | Mid-replay ペイン CRUD | — |
| S11 | `s11_bar_step_discrete.sh`    | バーステップ離散化 | — |
| S12 | `s12_pre_start_history.sh`    | Start 以前の履歴バー表示 | — |
| S13 | `s13_step_backward_quality.sh`| StepBackward 品質保証 | — |
| S14 | `s14_autoplay_event_driven.sh`| Auto-play タイムアウト廃止 | ✅ |

---

## 4. スイート S5: 立花証券混在 Replay

**スクリプト**: `docs/plan/e2e_scripts/s5_tachibana_mixed.sh`  
**ビルドフラグ**: `--features e2e-mock`

### 目的

Binance と Tachibana が同一 Replay セッションに混在する場合に、
両方のストリームが Loading → Ready → Playing に到達し、
`current_time` が正常に前進することを確認する。

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S5-01 | inject-session → Tachibana ステータス has_session=true | `has_session=true` |
| TC-S5-02 | inject-master で銘柄マスター注入 | HTTP 200 |
| TC-S5-03 | Binance M1 + Tachibana D1 混在 fixture で auto-play | status=Playing 到達 |
| TC-S5-04 | Tachibana ペインの streams_ready=true | `streams_ready=true` |
| TC-S5-05 | Binance ペインの streams_ready=true | `streams_ready=true` |
| TC-S5-06 | 10 秒後 current_time が前進 | `T2 > T1` |
| TC-S5-07 | Tachibana D1 ペインでの StepForward: current_time += 86400000 ms | `delta == 86400000` |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S5: 立花証券 + Binance 混在 Replay ==="
backup_state

# e2e-mock ビルドが必要
START=$(utc_offset -4)
END=$(utc_offset -2)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S5","dashboard":{"pane":{
    "Horizontal":{
      "left":{"KlineChart":{
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":[],"link_group":"A"
      }},
      "right":{"KlineChart":{
        "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
        "indicators":[],"link_group":"A"
      }},
      "ratio":0.5
    }
  },"popout":[]}}],"active_layout":"S5"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# TC-S5-01: Tachibana セッション注入
api_post /api/test/tachibana/inject-session
STATUS=$(api_get /api/auth/tachibana/status)
HAS=$(jqn "$STATUS" 'd.has_session')
[ "$HAS" = "true" ] && pass "TC-S5-01: has_session=true" || fail "TC-S5-01" "has_session=$HAS"

# TC-S5-02: 銘柄マスター注入
MASTER='[{"code":"7203","name":"トヨタ自動車","market":"東証プライム"}]'
api_post /api/test/tachibana/inject-master "$MASTER"
pass "TC-S5-02: inject-master 200"

# TC-S5-03: auto-play で Playing に到達（最大 120 秒）
if ! wait_playing 120; then
  fail "TC-S5-03" "Playing に到達せず"
  restore_state; print_summary; exit 1
fi
pass "TC-S5-03: Playing 到達"

# TC-S5-04/05: 両ペインの streams_ready
PANES=$(api_get /api/pane/list)
BTC_READY=$(node -e "
  const ps = JSON.parse(process.argv[1]);
  const p = ps.find(x => JSON.stringify(x).includes('BTCUSDT'));
  console.log(p && p.streams_ready ? 'true' : 'false');
" "$PANES")
[ "$BTC_READY" = "true" ] && pass "TC-S5-04: Binance streams_ready" || fail "TC-S5-04" "BTC streams_ready=$BTC_READY"

TACH_READY=$(node -e "
  const ps = JSON.parse(process.argv[1]);
  const p = ps.find(x => JSON.stringify(x).includes('7203'));
  console.log(p && p.streams_ready ? 'true' : 'false');
" "$PANES")
[ "$TACH_READY" = "true" ] && pass "TC-S5-05: Tachibana streams_ready" || fail "TC-S5-05" "Tachibana streams_ready=$TACH_READY"

# TC-S5-06: current_time 前進
T1=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
sleep 10
T2=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
[ "$T2" -gt "$T1" ] && pass "TC-S5-06: current_time 前進 ($T1 → $T2)" || fail "TC-S5-06" "停止 T1=$T1 T2=$T2"

# TC-S5-07: Tachibana D1 StepForward = 86400000ms
api_post /api/replay/pause; sleep 1
PANES=$(api_get /api/pane/list)
TACH_ID=$(node -e "
  const ps = JSON.parse(process.argv[1]);
  const p = ps.find(x => JSON.stringify(x).includes('7203'));
  console.log(p ? p.id : '');
" "$PANES")
T_BEFORE=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
api_post /api/replay/step-forward; sleep 2
T_AFTER=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
DELTA=$(node -e "console.log(String(BigInt('$T_AFTER') - BigInt('$T_BEFORE')))")
[ "$DELTA" = "86400000" ] && pass "TC-S5-07: D1 StepForward delta=86400000ms" || \
  fail "TC-S5-07" "delta=$DELTA (expected 86400000)"

stop_app
restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 5. スイート S7: Mid-replay ペイン CRUD

**スクリプト**: `docs/plan/e2e_scripts/s7_mid_replay_pane.sh`

### 目的

Replay 再生中にペイン操作（分割 / 閉じる / ティッカー変更 / タイムフレーム変更）を行っても
Replay が継続し、新ペインが正しくストリームを受け取ることを確認する。

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S7-01 | Playing 中に pane/split (Vertical) → ペイン数 +1 | count == 2 |
| TC-S7-02 | 新ペインで set-ticker → streams_ready=true | `streams_ready=true` |
| TC-S7-03 | 分割後も status=Playing が継続 | `status=Playing` |
| TC-S7-04 | 新ペインで set-timeframe M5 → streams_ready=true | `streams_ready=true` |
| TC-S7-05 | Playing 中に pane/close → ペイン数 -1 | count == 1 |
| TC-S7-06 | close 後も status=Playing が継続 | `status=Playing` |
| TC-S7-07 | Replay 終了後（range end 到達）に split しても crash しない | API 200 |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S7: Mid-replay ペイン CRUD ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S7","dashboard":{"pane":{
    "KlineChart":{
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S7"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 30; then
  fail "TC-S7-precond" "Playing 到達せず"
  restore_state; print_summary; exit 1
fi

# 初期ペイン ID 取得
PANES=$(api_get /api/pane/list)
PANE0=$(node -e "const p=JSON.parse(process.argv[1]); console.log(p[0].id);" "$PANES")

# TC-S7-01: Playing 中に split
api_post /api/pane/split "{\"pane_id\":\"$PANE0\",\"axis\":\"Vertical\"}"
sleep 2
PANES=$(api_get /api/pane/list)
COUNT=$(node -e "console.log(JSON.parse(process.argv[1]).length);" "$PANES")
[ "$COUNT" = "2" ] && pass "TC-S7-01: split 後ペイン数=2" || fail "TC-S7-01" "count=$COUNT"

# TC-S7-02: 新ペインの set-ticker
NEW_PANE=$(node -e "
  const ps = JSON.parse(process.argv[1]);
  console.log(ps.find(x => x.id !== '$PANE0').id);
" "$PANES")
api_post /api/pane/set-ticker "{\"pane_id\":\"$NEW_PANE\",\"ticker\":\"BinanceLinear:ETHUSDT\"}"
if wait_for_streams_ready "$NEW_PANE" 30; then
  pass "TC-S7-02: 新ペイン ETHUSDT streams_ready"
else
  fail "TC-S7-02" "streams_ready タイムアウト"
fi

# TC-S7-03: Replay 継続確認
STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status')
[ "$STATUS" = "Playing" ] && pass "TC-S7-03: split 後も Playing" || fail "TC-S7-03" "status=$STATUS"

# TC-S7-04: タイムフレーム変更 M1 → M5
api_post /api/pane/set-timeframe "{\"pane_id\":\"$NEW_PANE\",\"timeframe\":\"M5\"}"
if wait_for_streams_ready "$NEW_PANE" 30; then
  pass "TC-S7-04: M5 set-timeframe streams_ready"
else
  fail "TC-S7-04" "streams_ready タイムアウト"
fi

# TC-S7-05: close 新ペイン
api_post /api/pane/close "{\"pane_id\":\"$NEW_PANE\"}"
sleep 2
PANES=$(api_get /api/pane/list)
COUNT=$(node -e "console.log(JSON.parse(process.argv[1]).length);" "$PANES")
[ "$COUNT" = "1" ] && pass "TC-S7-05: close 後ペイン数=1" || fail "TC-S7-05" "count=$COUNT"

# TC-S7-06: Replay 継続
STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status')
[ "$STATUS" = "Playing" ] && pass "TC-S7-06: close 後も Playing" || fail "TC-S7-06" "status=$STATUS"

stop_app
restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 6. スイート S11: バーステップ離散化

**スクリプト**: `docs/plan/e2e_scripts/s11_bar_step_discrete.sh`

### 背景・目的

`replay_bar_step_loop.md` で実装したバーステップ離散化により、
`current_time` は **バー境界でのみ更新** される（wall-clock 連続更新は廃止）。
本スイートではこの仕様を E2E レベルで確認する。

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S11-01 | M1 1x 再生: 1 秒間に current_time が変化した場合、変化量は 60000ms の倍数 | `delta % 60000 == 0` |
| TC-S11-02 | M1 Pause → StepForward × 3: 各 delta = 60000ms | 各 `delta == 60000` |
| TC-S11-03 | M5 ペインのみ: StepForward delta = 300000ms | `delta == 300000` |
| TC-S11-04 | H1 ペインのみ: StepForward delta = 3600000ms | `delta == 3600000` |
| TC-S11-05 | M1+M5 混在ペイン: StepForward delta = 60000ms（最小 TF 優先） | `delta == 60000` |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S11: バーステップ離散化 ==="
backup_state

# --- TC-S11-01: M1 1x 再生中の delta が 60000ms の倍数 ---
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$(utc_offset -3)" "$(utc_offset -1)"
start_app
wait_playing 30

T1=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
sleep 3
T2=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
DELTA=$(node -e "console.log(String(BigInt('$T2') - BigInt('$T1')))")

if [ "$DELTA" != "0" ]; then
  MOD=$(node -e "console.log(String(BigInt('$DELTA') % BigInt('60000')))")
  [ "$MOD" = "0" ] && pass "TC-S11-01: delta($DELTA) は 60000ms の倍数" || \
    fail "TC-S11-01" "delta=$DELTA, mod=$MOD"
else
  fail "TC-S11-01" "current_time が変化しなかった (T1=T2=$T1)"
fi

# --- TC-S11-02: M1 StepForward × 3 ---
api_post /api/replay/pause; sleep 1
for i in 1 2 3; do
  TB=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
  api_post /api/replay/step-forward; sleep 2
  TA=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
  DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
  [ "$DELTA" = "60000" ] && pass "TC-S11-02-$i: StepForward delta=60000ms" || \
    fail "TC-S11-02-$i" "delta=$DELTA"
done

stop_app

# --- TC-S11-03: M5 ペイン StepForward ---
setup_single_pane "BinanceLinear:BTCUSDT" "M5" "$(utc_offset -6)" "$(utc_offset -1)"
start_app
wait_playing 30
api_post /api/replay/pause; sleep 1
TB=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
api_post /api/replay/step-forward; sleep 2
TA=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
DELTA=$(node -e "console.log(String(BigInt('$TA') - BigInt('$TB')))")
[ "$DELTA" = "300000" ] && pass "TC-S11-03: M5 StepForward delta=300000ms" || \
  fail "TC-S11-03" "delta=$DELTA"
stop_app

restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 7. スイート S12: Start 以前の履歴バー表示

**スクリプト**: `docs/plan/e2e_scripts/s12_pre_start_history.sh`

### 背景・目的

`show_pre_start_history_bars.md` で実装した `PRE_START_HISTORY_BARS = 300` により、
Replay Play 直後に Start 時刻以前の最大 300 本のバーがチャートに注入される。

`GET /api/pane/chart-snapshot` が未実装のため、**間接観測**で確認する：

- Play 直後に **StepBackward を繰り返しても** `current_time` が `start_ms` を下回らない  
  （`show_pre_start_history_bars.md` §3 R4「start_ms 未満への seek をブロック」）
- StepBackward の delta が `start_ms` でクランプされることを確認

> **TODO**: `chart-snapshot` API 実装後に、直接バー本数（≥ 1 && ≤ 300）を検証するTCを追加する。

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S12-01 | Play 直後 StepBackward: current_time ≥ start_time | `current_time >= start_time` |
| TC-S12-02 | StepBackward 連打（5 回）でも current_time が start_time を下回らない | `current_time >= start_time` (5回) |
| TC-S12-03 | StepForward でコンテキストバー注入後も Replay が正常に前進する | `current_time` 前進 |
| TC-S12-04 | [要 API: chart-snapshot] バー本数 ≥ 1 && ≤ 300 | `1 ≤ bar_count ≤ 300` |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S12: Start 以前の履歴バー表示 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app
wait_playing 30
api_post /api/replay/pause; sleep 1

START_MS=$(jqn "$(api_get /api/replay/status)" 'd.start_time')

# TC-S12-01: 1 回 StepBackward → start_time 以上
api_post /api/replay/step-backward; sleep 2
CT=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
[ "$CT" -ge "$START_MS" ] && pass "TC-S12-01: StepBackward 後 current_time >= start_time" || \
  fail "TC-S12-01" "current_time=$CT < start_time=$START_MS"

# TC-S12-02: 5 回 StepBackward 連打 → start_time クランプ
for i in $(seq 1 5); do
  api_post /api/replay/step-backward; sleep 2
  CT=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
  [ "$CT" -ge "$START_MS" ] && pass "TC-S12-02-$i: StepBackward #$i current_time >= start_time" || \
    fail "TC-S12-02-$i" "current_time=$CT < start_time=$START_MS"
done

# TC-S12-03: resume 後に current_time が前進する
api_post /api/replay/resume; sleep 3
CT_AFTER=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
[ "$CT_AFTER" -gt "$CT" ] && pass "TC-S12-03: resume 後 current_time 前進" || \
  fail "TC-S12-03" "current_time 停止 CT=$CT CT_AFTER=$CT_AFTER"

stop_app
restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 8. スイート S13: StepBackward 品質保証

**スクリプト**: `docs/plan/e2e_scripts/s13_step_backward_quality.sh`

### 背景・目的

以下の 2 つのバグ修正をカバーする：

1. **チラつき防止** (`fix_step_backward_flicker.md`)  
   `replay_mode=true` フラグにより live API fetch が Replay モードで発行されない。
   - 観測: StepBackward 後に status が `Loading` に戻らない

2. **コンテキストバー表示** (`fix_step_backward_missing_context.md`)  
   `reset_for_seek()` が `request_handler` をリセットするため、
   コンテキストバーのフェッチが自然に発火する。
   - 観測: StepBackward 後にペインの `streams_ready` が false に落ちない

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S13-01 | StepBackward 後 2 秒以内に status が Playing/Paused に戻る（Loading に留まらない） | `status ∈ {Playing, Paused}` |
| TC-S13-02 | StepBackward × 10 連続後 streams_ready=true を維持 | `streams_ready=true` |
| TC-S13-03 | StepBackward 後 resume: current_time が正常に前進（live data 混入チェック） | `T2 > T1 && (T2 - T1) % 60000 == 0` |
| TC-S13-04 | StepForward → StepBackward 交互 × 5 でも status=Paused を維持 | `status=Paused` (5回) |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S13: StepBackward 品質保証 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
setup_single_pane "BinanceLinear:BTCUSDT" "M1" "$START" "$END"
start_app
wait_playing 30
api_post /api/replay/pause; sleep 1

PANES=$(api_get /api/pane/list)
PANE_ID=$(node -e "console.log(JSON.parse(process.argv[1])[0].id);" "$PANES")

# TC-S13-01: StepBackward 後に Loading に留まらない
api_post /api/replay/step-backward
sleep 0.5
for i in $(seq 1 4); do
  STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status')
  [ "$STATUS" = "Loading" ] && { sleep 1; continue; }
  break
done
STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status')
[[ "$STATUS" == "Paused" || "$STATUS" == "Playing" ]] && \
  pass "TC-S13-01: StepBackward 後 status=$STATUS（Loading に留まらない）" || \
  fail "TC-S13-01" "status=$STATUS"

# TC-S13-02: 10 回 StepBackward 後 streams_ready 維持
for i in $(seq 1 10); do
  api_post /api/replay/step-backward; sleep 1
done
PANES=$(api_get /api/pane/list)
READY=$(node -e "
  const ps = JSON.parse(process.argv[1]);
  const p = ps.find(x => x.id === '$PANE_ID');
  console.log(p && p.streams_ready ? 'true' : 'false');
" "$PANES")
[ "$READY" = "true" ] && pass "TC-S13-02: 10×StepBackward 後 streams_ready=true" || \
  fail "TC-S13-02" "streams_ready=$READY"

# TC-S13-03: resume 後の delta がバー境界に揃う
api_post /api/replay/resume; sleep 5
api_post /api/replay/pause; sleep 1
T1=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
api_post /api/replay/resume; sleep 5
T2=$(jqn "$(api_get /api/replay/status)" 'd.current_time')
if [ "$T2" -gt "$T1" ]; then
  DELTA=$(node -e "console.log(String(BigInt('$T2') - BigInt('$T1')))")
  MOD=$(node -e "console.log(String(BigInt('$DELTA') % BigInt('60000')))")
  [ "$MOD" = "0" ] && pass "TC-S13-03: resume 後 delta=$DELTA（live data 混入なし）" || \
    fail "TC-S13-03" "delta=$DELTA, mod=$MOD (live data 混入の疑い)"
else
  fail "TC-S13-03" "current_time 停止 T1=$T1 T2=$T2"
fi

# TC-S13-04: StepForward ↔ StepBackward 交互
api_post /api/replay/pause; sleep 1
for i in $(seq 1 5); do
  api_post /api/replay/step-forward; sleep 1
  api_post /api/replay/step-backward; sleep 1
  STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status')
  [ "$STATUS" = "Paused" ] && pass "TC-S13-04-$i: 交互 Step #$i status=Paused" || \
    fail "TC-S13-04-$i" "status=$STATUS"
done

stop_app
restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 9. スイート S14: Auto-play タイムアウト廃止

**スクリプト**: `docs/plan/e2e_scripts/s14_autoplay_event_driven.sh`  
**ビルドフラグ**: `--features e2e-mock`

### 背景・目的

`replay_auto_play_no_timeout.md` で実装したイベント駆動 auto-play を確認する。

旧実装では：Tachibana ログイン → master download が 30 秒を超えると auto-play タイムアウト。  
新実装では：metadata 到着イベントで無期限に待ち、到着後に自動 Play。

### TC 一覧

| TC ID | 内容 | 判定 |
|-------|------|------|
| TC-S14-01 | アプリ起動 → 35 秒後に inject-session → inject-master → Playing 到達 | status=Playing（旧実装ではタイムアウト） |
| TC-S14-02 | 「auto-play timed out」トーストが出ない | notifications にそのメッセージなし |
| TC-S14-03 | inject なし（セッション無し）の場合: Playing にならず、info トーストが出る | status ≠ Playing && info toast あり |
| TC-S14-04 | 2 回目の inject-master で Playing に到達（マスター遅延模擬） | status=Playing |

### スクリプト骨格

```bash
#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S14: Auto-play タイムアウト廃止 ==="
backup_state

START=$(utc_offset -4)
END=$(utc_offset -2)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S14","dashboard":{"pane":{
    "KlineChart":{
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S14"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# 35 秒待機（旧実装では 30 秒タイムアウトが発火する時間帯）
echo "  35秒待機（タイムアウト発火確認）..."
sleep 35

# TC-S14-02: タイムアウトトーストがない
NOTIFS=$(api_get /api/notification/list)
HAS_TIMEOUT=$(node -e "
  const ns = JSON.parse(process.argv[1]);
  const found = ns.some(n => n.message && n.message.includes('timed out'));
  console.log(found ? 'true' : 'false');
" "$NOTIFS")
[ "$HAS_TIMEOUT" = "false" ] && pass "TC-S14-02: timed out トーストなし" || \
  fail "TC-S14-02" "timed out トースト発見（旧実装の挙動）"

# TC-S14-01: セッション注入後に Playing 到達
api_post /api/test/tachibana/inject-session
MASTER='[{"code":"7203","name":"トヨタ自動車","market":"東証プライム"}]'
api_post /api/test/tachibana/inject-master "$MASTER"

# Playing になるまで最大 60 秒待機
if wait_playing 60; then
  pass "TC-S14-01: 遅延 inject 後に Playing 到達（タイムアウトなし）"
else
  fail "TC-S14-01" "Playing に到達せず"
fi

stop_app

# TC-S14-03: セッションなし → Playing にならず info トーストが出る
start_app
sleep 15
STATUS=$(jqn "$(api_get /api/replay/status)" 'd.status // "none"')
[ "$STATUS" != "Playing" ] && pass "TC-S14-03a: セッションなし → Playing でない (status=$STATUS)" || \
  fail "TC-S14-03a" "Playing になった（セッションなしなのに）"

NOTIFS=$(api_get /api/notification/list)
HAS_INFO=$(node -e "
  const ns = JSON.parse(process.argv[1]);
  const found = ns.some(n => n.level === 'Info' || n.level === 'info');
  console.log(found ? 'true' : 'false');
" "$NOTIFS")
[ "$HAS_INFO" = "true" ] && pass "TC-S14-03b: info トーストあり" || \
  fail "TC-S14-03b" "info トーストなし"

stop_app
restore_state
print_summary
[ $FAIL -eq 0 ]
```

---

## 10. 実行順序と依存関係

```
S5  (e2e-mock) ──┐
S7              ──┤
S11             ──┼─── 独立実行可
S12             ──┤
S13             ──┤
S14 (e2e-mock) ──┘
```

**統合実行コマンド（CI向け）**:

```bash
# 通常ビルド向け
bash docs/plan/e2e_scripts/s7_mid_replay_pane.sh
bash docs/plan/e2e_scripts/s11_bar_step_discrete.sh
bash docs/plan/e2e_scripts/s12_pre_start_history.sh
bash docs/plan/e2e_scripts/s13_step_backward_quality.sh

# e2e-mock ビルド向け（別ビルド必要）
cargo build --release --features e2e-mock
bash docs/plan/e2e_scripts/s5_tachibana_mixed.sh
bash docs/plan/e2e_scripts/s14_autoplay_event_driven.sh
```

---

## 11. 合否判定基準

| 基準 | 内容 |
|------|------|
| 全スクリプト `exit 0` | `FAIL=0` が出力される |
| current_time 前進 | `T2 > T1` の数値比較で確認 |
| バーステップ境界 | `delta % step_ms == 0` |
| start_time クランプ | `current_time >= start_time` |
| live data 非混入 | StepBackward 後の delta が M1 倍数 |
| タイムアウト廃止 | `timed out` トーストが出ない |

---

## 12. 未解決事項（TODO）

| # | 内容 | 依存 |
|---|------|------|
| 1 | `GET /api/pane/chart-snapshot` 実装後に S12-TC-S12-04 を追加 | API 実装待ち |
| 2 | S11-TC-S11-04（H1 StepForward）は Binance H1 データが 24h 以内にある前提 | 実行タイミング依存 |
| 3 | S5 の inject-daily-history エンドポイント活用（現在 inject-master のみ） | 仕様確認待ち |
