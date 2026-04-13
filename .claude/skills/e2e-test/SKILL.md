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
| 状態管理 | `src/replay/` | VirtualClock・EventStore・dispatch_tick |
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
  "$EXE" &
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

```bash
# 1. バックアップ & Fixture 配置
cp "$DATA_DIR/saved-state.json" "$DATA_DIR/saved-state.json.bak"
cp C:/tmp/test-fixture.json "$DATA_DIR/saved-state.json"

# 2. 起動
start_app

# 3. Live モード起動時はティッカー解決を待つ
sleep 15  # ticker metadata fetch を待つ（Replay モード fixture を使う場合は不要）

# 4. API 操作 & 検証（scenarios.md のシナリオを参照）

# 5. クリーンアップ
stop_app
cp "$DATA_DIR/saved-state.json.bak" "$DATA_DIR/saved-state.json"
```

---

## 重要な注意点（常に適用）

### Live モード fixture を使うこと（リプレイ E2E）

**Replay モードで saved-state.json を起動すると `ResolvedStream::Waiting` のため
`prepare_replay()` が kline_targets を収集できず EventStore が空になる。**
→ StepForward/StepBackward が機能しない。

**正しいフロー**: fixture に `replay` フィールドを含めない（Live モード起動）→ 15s 待機 → Toggle + Play

### taskkill //f は保存をトリガーしない

永続化テストでは **`POST /api/app/save` で明示的に保存してから** kill する:

```bash
curl -s -X POST "$API/app/save" > /dev/null
stop_app
```

### Loading → Playing が一瞬で完了する場合がある

`trade_fetch_enabled: false` かつフェッチ対象が少ない場合、Play レスポンス時点で既に Playing になる。
**両方を許容**すること:

```bash
if [ "$PLAY_ST" = "Loading" ] || [ "$PLAY_ST" = "Playing" ]; then pass "Play accepted"; fi
```

### テスト日時は必ず「過去」にすること

**未来の日時を指定すると Binance API からデータが取得できず EventStore が空になる。**
→ `StepForward` が `next_time = None` を返し無効になる。

常に **現在時刻より過去 24〜48h 以内** の範囲を使う。現在の UTC 時刻確認: `date -u +"%Y-%m-%d %H:%M"`

### StepForward / StepBackward は 60000ms 固定（StepClock）

Play→Pause 後に StepForward すれば 1回目から 60000ms になる（clock.now_ms はバー境界値のみ保持）。
旧注記「初回は端数分で不定」は VirtualClock 連続モデル時代のもの。**現在は不適用。**

### アプリがログイン画面で停止している場合

API が応答しない・ログが出力されない場合、アプリがログイン画面（立花証券等）で待機中の可能性がある。

```bash
# スクリーンショットで確認
curl -s -X POST "$API/app/screenshot"
# → {"ok":true,"path":"C:/tmp/screenshot.png"} が返ったら Read ツールで画面を確認
```

立花証券など**ログイン画面を要するアダプタのペイン**が fixture に含まれていないか確認すること。

### Toggle→Live 時に range_input がリセットされる

`POST /api/replay/toggle` で Live に切り替えると `range_start` / `range_end` が空文字列にリセットされる。
永続化テストでは **必ず Play 実行後（range_input が設定された状態）に保存**すること。

### replay_buffer_* フィールドは削除済み（R3）

`/api/pane/list` のレスポンスに以下フィールドは**存在しない**:
`replay_buffer_ready`, `replay_buffer_cursor`, `replay_buffer_len`

---

## 検証チートシート

| 検証対象 | 方法 | 注意 |
|---------|------|------|
| モード遷移 | `jqn "$STATUS" "d.mode"` | "Live" or "Replay" |
| current_time 前進 | 2回取得して差分 > 0 | BigInt 比較推奨 |
| step-forward | pause 後に step → `T_AFTER > T_BEFORE` | Play→Pause後1回目から 60000ms |
| speed | cycle 後に期待値一致 | "1x","2x","5x","10x" の順 |
| Loading→Playing | ポーリング（最大120秒） | 即 Playing になる場合あり |
| HTTP ステータス | `-o /dev/null -w "%{http_code}"` | 200/400/404 |
| 永続化復元 | テンプレート配置→起動→status 確認 | playback は常に null |
| 永続化保存 | `POST /api/app/save` → kill → JSON 確認 | taskkill だけでは保存されない |
| BigInt 比較 | `node -e "console.log(BigInt('$A')>BigInt('$B'))"` | current_time は大きな数値 |

---

## 計画書運用（複数セッション作業）

`docs/plan/<feature>.md` を作成し、セッション間のコンテキストを保持する:

- **目的**: 何を達成するか（1〜3行）
- **実装ステップ (Phase 分割)**: 完了したものに `✅` をつける
- **Tips**: Phase 中に発見したエッジケース・落とし穴をその場で追記
- **撤回した旧仕様**: 前のセッションで変更した内容を明示的に記録

参考実例: [docs/plan/replay_bar_step_loop.md](../../docs/plan/replay_bar_step_loop.md)

---

## ログ駆動デバッグ

確認したい箇所に `eprintln!("[E2E DEBUG] field={:?}", value);` を追加して検証する。
stderr に出るため API の JSON レスポンスと混ざらない。

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
