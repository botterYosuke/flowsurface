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
