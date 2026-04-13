---
name: e2e
description: "flowsurface 全機能の E2E テストスキル。HTTP API 経由でアプリを操作・検証し、不足 API があれば新規追加する。"
allowed-tools: Read Grep Glob Bash Write Edit
---

# flowsurface E2E テスト

## アーキテクチャ

```
テストスクリプト (curl + node)
    ↓ HTTP
API サーバー (src/replay_api.rs, TCP :9876)
    ↓ mpsc channel
iced アプリ (Message → update() → State 変更)
    ↓ oneshot
API レスポンス (JSON)
```

| レイヤー | ファイル | 役割 |
|---------|---------|------|
| API サーバー | `src/replay_api.rs` | HTTP → ReplayCommand 変換、ルーティング |
| 状態管理 | `src/replay/` | StepClock・EventStore・dispatch_tick |
| アプリ本体 | `src/main.rs` | Message ハンドリング、全機能の update() |
| 永続化 | `data/src/config/state.rs` | State / ReplayConfig の serialize/deserialize |
| レイアウト | `data/src/layout/pane.rs` | ペイン構成・ストリーム設定 |
| 取引所 | `exchange/src/adapter/` | Binance, Bybit, OKX, Hyperliquid, MEXC, 立花 |

## 前提条件

- Windows (bash from Git Bash / MSYS2)
- `curl`, `node` が使用可能
- **`jq` は未インストールの可能性がある → `node -e` で代用する**
- ポート 9876 が空いている（変更: `FLOWSURFACE_API_PORT=9877`）
- `cargo build --release` を E2E 前に実行すること（テストは release バイナリを使用）

## 共通ヘルパー関数

**全 E2E テストスクリプトの先頭にそのまま貼る**:

```bash
#!/bin/bash
set -e

DATA_DIR="$APPDATA/flowsurface"
API="http://127.0.0.1:9876/api"
PASS=0
FAIL=0
EXE="C:/Users/sasai/Documents/flowsurface/target/release/flowsurface.exe"

# jq 代替（node で JSON パース）
# 使い方: jqn "$JSON" "d.mode"  → d は parse 済みオブジェクト
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
```

## テスト実行フロー

**新規シナリオは必ず Replay fixture 直接起動を使う**（Live fixture + `sleep 15` は旧フロー）:

```bash
# 1. バックアップ & Replay fixture 配置
cp "$DATA_DIR/saved-state.json" "$DATA_DIR/saved-state.json.bak"
cp C:/tmp/replay-fixture.json "$DATA_DIR/saved-state.json"

# 2. 起動
start_app

# 3. auto-play で Playing になるまでポーリング（最大 30s）
for i in $(seq 1 30); do
  ST=$(curl -s "$API/replay/status")
  STATUS=$(jqn "$ST" "d.status")
  [ "$STATUS" = "Playing" ] && break
  sleep 1
done

# 4. API 操作 & 検証（scenarios.md のシナリオを参照）

# 5. クリーンアップ
stop_app
cp "$DATA_DIR/saved-state.json.bak" "$DATA_DIR/saved-state.json"
```

---

## 重要な注意点（常に適用）

### Replay fixture の auto-play

`saved-state.json` に `replay.mode = "replay"` かつ `range_start` / `range_end` を含めて起動すると、
`ReplayState::pending_auto_play` フラグにより **全ペインの streams が Ready になった瞬間に自動で Play が発火する**。
`POST /replay/toggle` も `POST /replay/play` も叩く必要がない。

```bash
# Replay fixture を配置して起動するだけで Playing になる（例: 5〜9s 程度）
cp C:/tmp/replay-fixture.json "$DATA_DIR/saved-state.json"
start_app

for i in $(seq 1 30); do
  ST=$(curl -s "$API/replay/status")
  STATUS=$(jqn "$ST" "d.status")
  [ "$STATUS" = "Playing" ] && break
  sleep 1
done
```

**タイムアウトは廃止**（2026-04-13, replay_auto_play_no_timeout）: auto-play は **イベント駆動**に変更された。
streams が解決しない原因に応じて以下のパスをたどる:

| 状況 | 挙動 |
|------|------|
| Binance 等 — metadata がすぐ揃う | streams Ready → 即座に Play 発火（数秒） |
| Tachibana — 有効なセッションあり | session 復元 → master download → `UpdateMetadata` → `refresh_waiting_panes` → Play 発火 |
| Tachibana — セッションなし（未ログイン） | `SessionRestoreResult(None)` → `pending_auto_play = false` + info toast「Replay auto-play was deferred: please log in to resume」→ ログイン画面表示 |

Tachibana セッションなし時はアプリログ（`$DATA_DIR/flowsurface-current.log`）に
`[auto-play] session unavailable — auto-play deferred` が記録される。

**auto-play させたくない場合**: `range_start` / `range_end` を空文字にする。
`pending_auto_play` の起動時ガードが `!range_start.is_empty() && !range_end.is_empty()` なので、
range 未設定なら手動 Play 待ちになる。

### taskkill //f は保存をトリガーしない

永続化テストでは **`POST /api/app/save` で明示的に保存してから** kill する:

```bash
curl -s -X POST "$API/app/save" > /dev/null
stop_app
```

### テスト日時は必ず「過去」にすること

**未来の日時を指定すると Binance API からデータが取得できず EventStore が空になる。**
→ `StepForward` が `next_time = None` を返し無効になる。

常に **現在時刻より過去 24〜48h 以内** の範囲を使う。現在の UTC 時刻確認: `date -u +"%Y-%m-%d %H:%M"`

### StepForward / StepBackward は 60000ms 固定（StepClock）

Play→Pause 後に StepForward すれば 1 回目から 60000ms になる（clock.now_ms はバー境界値のみ保持）。

### current_time は range_start 以上であることを確認する

auto-play 後、Playing 検知時点で既に数 tick 進んでいる場合があるため、
`== range_start` ではなく `>= range_start` で比較する:

```bash
START_MS=$(node -e "const d=new Date('${START}:00Z'); console.log(d.getTime())")
END_MS=$(node -e "const d=new Date('${END}:00Z'); console.log(d.getTime())")
IN_RANGE=$(node -e "console.log(BigInt('$CT') >= BigInt('$START_MS') && BigInt('$CT') <= BigInt('$END_MS'))")
[ "$IN_RANGE" = "true" ] && pass "current_time in range" || fail "current_time" "got $CT"
```

### アプリがログイン画面で停止している場合

API が応答しない・ログが出力されない場合、アプリがログイン画面（立花証券等）で待機中の可能性がある。

```bash
curl -s -X POST "$API/app/screenshot"
# → {"ok":true,"path":"C:/tmp/screenshot.png"} が返ったら Read ツールで画面を確認
```

立花証券など**ログイン画面を要するアダプタのペイン**が fixture に含まれていないか確認すること。

### Toggle→Live 時に range_input がリセットされる

`POST /api/replay/toggle` で Live に切り替えると `range_start` / `range_end` が空文字列にリセットされる。
永続化テストでは **必ず Play 実行後（range_input が設定された状態）に保存**すること。

---

## 検証チートシート

| 検証対象 | 方法 | 注意 |
|---------|------|------|
| モード遷移 | `jqn "$STATUS" "d.mode"` | "Live" or "Replay" |
| current_time 前進 | 2回取得して差分 > 0 | BigInt 比較推奨 |
| current_time 初期値 | `>= range_start` かつ `<= range_end` | auto-play 後は既に数 tick 進んでいる場合あり |
| step-forward | pause 後に step → diff = 60000 | StepClock は常に 60000ms 固定 |
| speed | cycle 後に期待値一致 | "1x","2x","5x","10x" の順 |
| auto-play 完了 (Binance) | ポーリング（最大 30s） | 数秒で Playing になる |
| auto-play 完了 (Tachibana) | ポーリング（最大 120s） | master download 完了まで待機 |
| auto-play 放棄確認 | ログに "auto-play deferred" | Tachibana セッションなし時の期待動作 |
| HTTP ステータス | `-o /dev/null -w "%{http_code}"` | 200/400/404 |
| 永続化復元 | fixture 配置→起動→status 確認 | playback は常に null（clock は保存されない） |
| 永続化保存 | `POST /api/app/save` → kill → JSON 確認 | taskkill だけでは保存されない |
| BigInt 比較 | `node -e "console.log(BigInt('$A')>BigInt('$B'))"` | current_time は大きな数値 |


---

## ログ駆動デバッグ

### アプリログの場所

**アプリの `log::info!` / `log::error!` は stderr ではなく `$DATA_DIR/flowsurface-current.log` に書かれる。**
`"$EXE" 2>C:/tmp/e2e_debug.log` で取れるのは stderr のみ（通常は空）。
ログチェックは必ずファイルから読むこと:

```bash
LOG_FILE="$APPDATA/flowsurface/flowsurface-current.log"

# 起動前にロール（古いログを消す）
> "$LOG_FILE"

# 起動後
tail -20 "$LOG_FILE"
grep "auto-play deferred" "$LOG_FILE"
```

`cat C:/tmp/e2e_debug.log` で条件分岐を書くとログが常に空のため else 分岐に落ち、
テストがすり抜ける。過去に Phase 4 E2E で踏んだ罠。

### eprintln! で stderr を使うケース

API の JSON レスポンスと混ざらない値を確認したい時のみ:

```rust
eprintln!("[E2E DEBUG] field={:?}", value);
```

```bash
"$EXE" 2>C:/tmp/debug.log &
tail -f C:/tmp/debug.log
```

**PR 前に必ず削除**すること: `grep -r "E2E DEBUG" src/`

---

## 支援ファイル

| ファイル | 内容 |
|---------|------|
| [api-reference.md](api-reference.md) | API エンドポイント一覧・レスポンス形式・追加実装パターン |
| [fixtures.md](fixtures.md) | saved-state.json テンプレート集（5種類） |
| [scenarios.md](scenarios.md) | テストシナリオコード（カテゴリ 1〜4） |

## Windows 固有の注意

- `jq` がインストールされていない → `node -e` で JSON パース（上記 `jqn` ヘルパー）
- `/tmp/` パスは使えない → `C:/tmp/` を使用
- exe 起動中は `cargo build` が失敗 → `taskkill //f //im flowsurface.exe` で先に停止
- bash から taskkill はスラッシュを `//f //im` にエスケープ
- `$APPDATA` は `C:\Users\{user}\AppData\Roaming`
