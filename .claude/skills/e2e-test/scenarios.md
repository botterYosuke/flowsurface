# テストシナリオ集

## カテゴリ 1: リプレイ基本ライフサイクル（20項目、全PASS確認済み）

**Fixture**: 最小構成（Live モード起動）→ 15s 待機後 Toggle + Play

```bash
# Step 1: 起動・復元確認
STATUS=$(curl -s "$API/replay/status")
MODE=$(jqn "$STATUS" "d.mode")         # → "Replay"
PSTATUS=$(jqn "$STATUS" "d.status")    # → "null" (playback なし)
RS=$(jqn "$STATUS" "d.range_start")    # → 保存された日時
RE=$(jqn "$STATUS" "d.range_end")      # → 保存された日時

# Step 2: Play
PLAY_RESULT=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$RS\",\"end\":\"$RE\"}")
PLAY_ST=$(jqn "$PLAY_RESULT" "d.status")  # → "Loading" or "Playing"

# Step 3: Loading → Playing 遷移待ち（最大120秒）
for i in $(seq 1 120); do
  ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$ST" = "Playing" ] && break
  sleep 1
done

# Step 4: 再生中の検証（current_time が前進することを確認）
CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 2
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# CT2 > CT なら時間が進んでいる（BigInt比較推奨）

# Step 5: Pause
curl -s -X POST "$API/replay/pause" > /dev/null
P1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 2
P2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# P1 == P2 なら固定されている

# Step 6: Resume
curl -s -X POST "$API/replay/resume" > /dev/null
sleep 2
R1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# R1 > P2 なら再開している

# Step 7: Speed サイクル
for expected in "2x" "5x" "10x" "1x"; do
  SPEED=$(jqn "$(curl -s -X POST "$API/replay/speed")" "d.speed")
  # SPEED == expected
done

# Step 8: Step forward/backward (pause 中)
curl -s -X POST "$API/replay/pause" > /dev/null
T_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
T_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# T_AFTER > T_BEFORE。Play→Pause後1回目も 60000ms になる（StepClock）

curl -s -X POST "$API/replay/step-backward" > /dev/null
T_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# T_BACK < T_AFTER（前の kline 境界に戻る）

# Step 9: Toggle back to Live
TOGGLE=$(curl -s -X POST "$API/replay/toggle")
# mode == "Live", status == null
```

---

## カテゴリ 2: 永続化テスト（11項目、全PASS確認済み）

```bash
# Test 1: replay 付きテンプレートで起動
# → mode=="Replay", status==null, range_start/range_end 復元

# Test 2: replay フィールドなしで起動（後方互換）
# → mode=="Live", range_start=="", range_end==""

# Test 3: 保存→再起動 往復テスト
# Play 実行後（range_input が設定された状態）に保存すること
# Toggle to Replay → POST /api/app/save → kill → restart → mode still Replay
curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$RS\",\"end\":\"$RE\"}" > /dev/null
# Playing 待ち...
curl -s -X POST "$API/app/save" > /dev/null
stop_app
# 再起動後に mode が保持されることを確認
```

---

## カテゴリ 3: エラーケース

```bash
# 404: 存在しないパス
curl -s -o /dev/null -w "%{http_code}" "$API/nonexistent"
# → 404

# 400: 不正 JSON
curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d 'not json'
# → 400

# 400: 必須フィールド欠損
curl -s -o /dev/null -w "%{http_code}" -X POST "$API/replay/play" \
  -H "Content-Type: application/json" -d '{"start":"2026-04-10 09:00"}'
# → 400

# 404: メソッド不一致 (GET on POST endpoint)
curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:9876/api/replay/toggle"
# → 404
```

---

## カテゴリ 4: マルチペイン・リプレイ回帰テスト（7項目、全PASS確認済み 2026-04-12）

**背景**: kline/trades 分離修正（trades を `Task::sip` でバックグラウンド実行）の回帰テスト。
単一ペイン構成では trades フェッチの遅延が顕在化しないため、マルチペイン + 長時間レンジで検証する。

**Fixture**: `fixtures.md` の「マルチペイン + リプレイ」テンプレート（12時間レンジ推奨）

```bash
# Test A: Playing transition within 15 seconds (regression for kline/trades separation)
PLAY_RESULT=$(curl -s -X POST "$API/replay/play" \
  -H "Content-Type: application/json" \
  -d "{\"start\":\"$RS\",\"end\":\"$RE\"}")
PLAY_ST=$(jqn "$PLAY_RESULT" "d.status")
# Accept Loading or Playing
START_TIME=$(date +%s)
for i in $(seq 1 15); do
  ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$ST" = "Playing" ] && break
  sleep 1
done
ELAPSED=$(($(date +%s) - START_TIME))
# MUST reach Playing within 15s — if not, trades may be blocking kline gate

# Test B: current_time progression
CT1=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 3
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# CT2 > CT1 (use BigInt: node -e "console.log(BigInt('$CT2')>BigInt('$CT1'))")

# Test C: App stability after 10s (trades arriving in background)
sleep 10
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# Still "Playing" — app not crashed by background trades
```

**追加の検証指針**:

| 検証対象 | 方法 | 注意 |
|---------|------|------|
| Loading→Playing 高速遷移 | 15秒タイムアウト付きポーリング | **マルチペイン構成必須**。単一ペインでは trades 影響が小さく検出不能 |
| trades バックグラウンド | Playing 遷移後 10s 経過しても Playing 継続 | 直接 API なし。間接検証 |
| BigInt 比較 | `node -e "console.log(BigInt(a)>BigInt(b))"` | current_time は大きい数値。JS Number で精度不足の場合あり |

---

## カテゴリ 4: Replay fixture 直接起動（auto-play）

**概要**: `saved-state.json` に `replay.mode = "replay"` + 有効な `range_start` / `range_end` を含めて起動すると、
全ペインの streams が Ready になった瞬間に自動で `ReplayMessage::Play` が発火する。

**Fixture**: fixtures.md #2「最小構成 + リプレイ復元テスト」（または #5 マルチペイン + リプレイ）に
過去 24-48h 以内の range を設定して配置。

```bash
# 1. Replay fixture 配置 & 起動
START=$(date -u -d "2 hours ago" +"%Y-%m-%d %H:%M" 2>/dev/null || date -u -v-2H +"%Y-%m-%d %H:%M")
END=$(date -u -d "1 hour ago" +"%Y-%m-%d %H:%M" 2>/dev/null || date -u -v-1H +"%Y-%m-%d %H:%M")

cat > C:/tmp/replay-direct-boot.json <<EOF
{
  "layout_manager": { "layouts": [{ "name": "E2E-DirectBoot", "dashboard": {
    "pane": { "KlineChart": {
      "layout": { "splits": [0.78], "autoscale": "FitToVisible" }, "kind": "Candles",
      "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
      "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
      "indicators": ["Volume"], "link_group": "A"
    } }, "popout": []
  } }], "active_layout": "E2E-DirectBoot" },
  "timezone": "UTC", "trade_fetch_enabled": false, "size_in_quote_ccy": "Base",
  "replay": { "mode": "replay", "range_start": "$START", "range_end": "$END" }
}
EOF

cp C:/tmp/replay-direct-boot.json "$DATA_DIR/saved-state.json"
start_app

# 2. auto-play で Playing になるまでポーリング（最大 30s）— sleep 15 は不要
STATUS=""
for i in $(seq 1 30); do
  STATUS=$(curl -s "$API/replay/status")
  ST=$(jqn "$STATUS" "d.status")
  [ "$ST" = "Playing" ] && break
  sleep 1
done
[ "$(jqn "$STATUS" "d.status")" = "Playing" ] && pass "auto-play: status=Playing" || fail "auto-play" "Expected Playing, got $(jqn "$STATUS" "d.status")"

# 3. current_time が range_start 以上であることを確認
#    （auto-play 後、Playing 検知時点で既に数 tick 進んでいる場合があるため >= で比較）
CT=$(jqn "$STATUS" "d.current_time")
START_MS=$(node -e "const d=new Date('$START:00Z'); console.log(d.getTime())")
END_MS=$(node -e "const d=new Date('$END:00Z'); console.log(d.getTime())")
IN_RANGE=$(node -e "console.log(BigInt('$CT') >= BigInt('$START_MS') && BigInt('$CT') <= BigInt('$END_MS'))")
[ "$IN_RANGE" = "true" ] && pass "auto-play: current_time in range [$CT]" || \
  fail "auto-play: current_time" "Expected [$START_MS,$END_MS], got $CT"

# 4. StepForward で 60000ms 進む
curl -s -X POST "$API/replay/pause" > /dev/null
PRE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
curl -s -X POST "$API/replay/step-forward" > /dev/null
POST=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
DIFF=$(node -e "console.log(BigInt('$POST') - BigInt('$PRE'))")
[ "$DIFF" = "60000" ] && pass "auto-play: StepForward +60000ms" || \
  fail "auto-play: StepForward" "Expected diff=60000, got $DIFF"
```

**検証ポイント**:
- `start_app` 後に `sleep 15` が**不要**（auto-play が streams 解決を自動検知して Play）
- `POST /replay/toggle` も `POST /replay/play` も**不要**
- タイムアウト 30s を超えると toast 通知が出て auto-play は発火しない（symbol ミス等のエラー検知）
