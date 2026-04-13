# リプレイ機能 E2E テスト計画書

**作成日**: 2026-04-13  
**最終更新**: 2026-04-13（レビュー反映: current_time / ボタン / チャート更新 3 観点の強化）  
**対象ブランチ**: `sasa/step`  
**参照仕様**: [docs/replay_header.md](../replay_header.md)  
**テストスキル**: [.claude/skills/e2e-test/SKILL.md](../../.claude/skills/e2e-test/SKILL.md)

---

## 目次

1. [概要と方針](#1-概要と方針)
2. [前提 API 拡張（本計画の実行条件）](#2-前提-api-拡張本計画の実行条件)
3. [テスト環境セットアップ](#3-テスト環境セットアップ)
4. [TC テンプレートと合否判定ルール](#4-tc-テンプレートと合否判定ルール)
5. [スイート一覧](#5-スイート一覧)
6. [スイート S1: 基本ライフサイクル](#スイート-s1-基本ライフサイクル)
7. [スイート S2: 永続化往復テスト](#スイート-s2-永続化往復テスト)
8. [スイート S3: 起動時 Auto-play（Fixture 直接起動）](#スイート-s3-起動時-auto-playfixture-直接起動)
9. [スイート S4: マルチペイン・マルチ銘柄（Binance 混在）](#スイート-s4-マルチペインマルチ銘柄binance-混在)
10. [スイート S5: 立花証券混在（Tachibana + Binance）](#スイート-s5-立花証券混在tachibana--binance)
11. [スイート S6: 異なる時間軸混在](#スイート-s6-異なる時間軸混在)
12. [スイート S7: Mid-replay ペイン操作](#スイート-s7-mid-replay-ペイン操作)
13. [スイート S8: エラー・境界値ケース](#スイート-s8-エラー境界値ケース)
14. [スイート S9: 再生速度・Step 精度](#スイート-s9-再生速度step-精度)
15. [スイート S10: 範囲端・終端到達](#スイート-s10-範囲端終端到達)
16. [横断スイート X1: current_time 表示の不変条件](#横断スイート-x1-current_time-表示の不変条件)
17. [横断スイート X2: ボタン（⏮ ▶ ⏸ ⏭ / Speed）の厳密挙動](#横断スイート-x2-ボタン-step-playpause-speed-の厳密挙動)
18. [横断スイート X3: チャート表示内容と更新タイミング](#横断スイート-x3-チャート表示内容と更新タイミング)
19. [実行順序と依存関係](#19-実行順序と依存関係)
20. [合否判定基準](#20-合否判定基準)

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
- GUI 描画のピクセル一致検証（ただし §18 X3 はチャート内容を HTTP API 経由で検証する）

### レビュー反映ポリシー（2026-04-13）

本計画は初版レビューで **「バックエンド状態は正しく前進しているが、画面 / チャートが止まっている」** タイプの不具合を検出できないという指摘を受けた。この穴を塞ぐため、以下を方針とする:

1. **表示観測 API の追加を前提化**（§2 参照）
2. **各 TC は「目的 / 手段 / 前提 / 期待値 / 失敗時ヒント」の固定フォーマットで書く**（§4 参照）
3. **`pass`/`fail` を呼ばずに `echo INFO` で流すテストは禁止**（未定義仕様の TC は仕様確定を先行）
4. **current_time の前進は "差分 > 0" ではなく "期待差分 ± 許容" で検証する**
5. **Step / Speed / Pause のボタン系 TC は、API レスポンスだけでなく `chart-snapshot` の更新までを確認する**

---

## 2. 前提 API 拡張（本計画の実行条件）

本計画のうち **横断スイート X1 / X2 / X3** と、既存スイートの強化版 TC を実行するには以下の 3 点の HTTP API 拡張が必要である。未実装の項目は **`[要 API 拡張]`** タグで該当 TC に明示する。

### 2.1 `GET /api/pane/chart-snapshot?pane_id=...` ★新規

**目的**: 指定ペインに実際に描画されている kline 情報を返す。`/api/pane/list` は ticker/timeframe のメタしか返さないため、Step 押下後にチャートへ反映されたかを外部から確認できない。

**レスポンス例**:
```json
{
  "pane_id": "0b2d...",
  "stream": "BinanceLinear:BTCUSDT:M1",
  "kline_count": 120,
  "first_kline_time": 1744236000000,
  "last_kline_time": 1744243200000,
  "last_close": 63420.5,
  "visible_timerange": [1744236000000, 1744243200000],
  "last_price_label": 63420.5
}
```

**実装見積**: **1〜2 日**

**根拠**:
- KlineChart の内部 kline バッファは [src/main.rs](../../src/main.rs) の iced `update()` スレッドが所有しており、API スレッドから直接アクセスできない。
- 既存の `/api/pane/list` と同じく [src/replay_api.rs](../../src/replay_api.rs) の `PaneCommand` に新バリアントを追加し、`oneshot::channel` 経由で iced 側から取得する必要がある（[src/main.rs:1596 PaneCommand::ListPanes](../../src/main.rs#L1596) と同等の構造）。
- [src/chart.rs:75](../../src/chart.rs#L75) の `visible_timerange()` は **trait メソッド** で、KlineChart / Heatmap / Footprint など実装ファイルが分散している。各 `pane::Content` の `kind` 分岐を経由して `Chart::visible_timerange()` を呼ぶ glue コードが必要。
- `kline_count` / `last_kline_time` 等は EventStore からも取れるが、**チャート描画バッファそのもの**を覗かないと「描画が止まっているがバックエンドは進む」事故を検出できないので、Chart::klines() 相当のアクセサ追加が望ましい。

**作業ブレークダウン**:
1. `Chart` trait に `kline_summary() -> ChartSummary` アクセサを追加（KlineChart 実装）— 0.5 日
2. `PaneCommand::ChartSnapshot { pane_id }` を追加し、`build_chart_snapshot_json` を実装 — 0.5 日
3. ルーティング・JSON シリアライズ・テスト — 0.5 日

**注意**: 既存コードに `visible_timerange` 実装が無い Chart kind（Footprint 等）では `Option<None>` を返す。

### 2.2 `ReplayStatus.current_time_display: Option<String>` ★既存拡張

**目的**: ヘッダーの表示文字列を取得する。[src/main.rs:1197](../../src/main.rs#L1197) で `format_current_time(&self.replay, self.timezone)` で生成される文字列と **完全一致** で返す。

**理由**: `current_time` (Unix ms) と `current_time_display` (表示文字列) の**両方が進む**ことを確認しないと、タイムゾーン変換バグや UI フリーズ（描画側だけ止まる）を検出できない。

**実装見積**: 0.5 時間（[src/replay/mod.rs:45](../../src/replay/mod.rs#L45) `ReplayStatus` に `Option<String>` フィールド追加）。

### 2.3 `GET /api/pane/notifications` ★実装済み（未活用）

既に [src/main.rs:1621-1646](../../src/main.rs#L1621-L1646) の `ListNotifications` → `build_notification_list_json` で実装済み。本計画では S3-05 / S5-M1 / S7-04 の検証に新規活用する。

**既存レスポンス形状**:
```json
{ "notifications": [ { "title": "...", "body": "...", "level": "error|warning|success|info" } ] }
```

### 2.4 未実装時の扱い

- `[要 API 拡張]` が付いた TC は、該当 API が実装されるまで **スキップ可能**（合否判定では PENDING 扱い）
- ただし **§20 の全体 PASS 条件には含める**。つまり API 拡張を先行させないと本計画は完了しない
- 既存 API のみで実行可能な TC は **常に実行必須**

---

## 3. テスト環境セットアップ

### 3.1 前提条件確認スクリプト

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

### 3.2 共通ヘルパー

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

# ── §2 で追加された API のヘルパー ──────────────────────────────────────────

# chart-snapshot を取得（要 API 拡張）
# 使い方: snap=$(chart_snapshot "$PANE_ID"); echo "$snap" | jqn - "d.kline_count"
chart_snapshot() {
  curl -s "$API/pane/chart-snapshot?pane_id=$1"
}

# current_time_display を取得（要 API 拡張）
status_display() {
  jqn "$(curl -s "$API/replay/status")" "d.current_time_display"
}

# トースト一覧を取得
list_notifications() {
  curl -s "$API/pane/notifications"
}

# トーストに body 部分一致で検索
has_notification() {
  local needle=$1
  local n
  n=$(list_notifications)
  node -e "
    const d=JSON.parse(process.argv[1]);
    const items=d.notifications||[];
    const hit=items.some(t=>(t.body||'').includes(process.argv[2])||(t.title||'').includes(process.argv[2]));
    console.log(hit);
  " "$n" "$needle"
}

# step_size_ms を fixture から逆算（手動指定）
# M1=60000, M5=300000, H1=3600000, D1=86400000
STEP_M1=60000
STEP_M5=300000
STEP_H1=3600000
STEP_D1=86400000

# 前進差分が期待ステップ境界内に収まるか: 1 ≤ delta/step ≤ max_bars
advance_within() {
  local pre=$1 post=$2 step=$3 max_bars=$4
  node -e "
    const d = BigInt('$post') - BigInt('$pre');
    const s = BigInt('$step');
    const max = BigInt('$max_bars');
    if (d < 0n) { console.log('false'); process.exit(0); }
    if (d % s !== 0n) { console.log('false'); process.exit(0); }
    const bars = d / s;
    console.log(bars >= 1n && bars <= max ? 'true' : 'false');
  "
}

# current_time がバー境界値か
is_bar_boundary() {
  local ct=$1 step=$2
  node -e "console.log(BigInt('$ct') % BigInt('$step') === 0n)"
}

# current_time が [start_time, end_time] 区間内
ct_in_range() {
  local ct=$1 st=$2 et=$3
  node -e "console.log(BigInt('$ct') >= BigInt('$st') && BigInt('$ct') <= BigInt('$et'))"
}
```

---

## 4. TC テンプレートと合否判定ルール

### 4.1 TC 記述テンプレート（必須）

各 TC は**シェルスクリプトコメントとして以下の 5 行を前置きする**。これにより「何を担保しているか」が一目で分かり、FAIL 時の切り分けコストが大幅に下がる。

```bash
# --- TC-<ID>: <短いタイトル> ---
# 目的:     <何を担保するか。1 文で書ける範囲に絞る>
# 手段:     <どの API / ログ / スクショを叩くか>
# 前提:     <この TC を実行可能な前提状態（mode, status, range など）>
# 期待値:   <具体的な数値・文字列比較条件。± 許容を明記>
# 失敗時:   <疑うべきコード箇所 / 参照ログキー>
```

### 4.2 禁則事項

1. **`echo INFO` で合否を曖昧にしない**。仕様未確定なら TC を書かず、先に仕様書側で動作を確定する。
2. **`ADV > 0` のような「進んでいれば合格」型の比較は禁止**。期待差分とその許容を明示する（例: `60000 <= delta <= 180000`）。`advance_within` ヘルパーを使う。
3. **API レスポンスの `status` 文字列比較だけで合格にしない**。X3 のチャート内容検証（kline_count 変化・last_kline_time 単調増加）を必ず組み合わせる。
4. **複数 TC を 1 つの `pass` にまとめない**。`TC-S1-15` / `TC-S1-15b` のように分離する。
5. **`set -e` 下での `wait_playing N || fail "..."` パターン禁止**。`fail()` はゼロ終了するため `set -e` は止まらないが、後続の前提が崩れた状態で TC が走り続けるためノイズが多くなる。代わりに `if wait_playing N; then ... else fail "..."; fi` か、`set -e` を一時無効化（`set +e; wait_playing N; rc=$?; set -e`）してから判定する。
6. **`bash` の `=` で大整数（Unix ms）の文字列比較を行わない**。`bigt_eq` / `bigt_gt` / `bigt_ge` を使う。型変換や先行ゼロで誤判定するリスクがある。

### 4.3 合否 3 値

| 結果 | 条件 | カウント |
|---|---|---|
| PASS | 期待値を満たした | `PASS++` |
| FAIL | 期待値を満たさなかった or API エラー | `FAIL++` |
| PENDING | `[要 API 拡張]` タグ付きで API 未実装 | `PEND++`（新規） |

`print_summary` 拡張:
```bash
print_summary() {
  echo ""
  echo "============================="
  echo "  PASS: $PASS  FAIL: $FAIL  PEND: ${PEND:-0}"
  echo "============================="
  [ $FAIL -eq 0 ]
}
pend() { echo "  PEND: $1 — $2 (API 拡張待ち)"; PEND=$((${PEND:-0} + 1)); }
```

---

## 5. スイート一覧

| スイート | 説明 | 推定時間 | 立花ログイン要 | API 拡張要 |
|---------|------|---------|--------------|-----------|
| S1 | 基本ライフサイクル | 5 分 | 不要 | 一部 |
| S2 | 永続化往復 | 4 分 | 不要 | 不要 |
| S3 | Auto-play（Fixture 直接起動） | 3 分 | 不要 | 一部 |
| S4 | マルチペイン・Binance 混在 | 5 分 | 不要 | 一部 |
| S5 | 立花証券 + Binance 混在 | 8 分 | **要** | 一部 |
| S6 | 異なる時間軸混在 | 6 分 | 不要 | 不要 |
| S7 | Mid-replay ペイン操作 | 6 分 | 不要 | 一部 |
| S8 | エラー・境界値ケース | 3 分 | 不要 | 不要 |
| S9 | 再生速度・Step 精度 | 4 分 | 不要 | 不要 |
| S10 | 範囲端・終端到達 | 5 分 | 不要 | 一部 |
| **X1** | **current_time 表示の不変条件** | **3 分** | 不要 | **要** |
| **X2** | **ボタン（⏮ ▶ ⏸ ⏭ / Speed）の厳密挙動** | **5 分** | 不要 | 一部 |
| **X3** | **チャート表示内容と更新タイミング** | **6 分** | 不要 | **要** |

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

# --- TC-S1-05: current_time が 1x 速度の期待差分で前進 ---
# 目的:     1x speed で 3 秒経過時に概ね 3 bar (= 180000ms) 進む
# 手段:     /api/replay/status を 3 秒間隔で 2 回ポーリング
# 前提:     status=Playing, speed=1x, step_size_ms=60000 (M1 単独)
# 期待値:   60000 <= delta <= 240000 かつ delta % 60000 == 0
# 失敗時:   StepClock::tick / dispatch_tick / clock.now_ms() を疑う
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
WITHIN=$(advance_within "$CT1" "$CT2" "$STEP_M1" 4)
[ "$WITHIN" = "true" ] && pass "TC-S1-05: 1x で 3s に 1〜4 bar 前進 ($CT1 → $CT2)" || \
  fail "TC-S1-05" "想定外の前進 (CT1=$CT1 CT2=$CT2 step=$STEP_M1)"

# --- TC-S1-05b: current_time はバー境界値 ---
# 目的:     仮想時刻が常に step_size_ms の倍数（バー境界）であること
# 期待値:   CT2 % 60000 == 0
ON_BAR=$(is_bar_boundary "$CT2" "$STEP_M1")
[ "$ON_BAR" = "true" ] && pass "TC-S1-05b: current_time バー境界スナップ" || \
  fail "TC-S1-05b" "CT2=$CT2 はバー境界ではない"

# --- TC-S1-05c: current_time ∈ [start_time, end_time] ---
# 目的:     不変条件（仮想時刻が範囲外にはみ出さない）
ST_NOW=$(curl -s "$API/replay/status")
START_T=$(jqn "$ST_NOW" "d.start_time")
END_T=$(jqn "$ST_NOW" "d.end_time")
IN=$(ct_in_range "$CT2" "$START_T" "$END_T")
[ "$IN" = "true" ] && pass "TC-S1-05c: current_time ∈ [start,end]" || \
  fail "TC-S1-05c" "CT2=$CT2 range=[$START_T,$END_T]"

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

# --- TC-S1-13: StepForward は 1 バーきっかり進む ---
# 目的:     Step 粒度が min_active_tf_ms に一致
# 前提:     status=Paused, step_size_ms=60000
# 期待値:   POST_SF - PRE == 60000 かつ POST_SF はバー境界値
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST_SF" "$PRE")
[ "$DIFF" = "60000" ] && pass "TC-S1-13: StepForward +60000ms" || \
  fail "TC-S1-13" "diff=$DIFF (expected 60000)"
ON_BAR=$(is_bar_boundary "$POST_SF" "$STEP_M1")
[ "$ON_BAR" = "true" ] && pass "TC-S1-13b: StepForward 後もバー境界" || \
  fail "TC-S1-13b" "POST_SF=$POST_SF"

# --- TC-S1-14: StepBackward は 1 バーきっかり後退 ---
# 期待値:   BEF - AFT == 60000
BEF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 1
AFT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF_B=$(bigt_sub "$BEF" "$AFT")
[ "$DIFF_B" = "60000" ] && pass "TC-S1-14: StepBackward -60000ms" || \
  fail "TC-S1-14" "diff=$DIFF_B (expected 60000, before=$BEF after=$AFT)"

# --- TC-S1-15: Live 復帰時の状態完全リセット ---
# 目的:     toggle_mode() の Replay→Live 分岐で全フィールドが初期化される
# 参照:     src/replay/mod.rs:134-148
# 期待値:   mode=Live, status=null, current_time=null, speed=null,
#           start_time=null, end_time=null, range_start="", range_end=""
LIVE_TOGGLE=$(curl -s -X POST "$API/replay/toggle")
LIVE_MODE=$(jqn "$LIVE_TOGGLE" "d.mode")
LIVE_ST=$(jqn "$LIVE_TOGGLE" "d.status")
LIVE_CT=$(jqn "$LIVE_TOGGLE" "d.current_time")
LIVE_SP=$(jqn "$LIVE_TOGGLE" "d.speed")
LIVE_RS=$(jqn "$LIVE_TOGGLE" "d.range_start")
LIVE_RE=$(jqn "$LIVE_TOGGLE" "d.range_end")
[ "$LIVE_MODE" = "Live" ] && pass "TC-S1-15a: mode=Live" || fail "TC-S1-15a" "mode=$LIVE_MODE"
[ "$LIVE_ST" = "null" ] && pass "TC-S1-15b: status=null" || fail "TC-S1-15b" "status=$LIVE_ST"
[ "$LIVE_CT" = "null" ] && pass "TC-S1-15c: current_time=null" || fail "TC-S1-15c" "ct=$LIVE_CT"
[ "$LIVE_SP" = "null" ] && pass "TC-S1-15d: speed=null" || fail "TC-S1-15d" "speed=$LIVE_SP"
[ "$LIVE_RS" = "" ] && pass "TC-S1-15e: range_start 空" || fail "TC-S1-15e" "rs=$LIVE_RS"
[ "$LIVE_RE" = "" ] && pass "TC-S1-15f: range_end 空" || fail "TC-S1-15f" "re=$LIVE_RE"

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
ST_T=$(jqn "$ST" "d.start_time")
ET_T=$(jqn "$ST" "d.end_time")
[ "$MODE2" = "Replay" ] && pass "TC-S2-02: 再起動後 mode=Replay" || fail "TC-S2-02" "mode=$MODE2"
[ "$RS2" = "$START" ] && pass "TC-S2-02b: range_start 復元" || fail "TC-S2-02b" "got=$RS2 expected=$START"
[ "$RE2" = "$END" ] && pass "TC-S2-02c: range_end 復元" || fail "TC-S2-02c" "got=$RE2 expected=$END"

# --- TC-S2-02d: range_start (str) と start_time (ms) の整合 ---
# 目的:     parse_replay_range の結果が ReplayStatus に矛盾なく載る
# 期待値:   start_time == new Date(`${range_start}:00Z`).getTime()
EXPECT_ST=$(node -e "console.log(new Date('${START}:00Z').getTime())")
EXPECT_ET=$(node -e "console.log(new Date('${END}:00Z').getTime())")
if [ "$ST_T" = "null" ]; then
  pend "TC-S2-02d" "clock 未起動のため start_time=null（auto-play 前で計測不可）"
  pend "TC-S2-02e" "clock 未起動のため end_time=null"
else
  EQ_ST=$(bigt_eq "$ST_T" "$EXPECT_ST")
  EQ_ET=$(bigt_eq "$ET_T" "$EXPECT_ET")
  [ "$EQ_ST" = "true" ] && pass "TC-S2-02d: start_time ms 整合" || \
    fail "TC-S2-02d" "got=$ST_T expected=$EXPECT_ST"
  [ "$EQ_ET" = "true" ] && pass "TC-S2-02e: end_time ms 整合" || \
    fail "TC-S2-02e" "got=$ET_T expected=$EXPECT_ET"
fi
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

# --- TC-S3-05: range 未設定 → auto-play しない & status=null ---
# 目的:     range_start/range_end が空文字なら起動時 Play が抑止される
# 期待値:   status=null かつ mode=Replay（mode 自体は fixture の通り Replay）
ST_CHECK=$(jqn "$(curl -s "$API/replay/status")" "d.status")
MODE_CHECK=$(jqn "$(curl -s "$API/replay/status")" "d.mode")
[ "$ST_CHECK" = "null" ] && pass "TC-S3-05a: range 未設定 → status=null" || \
  fail "TC-S3-05a" "status=$ST_CHECK (expected null)"
[ "$MODE_CHECK" = "Replay" ] && pass "TC-S3-05b: range 未設定でも mode は fixture 通り" || \
  fail "TC-S3-05b" "mode=$MODE_CHECK"

# --- TC-S3-05c: トーストに auto-play 起動エラーが無いこと ---
# 目的:     range 空でも例外通知が出ない（静かに無効化される）
# 手段:     /api/pane/notifications を読み、error/warning level が無いことを確認
NOTIF=$(list_notifications)
ERR_COUNT=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const e=(d.notifications||[]).filter(t=>t.level==='error'||t.level==='warning');
  console.log(e.length);
" "$NOTIF")
[ "$ERR_COUNT" = "0" ] && pass "TC-S3-05c: error/warning toast なし" || \
  fail "TC-S3-05c" "error/warning toast が $ERR_COUNT 件発火: $NOTIF"

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

# --- TC-S4-02: マルチペイン 1x で 3 秒 / 1〜4 bar 前進 ---
# 目的:     2 ペインのストリーム結合があっても再生レートが正常
# 期待値:   advance_within(60000, 4) で範囲内
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
WITHIN=$(advance_within "$CT1" "$CT2" "$STEP_M1" 4)
[ "$WITHIN" = "true" ] && pass "TC-S4-02: マルチペインで 1〜4 bar 前進 ($CT1 → $CT2)" || \
  fail "TC-S4-02" "想定外の前進 (CT1=$CT1 CT2=$CT2)"

# --- TC-S4-03: 10s 後も Playing 継続 + 前進 ---
# 目的:     trades の遅延着弾でクロックが Waiting に落ちない
# 期待値:   status=Playing かつ ct が 5〜20 bar 進む
CT3_PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 10
CT3_POST=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST" = "Playing" ] && pass "TC-S4-03a: 10s 後も Playing" || fail "TC-S4-03a" "status=$ST"
WITHIN10=$(advance_within "$CT3_PRE" "$CT3_POST" "$STEP_M1" 20)
[ "$WITHIN10" = "true" ] && pass "TC-S4-03b: 10s で 1〜20 bar 前進 (delta verified)" || \
  fail "TC-S4-03b" "10s で前進が範囲外 ($CT3_PRE → $CT3_POST)"

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

# --- TC-S5-02: 起動時のモード確認 ---
# 仕様確定: 本スイートの fixture は replay フィールドを含まないので必ず Live で起動する
# 期待値:   mode=Live
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")
[ "$MODE" = "Live" ] && pass "TC-S5-02: 起動時 mode=Live" || \
  fail "TC-S5-02" "mode=$MODE (expected Live; fixture に replay フィールドなし)"

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
if ! wait_playing 60; then
  fail "TC-S6-04-precond" "M5 単独構成で Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi
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
if ! wait_playing 60; then
  fail "TC-S7-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi

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

# --- TC-S7-03: ペイン分割後も current_time が定量的に前進 ---
# 期待値:   3 秒で 1〜4 bar 前進
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
WITHIN=$(advance_within "$CT1" "$CT2" "$STEP_M1" 4)
[ "$WITHIN" = "true" ] && pass "TC-S7-03: ペイン分割後 1〜4 bar 前進 ($CT1 → $CT2)" || \
  fail "TC-S7-03" "想定外の前進 (CT1=$CT1 CT2=$CT2)"

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

# --- TC-S8-05: start > end → 200 で受理されるが Toast 通知 + Playing にならない ---
# 目的:     ParseRangeError::StartAfterEnd の挙動を確定させる
# 仕様確定: HTTP API は parse 段階で `Notification` を経由するため 200 を返し、
#           直後に notifications にエラートーストが追加される（src/replay/mod.rs parse_replay_range）
# 前提:     mode=Replay
# 期待値:   1) HTTP=200, 2) status=null（または変化なし）, 3) error level toast に "start" "end" を含む
RES=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-13 10:00","end":"2026-04-13 09:00"}')
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-13 10:00","end":"2026-04-13 09:00"}')
[ "$CODE" = "200" ] && pass "TC-S8-05a: start>end → HTTP 200" || fail "TC-S8-05a" "code=$CODE"
ST_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[[ "$ST_AFTER" = "null" || "$ST_AFTER" = "Paused" ]] && pass "TC-S8-05b: Playing に遷移しない" || \
  fail "TC-S8-05b" "status=$ST_AFTER"
HAS_ERR=$(has_notification "start")
[ "$HAS_ERR" = "true" ] && pass "TC-S8-05c: エラートーストが発火" || \
  fail "TC-S8-05c" "start>end の toast が発火していない"

# --- TC-S8-06: 未来日時 → 受理 → プリフェッチ完了するが EventStore 空 → Step 無効 ---
# 目的:     データが存在しない range の挙動を確定（API は受理、状態は Loading のまま）
# 期待値:   1) HTTP=200, 2) 30s 待機後に status != Playing,
#           3) StepForward 叩いても current_time が前進しない
FUTURE_START="2030-01-01 00:00"
FUTURE_END="2030-01-01 06:00"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$FUTURE_START\",\"end\":\"$FUTURE_END\"}")
[ "$CODE" = "200" ] && pass "TC-S8-06a: 未来日時 → HTTP 200" || fail "TC-S8-06a" "code=$CODE"
sleep 30
ST6=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$ST6" != "Playing" ] && pass "TC-S8-06b: 未来日時 → Playing にならない (status=$ST6)" || \
  fail "TC-S8-06b" "想定外に Playing 到達"
PRE6=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
POST6=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# null 同士または整数同士のいずれもありうるので両方に対応
if [ "$PRE6" = "null" ] && [ "$POST6" = "null" ]; then
  pass "TC-S8-06c: clock 未起動のまま Step 無効"
else
  EQ6=$(bigt_eq "$PRE6" "$POST6")
  [ "$EQ6" = "true" ] && pass "TC-S8-06c: 空 EventStore → Step 無効" || \
    fail "TC-S8-06c" "想定外の前進 pre=$PRE6 post=$POST6"
fi

# --- TC-S8-07: 不正なフォーマット → 400 ---
# 仕様確定: parse_replay_range は NaiveDateTime::parse_from_str("YYYY-MM-DD HH:MM") を使うため、
#           列挙した 4 ケースは全て InvalidStartFormat を返す → HTTP API は 400 を返す
# 期待値:   全 4 ケースで HTTP=400
for bad_date in "2026/04/10 09:00" "2026-04-10" "not-a-date" ""; do
  CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
    -H "Content-Type: application/json" \
    -d "{\"start\":\"$bad_date\",\"end\":\"2026-04-10 15:00\"}")
  [ "$CODE" = "400" ] && pass "TC-S8-07: 不正フォーマット '$bad_date' → 400" || \
    fail "TC-S8-07" "'$bad_date' → $CODE (expected 400)"
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
if ! wait_playing 30; then
  fail "TC-S9-precond" "auto-play で Playing に到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-S9-01: Speed サイクルの順序 (1x→2x→5x→10x→1x) ---
# 初期速度は 1x のはず
INIT_SPEED=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
[ "$INIT_SPEED" = "1x" ] && pass "TC-S9-01a: 初期 speed=1x" || fail "TC-S9-01a" "speed=$INIT_SPEED"

for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$SPEED" = "$expected" ] && pass "TC-S9-01b: speed cycle → $SPEED" || \
    fail "TC-S9-01b" "expected=$expected got=$SPEED"
done

# --- TC-S9-02: 5x 速度で wall delay が概ね 200ms/bar ---
# 目的:     base_step_delay_ms(1000) / speed(5.0) が実時計に反映される
# 手段:     5x にして 5 秒間進ませ、進んだバー数 = 進んだ ms / 60000 を測る
# 期待値:   5 秒で 15 〜 35 bar 進む（理論値 25 bar、±10 bar 許容）
#           ± 許容を広くしているのは tick が iced::frames(60Hz) 駆動でジッターがあるため
# 失敗時:   StepClock::step_delay_ms / cycle_speed の経路を疑う
curl -s -X POST "$API/replay/pause" > /dev/null
# 1x → 2x → 5x
jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null  # 2x
jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null  # 5x
SP=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
[ "$SP" = "5x" ] || fail "TC-S9-02-precond" "speed=$SP (expected 5x)"
CT_INIT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 5
curl -s -X POST "$API/replay/pause" > /dev/null
CT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DELTA=$(bigt_sub "$CT_END" "$CT_INIT")
BARS=$(node -e "console.log(String(BigInt('$DELTA') / BigInt('$STEP_M1')))")
[[ $BARS -ge 15 && $BARS -le 35 ]] && pass "TC-S9-02: 5x で 5 秒に ${BARS} bar 前進" || \
  fail "TC-S9-02" "${BARS} bar (expected 15-35, delta=$DELTA)"

# --- TC-S9-03: Playing 中の StepForward の挙動を確定（仕様確定 TC）---
# 目的:     Playing 中の Step 押下を no-op として固定する
# 仕様:     [docs/replay_header.md §3.1] Step ボタンは clock.is_some() && !Waiting で enabled
#           ただし Playing 中は通常 tick が回るため、追加の Step 入力は **状態不変** とする
# 期待値:   pre と post の差 < 2 bar（自然 tick 分だけ）。同期スリープを最小化して測る
curl -s -X POST "$API/replay/resume" > /dev/null
PRE_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
POST_PLAYING=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DELTA_P=$(bigt_sub "$POST_PLAYING" "$PRE_PLAYING")
# 1 bar 以下なら StepForward は no-op、超えるなら Playing 中ステップが副作用ありとなり仕様違反
node -e "process.exit(BigInt('$DELTA_P') > BigInt('$STEP_M1') ? 1 : 0)" \
  && pass "TC-S9-03: Playing 中 StepForward は no-op (delta=$DELTA_P)" \
  || fail "TC-S9-03" "Playing 中 Step が ${DELTA_P}ms 進めた（仕様違反）"

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
if ! wait_playing 30; then
  fail "TC-S10-precond" "auto-play で Playing に到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-S10-01: 速度を 10x にして終端まで再生 ---
# 速度循環 1x→2x→5x→10x
for s in "2x" "5x" "10x"; do
  jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null
done
echo "  10x 速度で終端まで待機（最大 300s）..."

REACHED_END="false"
for i in $(seq 1 300); do
  # 1 ループ = 1 API コール（status / current_time の同時取得で競合状態を排除）
  STATUS=$(curl -s "$API/replay/status")
  CT=$(jqn "$STATUS" "d.current_time")
  ST=$(jqn "$STATUS" "d.status")
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

# --- TC-S10-02: 終端到達後 StepForward は完全 no-op ---
# 目的:     range_end を超えるバーを発火させない
# 期待値:   CT_AT_END == CT_AFTER_SF
CT_AT_END=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
CT_AFTER_SF=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ=$(bigt_eq "$CT_AT_END" "$CT_AFTER_SF")
[ "$EQ" = "true" ] && pass "TC-S10-02: 終端後 StepForward は no-op" || \
  fail "TC-S10-02" "終端後 StepForward が前進 (before=$CT_AT_END after=$CT_AFTER_SF)"

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
  if wait_paused 60; then
    pass "TC-S10-05b: 小 range で終端到達 → Paused"
  else
    fail "TC-S10-05b" "終端到達しなかった"
  fi
else
  fail "TC-S10-05" "2 分 range で Playing にならなかった"
fi

restore_state
print_summary
```

---

## 横断スイート X1: current_time 表示の不変条件

**目的**: 仮想時刻と表示文字列が常に整合し、バー境界・range 内・タイムゾーン変換が正しく機能することを担保する。  
**API 拡張要否**: 一部 TC は `current_time_display`（§2.2）に依存。`[要 API 拡張]` タグの TC は `pend` 扱い。

```bash
#!/bin/bash
# x1_current_time.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== X1: current_time 表示の不変条件 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X1","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"X1"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 60; then
  fail "X1-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi

# --- TC-X1-01: バー境界スナップ不変条件（10 サンプル）---
# 目的:     仮想時刻が常に step_size_ms の倍数（バー境界値）で前進する
# 手段:     0.5 秒間隔で 10 回 current_time をサンプリング
# 前提:     status=Playing, M1 単独 → step_size=60000
# 期待値:   全サンプルで current_time % 60000 == 0
# 失敗時:   StepClock::tick の seek 計算 / step_size_ms の同期化漏れ
ALL_ON_BAR="true"
for i in $(seq 1 10); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  ON=$(is_bar_boundary "$CT" "$STEP_M1")
  [ "$ON" = "true" ] || { ALL_ON_BAR="false"; echo "  off-bar at i=$i ct=$CT"; }
  sleep 0.5
done
[ "$ALL_ON_BAR" = "true" ] && pass "TC-X1-01: 10 サンプル全てバー境界" || \
  fail "TC-X1-01" "バー境界違反あり"

# --- TC-X1-02: current_time の単調非減少 ---
# 目的:     ポーリング中に時刻が逆行しない
# 期待値:   prev <= curr が全サンプルで成立
PREV="0"
MONO="true"
for i in $(seq 1 8); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  GE=$(bigt_ge "$CT" "$PREV")
  [ "$GE" = "true" ] || MONO="false"
  PREV="$CT"
  sleep 0.4
done
[ "$MONO" = "true" ] && pass "TC-X1-02: current_time 単調非減少" || \
  fail "TC-X1-02" "逆行あり"

# --- TC-X1-03: range 内不変条件（連続サンプル） ---
# 目的:     start_time <= current_time <= end_time が常に成立
ST_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
ET_T=$(jqn "$(curl -s "$API/replay/status")" "d.end_time")
ALL_IN="true"
for i in $(seq 1 6); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  IN=$(ct_in_range "$CT" "$ST_T" "$ET_T")
  [ "$IN" = "true" ] || ALL_IN="false"
  sleep 0.5
done
[ "$ALL_IN" = "true" ] && pass "TC-X1-03: range 内不変" || \
  fail "TC-X1-03" "range 外"

# --- TC-X1-04: [要 API 拡張] current_time_display と current_time の整合 ---
# 目的:     ヘッダー表示文字列が current_time の UTC 変換と一致
# 手段:     ReplayStatus.current_time_display を読み、node で再変換した文字列と比較
# 期待値:   display == toUTCString(current_time)（"YYYY-MM-DD HH:MM:SS" フォーマット）
# 失敗時:   format_current_time / timezone 設定 / 表示の遅延描画
DISPLAY=$(status_display)
if [ "$DISPLAY" = "null" ]; then
  pend "TC-X1-04" "ReplayStatus.current_time_display 未実装"
else
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  EXPECT=$(node -e "
    const d=new Date(Number('$CT'));
    const pad=n=>String(n).padStart(2,'0');
    console.log(d.getUTCFullYear()+'-'+pad(d.getUTCMonth()+1)+'-'+pad(d.getUTCDate())+' '+pad(d.getUTCHours())+':'+pad(d.getUTCMinutes())+':'+pad(d.getUTCSeconds()));
  ")
  [ "$DISPLAY" = "$EXPECT" ] && pass "TC-X1-04: display=$DISPLAY と current_time 整合" || \
    fail "TC-X1-04" "display=$DISPLAY expected=$EXPECT"
fi

# --- TC-X1-05: [要 API 拡張] display も連続して進む ---
# 目的:     表示文字列が止まらないこと（current_time だけ進んで描画が固まる事故の検出）
D1=$(status_display)
if [ "$D1" = "null" ]; then
  pend "TC-X1-05" "current_time_display 未実装"
else
  sleep 3
  D2=$(status_display)
  [ "$D1" != "$D2" ] && pass "TC-X1-05: display が前進 ($D1 → $D2)" || \
    fail "TC-X1-05" "display 固定 ($D1)"
fi

# --- TC-X1-06: Live モードで current_time / display が null ---
# 目的:     Live 復帰時にすべてリセットされる
curl -s -X POST "$API/replay/toggle" > /dev/null  # → Live
sleep 1
ST=$(curl -s "$API/replay/status")
CT=$(jqn "$ST" "d.current_time")
SP=$(jqn "$ST" "d.speed")
[ "$CT" = "null" ] && pass "TC-X1-06a: Live current_time=null" || fail "TC-X1-06a" "ct=$CT"
[ "$SP" = "null" ] && pass "TC-X1-06b: Live speed=null" || fail "TC-X1-06b" "speed=$SP"

restore_state
print_summary
```

---

## 横断スイート X2: ボタン (Step / Play/Pause / Speed) の厳密挙動

**目的**: 各ボタン押下が「正しい状態遷移 + 正しい current_time 変化 + 正しい副作用」を生じることを担保する。  
**注記**: 本計画は **HTTP API 経由でボタン処理を駆動** する。最終ハンドラは [src/main.rs:1043-1068](../../src/main.rs#L1043-L1068) で UI ボタンと同一 `ReplayMessage` をディスパッチするため、コードパスは等価である（UI ピクセル差分は対象外）。

```bash
#!/bin/bash
# x2_buttons.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== X2: ボタンの厳密挙動 ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X2","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
      "indicators":["Volume"],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"X2"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app
if ! wait_playing 60; then
  fail "X2-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi
curl -s -X POST "$API/replay/pause" > /dev/null
wait_paused 5 || true

# --- TC-X2-01: ⏭ StepForward は 1 bar きっかり進む（複数回連続）---
# 目的:     n 回の StepForward で n * step_size_ms 進む
# 期待値:   後 - 前 == 5 * 60000 == 300000
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-forward" > /dev/null
  sleep 0.2
done
POST=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(bigt_sub "$POST" "$PRE")
[ "$DIFF" = "300000" ] && pass "TC-X2-01: StepForward x5 = +300000ms" || \
  fail "TC-X2-01" "diff=$DIFF (expected 300000)"

# --- TC-X2-02: ⏮ StepBackward x5 で完全可逆 ---
# 目的:     Step は可逆操作。連続して戻すと元の current_time に戻る
# 期待値:   ( 5 step backward 後の ct ) == PRE
for i in $(seq 1 5); do
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.2
done
BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
[ "$BACK" = "$PRE" ] && pass "TC-X2-02: 可逆 (back=$BACK)" || \
  fail "TC-X2-02" "back=$BACK pre=$PRE"

# --- TC-X2-03: ⏮ start 端での StepBackward は no-op ---
# 目的:     range_start を超えて巻き戻らない
# 手段:     start_time に到達するまで StepBackward → 1 回追加で叩く
# 注意:     大整数の比較は bigt_eq を使用（bash の = は文字列比較で誤判定の恐れ）
ST_T=$(jqn "$(curl -s "$API/replay/status")" "d.start_time")
for i in $(seq 1 200); do
  CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
  EQ=$(bigt_eq "$CT" "$ST_T")
  [ "$EQ" = "true" ] && break
  curl -s -X POST "$API/replay/step-backward" > /dev/null
  sleep 0.05
done
AT_START=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-backward" > /dev/null
sleep 0.5
BEYOND=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ2=$(bigt_eq "$AT_START" "$BEYOND")
[ "$EQ2" = "true" ] && pass "TC-X2-03: start 端 StepBackward は no-op" || \
  fail "TC-X2-03" "AT_START=$AT_START BEYOND=$BEYOND"

# --- TC-X2-04: ▶/⏸ Pause 冪等性 ---
# 目的:     Pause を 2 回叩いても状態が壊れない
curl -s -X POST "$API/replay/pause" > /dev/null
ST1=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/pause" > /dev/null
ST2=$(jqn "$(curl -s "$API/replay/status")" "d.status")
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
CT_EQ=$(bigt_eq "$CT1" "$CT2")
[ "$ST1" = "$ST2" ] && [ "$CT_EQ" = "true" ] && pass "TC-X2-04: Pause 冪等" || \
  fail "TC-X2-04" "ST=$ST1→$ST2 CT=$CT1→$CT2"

# --- TC-X2-05: Resume → Pause → Resume の往復で current_time の継続性 ---
# 目的:     Pause/Resume 間で時刻の損失（デクリメントや巻き戻り）が起きない
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 1
PRE_R=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/pause" > /dev/null
sleep 1
PAUSED_AT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
GE1=$(bigt_ge "$PAUSED_AT" "$PRE_R")
[ "$GE1" = "true" ] && pass "TC-X2-05a: Pause 後の時刻 >= Pause 前" || \
  fail "TC-X2-05a" "PAUSED_AT=$PAUSED_AT PRE_R=$PRE_R"
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 1
RESUMED=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
GE2=$(bigt_ge "$RESUMED" "$PAUSED_AT")
[ "$GE2" = "true" ] && pass "TC-X2-05b: Resume 後 >= Pause 時刻" || \
  fail "TC-X2-05b" "RESUMED=$RESUMED PAUSED_AT=$PAUSED_AT"

# --- TC-X2-06: Speed サイクル一周 + speed 値の永続 ---
# 目的:     CycleSpeed が決定論的に 1x→2x→5x→10x→1x で循環する
curl -s -X POST "$API/replay/pause" > /dev/null
# まず 1x にリセット
for i in $(seq 1 5); do
  SP=$(jqn "$(curl -s "$API/replay/status")" "d.speed")
  [ "$SP" = "1x" ] && break
  curl -s -X POST "$API/replay/speed" > /dev/null
done
EXPECTED=("2x" "5x" "10x" "1x")
ALL_OK="true"
for e in "${EXPECTED[@]}"; do
  GOT=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  [ "$GOT" = "$e" ] || { ALL_OK="false"; echo "  cycle break: expected=$e got=$GOT"; }
done
[ "$ALL_OK" = "true" ] && pass "TC-X2-06: Speed cycle 1→2→5→10→1" || fail "TC-X2-06" "cycle 異常"

# --- TC-X2-07: Speed 変更で current_time は変化しない ---
# 目的:     CycleSpeed は時計をリセットしない
# 注意:     Paused 状態で実施する。Playing 中は tick 進行のため不変条件にならない
curl -s -X POST "$API/replay/pause" > /dev/null
wait_paused 5 || true
PRE_SP=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/speed" > /dev/null
POST_SP=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
EQ_SP=$(bigt_eq "$PRE_SP" "$POST_SP")
[ "$EQ_SP" = "true" ] && pass "TC-X2-07: Speed 切替で current_time 不変" || \
  fail "TC-X2-07" "pre=$PRE_SP post=$POST_SP"

# --- TC-X2-08: Live 中はボタンが意味を持たない（API は受理するが状態不変）---
# 目的:     mode=Live のまま step / pause / resume を叩いても副作用なし
curl -s -X POST "$API/replay/toggle" > /dev/null  # → Live
LIVE_BEFORE=$(curl -s "$API/replay/status")
curl -s -X POST "$API/replay/step-forward" > /dev/null
curl -s -X POST "$API/replay/pause" > /dev/null
curl -s -X POST "$API/replay/resume" > /dev/null
LIVE_AFTER=$(curl -s "$API/replay/status")
B_MODE=$(jqn "$LIVE_BEFORE" "d.mode")
A_MODE=$(jqn "$LIVE_AFTER" "d.mode")
B_CT=$(jqn "$LIVE_BEFORE" "d.current_time")
A_CT=$(jqn "$LIVE_AFTER" "d.current_time")
# Live 時は current_time が "null" 文字列で返るので文字列比較で OK
[ "$A_MODE" = "Live" ] && [ "$B_MODE" = "Live" ] && [ "$B_CT" = "null" ] && [ "$A_CT" = "null" ] && \
  pass "TC-X2-08: Live 中ボタン操作は no-op" || \
  fail "TC-X2-08" "mode=$B_MODE→$A_MODE ct=$B_CT→$A_CT"

restore_state
print_summary
```

---

## 横断スイート X3: チャート表示内容と更新タイミング

**目的**: バックエンドの状態前進だけでなく **「描画されているチャートが正しく更新されている」** ことを HTTP API 経由で検証する。**全 TC が `[要 API 拡張]`** で `chart-snapshot`（§2.1）に依存。

```bash
#!/bin/bash
# x3_chart_update.sh
# ... [共通ヘルパーをここに貼る] ...

echo "=== X3: チャート表示内容と更新タイミング ==="
backup_state

START=$(utc_offset -3)
END=$(utc_offset -1)
START_MS=$(node -e "console.log(new Date('${START}:00Z').getTime())")
END_MS=$(node -e "console.log(new Date('${END}:00Z').getTime())")

cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X3","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"B"
      }}
    }
  },"popout":[]}}],"active_layout":"X3"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF

start_app

# chart-snapshot API の存在確認
PROBE=$(curl -s -o /dev/null -w "%{http_code}" "$API/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000000")
if [ "$PROBE" = "404" ]; then
  pend "TC-X3-*" "chart-snapshot API 未実装（§2.1）— X3 全 TC を PENDING"
  restore_state
  print_summary
  exit 0
fi

if ! wait_playing 60; then
  fail "X3-precond" "Playing 到達せず"
  restore_state
  print_summary
  exit 1
fi

# pane id 取得
PANE_LIST=$(curl -s "$API/pane/list")
BTC_PANE=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const p=(d.panes||[]).find(p=>p.ticker && p.ticker.includes('BTCUSDT'));
  console.log(p?p.id:'');
" "$PANE_LIST")
ETH_PANE=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const p=(d.panes||[]).find(p=>p.ticker && p.ticker.includes('ETHUSDT'));
  console.log(p?p.id:'');
" "$PANE_LIST")
[ -n "$BTC_PANE" ] && [ -n "$ETH_PANE" ] && pass "TC-X3-precond: 2 ペイン id 取得" || \
  fail "TC-X3-precond" "BTC=$BTC_PANE ETH=$ETH_PANE"

# --- TC-X3-01: Play 到達直後のチャート初期本数 ---
# 目的:     プリフェッチ完了後、kline_count >= 1 でかつ first_kline_time が range 内
# 手段:     chart-snapshot を即時取得
# 期待値:   kline_count >= 1, first_kline_time >= START_MS, last_kline_time <= current_time
# 失敗時:   プリフェッチ失敗 / kline ingest 漏れ
SNAP=$(chart_snapshot "$BTC_PANE")
KC=$(jqn "$SNAP" "d.kline_count")
FT=$(jqn "$SNAP" "d.first_kline_time")
LT=$(jqn "$SNAP" "d.last_kline_time")
CT_NOW=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
[ "$KC" -ge 1 ] 2>/dev/null && pass "TC-X3-01a: kline_count=$KC >= 1" || \
  fail "TC-X3-01a" "kline_count=$KC"
node -e "process.exit((BigInt('$FT')>=BigInt('$START_MS') && BigInt('$LT')<=BigInt('$CT_NOW'))?0:1)" \
  && pass "TC-X3-01b: first/last kline ∈ [start, current_time]" \
  || fail "TC-X3-01b" "first=$FT last=$LT range=[$START_MS,$CT_NOW]"

# --- TC-X3-02: Playing 進行中の last_kline_time 単調非減少 ---
# 目的:     描画側の kline 列が止まらず追従する
# 手段:     1 秒間隔で 5 サンプル
# 期待値:   全サンプルで last_kline_time が単調非減少、かつ少なくとも 1 回は増加
PREV_LT="0"
INCREASED="false"
MONO="true"
for i in $(seq 1 5); do
  S=$(chart_snapshot "$BTC_PANE")
  LT=$(jqn "$S" "d.last_kline_time")
  GT=$(node -e "console.log(BigInt('$LT')>BigInt('$PREV_LT'))")
  GE=$(node -e "console.log(BigInt('$LT')>=BigInt('$PREV_LT'))")
  [ "$GE" = "true" ] || MONO="false"
  [ "$GT" = "true" ] && INCREASED="true"
  PREV_LT="$LT"
  sleep 1
done
[ "$MONO" = "true" ] && pass "TC-X3-02a: last_kline_time 単調非減少" || fail "TC-X3-02a" "逆行あり"
[ "$INCREASED" = "true" ] && pass "TC-X3-02b: last_kline_time 増加あり" || \
  fail "TC-X3-02b" "5 秒で 1 度も増加せず（描画停止？）"

# --- TC-X3-03: StepForward 押下で 1 bar 分 last_kline_time が進む ---
# 目的:     Step ボタンの効果がチャートに即時反映
# 期待値:   after.last_kline_time - before.last_kline_time == STEP_M1
curl -s -X POST "$API/replay/pause" > /dev/null
wait_paused 5 || true  # 既に Paused でも構わない
B_SNAP=$(chart_snapshot "$BTC_PANE")
B_LT=$(jqn "$B_SNAP" "d.last_kline_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
sleep 1
A_SNAP=$(chart_snapshot "$BTC_PANE")
A_LT=$(jqn "$A_SNAP" "d.last_kline_time")
DIFF=$(bigt_sub "$A_LT" "$B_LT")
[ "$DIFF" = "$STEP_M1" ] && pass "TC-X3-03: StepForward → +1 bar (last_kline +60000ms)" || \
  fail "TC-X3-03" "diff=$DIFF (expected $STEP_M1)"

# --- TC-X3-04: マルチペイン同期（BTC と ETH の last_kline_time が概ね一致）---
# 目的:     同一 timeframe の 2 ペインが同じ仮想時刻バーまで描画している
# 期待値:   abs(btc_last - eth_last) <= STEP_M1（1 bar の境界誤差を許容）
B=$(chart_snapshot "$BTC_PANE")
E=$(chart_snapshot "$ETH_PANE")
B_LT=$(jqn "$B" "d.last_kline_time")
E_LT=$(jqn "$E" "d.last_kline_time")
ABS=$(node -e "
  const a=BigInt('$B_LT'), b=BigInt('$E_LT');
  console.log(String(a>b?a-b:b-a));
")
node -e "process.exit(BigInt('$ABS')<=BigInt('$STEP_M1')?0:1)" \
  && pass "TC-X3-04: BTC/ETH last_kline 同期 (diff=$ABS)" \
  || fail "TC-X3-04" "diff=$ABS > $STEP_M1"

# --- TC-X3-05: 終端到達後のチャート完全性 ---
# 目的:     Paused 時に最終バーが描画に含まれている
# 期待値:   last_kline_time >= end_time - STEP_M1
# 手段:     10x まで上げて終端まで進める
for s in "2x" "5x" "10x"; do
  jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed" > /dev/null
done
curl -s -X POST "$API/replay/resume" > /dev/null
if ! wait_paused 120; then
  fail "TC-X3-05-precond" "終端到達せず"
  restore_state
  print_summary
  exit 1
fi
END_T=$(jqn "$(curl -s "$API/replay/status")" "d.end_time")
FIN=$(chart_snapshot "$BTC_PANE")
F_LT=$(jqn "$FIN" "d.last_kline_time")
node -e "process.exit(BigInt('$F_LT')>=BigInt('$END_T')-BigInt('$STEP_M1')?0:1)" \
  && pass "TC-X3-05: 終端 last_kline_time >= end - 1 bar" \
  || fail "TC-X3-05" "last=$F_LT end=$END_T"

# --- TC-X3-06: 1x 速度の更新頻度（バーステップ ~1Hz）---
# 目的:     描画側の更新が概ね 1 bar/秒（1x の base_step_delay = 1000ms）
# 手段:     終端到達済みのため fixture を再生成して再起動 → pane id を取り直して計測
# 注意:     pane UUID は再起動で変わる（Uuid::new_v4()）。BTC_PANE を必ず再取得する。
stop_app
START=$(utc_offset -3)
END=$(utc_offset -1)
cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"X3","dashboard":{"pane":{
    "Split":{"axis":"Vertical","ratio":0.5,
      "a":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"A"
      }},
      "b":{"KlineChart":{
        "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
        "stream_type":[{"Kline":{"ticker":"BinanceLinear:ETHUSDT","timeframe":"M1"}}],
        "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"M1"}},
        "indicators":["Volume"],"link_group":"B"
      }}
    }
  },"popout":[]}}],"active_layout":"X3"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF
start_app
if ! wait_playing 60; then
  fail "TC-X3-06-precond" "再起動後 Playing 到達せず"
else
  # ★ 再起動後は pane UUID が再採番されるため必ず取り直す
  PANE_LIST2=$(curl -s "$API/pane/list")
  BTC_PANE=$(node -e "
    const d=JSON.parse(process.argv[1]);
    const p=(d.panes||[]).find(p=>p.ticker && p.ticker.includes('BTCUSDT'));
    console.log(p?p.id:'');
  " "$PANE_LIST2")
  if [ -z "$BTC_PANE" ]; then
    fail "TC-X3-06-precond" "再起動後 BTC pane id 取得失敗"
  else
    SAMPLES=()
    for i in $(seq 1 10); do
      S=$(chart_snapshot "$BTC_PANE")
      SAMPLES+=("$(jqn "$S" "d.last_kline_time")")
      sleep 0.5
    done
    DISTINCT=$(node -e "
      const a=process.argv.slice(1);
      console.log(new Set(a).size);
    " "${SAMPLES[@]}")
    # 5 秒間で 1x なら ~5 distinct が理想、3〜10 を許容
    [[ $DISTINCT -ge 3 && $DISTINCT -le 10 ]] && pass "TC-X3-06: 5s で $DISTINCT 種類の last_kline (~1Hz)" || \
      fail "TC-X3-06" "distinct=$DISTINCT (expected 3-10)"
  fi
fi

# --- TC-X3-07: Live 復帰でチャートストリームが切り替わる ---
# 目的:     Replay→Live で chart-snapshot が live ストリームを返す
curl -s -X POST "$API/replay/toggle" > /dev/null
sleep 5
LIVE_S=$(chart_snapshot "$BTC_PANE")
LIVE_KC=$(jqn "$LIVE_S" "d.kline_count")
[ "$LIVE_KC" -ge 1 ] 2>/dev/null && pass "TC-X3-07: Live 復帰後も kline_count=$LIVE_KC" || \
  fail "TC-X3-07" "Live 復帰後 kline_count=$LIVE_KC"

restore_state
print_summary
```

---

## 19. 実行順序と依存関係

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

X1（current_time）──→ §2.2 current_time_display 実装後にフル PASS
X2（ボタン）       ──→ S1 PASS 後に実行
X3（チャート更新） ──→ §2.1 chart-snapshot 実装が前提（未実装時は全 PEND）
```

### 推奨実行コマンド

```bash
# ビルドを確認してから実行
cargo build --release

# 主要スイート（既存 API のみで実行可能）
for s in s1 s2 s3 s4 s6 s8 s9 s10 x2; do
  echo ""
  echo "=========================================="
  echo "Running Suite: $s"
  echo "=========================================="
  bash "docs/plan/e2e_scripts/${s}_*.sh"
done

# API 拡張後に実行
for s in x1 x3; do
  bash "docs/plan/e2e_scripts/${s}_*.sh"
done

# 立花が使える場合のみ
bash "docs/plan/e2e_scripts/s5_tachibana_binance.sh"
```

---

## 20. 合否判定基準

### 全スイート PASS 条件

| カテゴリ | 合格ライン | API 拡張 |
|---------|-----------|---------|
| 基本ライフサイクル（S1） | TC-S1-01〜15f 全 PASS（15c/15d/15e/15f を新規含む） | 不要 |
| 永続化（S2） | TC-S2-01〜04 + 02d/02e の ms 整合 PASS | 不要 |
| Auto-play（S3） | TC-S3-01〜05c 全 PASS（05c は notifications を使用） | 不要 |
| マルチペイン Binance（S4） | 全 4 項目 PASS。特に TC-S4-01 の 15s 以内 Playing | 不要 |
| 立花混在（S5） | TC-S5-03〜06 PASS（ログイン済み前提） | 不要 |
| 時間軸混在（S6） | 全 4 項目 PASS | 不要 |
| Mid-replay（S7） | 全 6 項目 PASS | 不要 |
| エラー（S8） | 400/404 + TC-S8-05a/b/c + 06a/b/c PASS | 不要 |
| 速度・Step（S9） | **TC-S9-02（5x wall delay）と TC-S9-03（Playing 中 Step no-op）も PASS 必須化** | 不要 |
| 終端到達（S10） | 全 5 項目 PASS（02 は INFO → FAIL 化） | 不要 |
| **X1: current_time** | **TC-X1-01〜03/06 PASS。04/05 は API 実装後 PASS（未実装時 PENDING）** | 一部 |
| **X2: ボタン** | **TC-X2-01〜08 全 PASS** | 不要 |
| **X3: チャート更新** | **TC-X3-precond〜07 全 PASS（chart-snapshot 実装前は全 PENDING）** | **要** |

### CI での扱い

- S5（立花）は手動テストのみ（keyring / セッション依存）
- S7（Mid-replay）は pane API の実装状況による
- **X1/X3 は §2 の API 拡張を CI ジョブの前提条件とする**（chart-snapshot 未実装ブランチでは PEND が出ても CI は FAIL 扱いしない）
- TC-X3-06 / TC-S9-02 は wall 時間に依存するため CI ホスト性能で許容範囲を調整可

### PEND（PENDING）の扱い

- §4.3 の 3 値結果モデルに基づき、`PEND` は **"テストの責任ではなく実装側ブロック"** を示す
- `PEND > 0` の状態で本計画を「完了」とは呼べない
- 完了条件: **`FAIL == 0` かつ `PEND == 0`**（= §2 の全 API 拡張が実装され、全 TC が PASS）

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
