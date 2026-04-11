---
name: e2e
description: "flowsurface 全機能の E2E テストスキル。HTTP API 経由でアプリを操作・検証し、不足 API があれば新規追加する。"
allowed-tools: Read Grep Glob Bash Write Edit
user-invocable: true
---

# flowsurface E2E テスト

## 概要

flowsurface アプリを実際に起動し、HTTP API (`127.0.0.1:9876`) 経由で全機能を操作・検証する E2E テスト手法。

### アーキテクチャ

```
テストスクリプト (curl + node)
    ↓ HTTP
API サーバー (src/replay_api.rs, TCP :9876)
    ↓ mpsc channel
iced アプリ (Message → update() → State 変更)
    ↓ oneshot
API レスポンス (JSON)
```

### 対象コード

| レイヤー | ファイル | 役割 |
|---------|---------|------|
| API サーバー | `src/replay_api.rs` | HTTP → ReplayCommand 変換、ルーティング |
| 状態管理 | `src/replay.rs` | リプレイ状態・PlaybackState |
| アプリ本体 | `src/main.rs` | Message ハンドリング、全機能の update() |
| 永続化 | `data/src/config/state.rs` | State / ReplayConfig の serialize/deserialize |
| レイアウト | `data/src/layout/pane.rs` | ペイン構成・ストリーム設定 |
| 取引所 | `exchange/src/adapter/` | Binance, Bybit, OKX, Hyperliquid, MEXC, 立花 |

## 前提条件

- Windows (bash from Git Bash / MSYS2)
- `curl`, `node` が使用可能
- **`jq` は未インストールの可能性がある → `node -e` で代用する**
- ポート 9876 が空いている（変更: `FLOWSURFACE_API_PORT=9877`）

## 実証済みヘルパー関数

以下のヘルパーは全 E2E テストで共通利用する。

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

### 1. saved-state.json の準備

```bash
# バックアップ
cp "$DATA_DIR/saved-state.json" "$DATA_DIR/saved-state.json.bak"

# テスト用テンプレートを配置（後述から選択）
cp C:/tmp/test-fixture.json "$DATA_DIR/saved-state.json"
```

**注意**: Windows では `/tmp/` は使えない。`C:/tmp/` を使用すること。

### 2. アプリ起動 & 起動待ち

`start_app` ヘルパーを使用（上記参照）。

### 3. API 操作 & 検証（後述のシナリオ集を参照）

### 4. クリーンアップ

```bash
stop_app
cp "$DATA_DIR/saved-state.json.bak" "$DATA_DIR/saved-state.json"
```

---

## 重要な注意点（実証済み）

### taskkill //f は保存をトリガーしない

`taskkill //f` は強制終了のため `save_state_to_disk()` が呼ばれない。
永続化テストでは **`POST /api/app/save` で明示的に保存してから** kill する:

```bash
curl -s -X POST "$API/app/save" > /dev/null
stop_app
```

### Loading → Playing が一瞬で完了する場合がある

`trade_fetch_enabled: false` かつフェッチ対象が少ない場合、Play レスポンス時点で既に Playing になる。
テストでは **Loading と Playing の両方を許容** する:

```bash
PLAY_ST=$(jqn "$PLAY_RESULT" "d.status")
if [ "$PLAY_ST" = "Loading" ] || [ "$PLAY_ST" = "Playing" ]; then
  pass "Play accepted"
fi
```

### リプレイ範囲に上限はない

6時間制限は撤廃済み。`fetch_klines` は自動ページングで任意の範囲を取得する。
ただしテストでは短い範囲（1-12h）が高速で推奨。

---

## API エンドポイント一覧

### リプレイ API

| メソッド | パス | 用途 |
|---------|------|------|
| `GET` | `/api/replay/status` | 現在状態の JSON 取得 |
| `POST` | `/api/replay/toggle` | Live↔Replay 切替 |
| `POST` | `/api/replay/play` | 再生開始（body: `{"start":"...","end":"..."}` 必須） |
| `POST` | `/api/replay/pause` | 一時停止 |
| `POST` | `/api/replay/resume` | 再開 |
| `POST` | `/api/replay/step-forward` | +60s ジャンプ |
| `POST` | `/api/replay/step-backward` | -60s ジャンプ |
| `POST` | `/api/replay/speed` | 速度サイクル（1x→2x→5x→10x→1x） |

### アプリ API

| メソッド | パス | 用途 |
|---------|------|------|
| `POST` | `/api/app/save` | 状態をディスクに保存（saved-state.json） |

### ReplayStatus レスポンス形式

```json
// Live モード（playback なし）
{"mode":"Live","range_start":"","range_end":""}

// Replay モード（playback なし、復元直後）
{"mode":"Replay","range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}

// Replay モード（再生中）
{
  "mode":"Replay",
  "status":"Playing",
  "current_time":1775869740288,
  "speed":"1x",
  "start_time":1775869740000,
  "end_time":1775912940000,
  "range_start":"2026-04-11 01:09",
  "range_end":"2026-04-11 13:09"
}
```

**フィールド説明**:
- `mode`: `"Live"` or `"Replay"`
- `status`: `"Loading"` / `"Playing"` / `"Paused"` / null（playback なし時は省略）
- `current_time`: 現在の仮想時刻 (Unix ms)。playback なし時は省略
- `speed`: `"1x"` / `"2x"` / `"5x"` / `"10x"`。playback なし時は省略
- `start_time` / `end_time`: パース済み範囲 (Unix ms)。playback なし時は省略
- `range_start` / `range_end`: UI の範囲入力テキスト（常に存在）

### 追加が必要な API（機能別）

E2E テストで全機能をカバーするために追加するエンドポイント:

#### アプリ状態 `/api/app/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/app/status` | — | アプリ全体の状態（画面、テーマ、timezone 等） |
| `POST` | `/api/app/theme` | `{"theme":"..."}` | テーマ切替 |
| `POST` | `/api/app/timezone` | `{"timezone":"UTC"}` | タイムゾーン変更 |
| `POST` | `/api/app/scale` | `{"factor":1.0}` | UI スケール変更 |
| `POST` | `/api/app/trade-fetch` | `{"enabled":true}` | Trades 自動フェッチの ON/OFF |
| `POST` | `/api/app/volume-unit` | `{"unit":"Base"}` | 出来高の表示単位（Base/Quote） |

#### レイアウト `/api/layout/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/layout/list` | — | レイアウト一覧 |
| `GET` | `/api/layout/active` | — | アクティブレイアウトの詳細（ペイン構成含む） |
| `POST` | `/api/layout/select` | `{"name":"..."}` | アクティブレイアウト切替 |
| `POST` | `/api/layout/create` | `{"name":"..."}` | 新規レイアウト作成 |
| `POST` | `/api/layout/delete` | `{"name":"..."}` | レイアウト削除 |
| `POST` | `/api/layout/rename` | `{"from":"...","to":"..."}` | リネーム |

#### ペイン操作 `/api/pane/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/pane/list` | — | 現在のペイン一覧（種類・ティッカー・設定） |
| `POST` | `/api/pane/add` | `{"type":"KlineChart","ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}` | ペイン追加 |
| `POST` | `/api/pane/remove` | `{"id":"..."}` | ペイン削除 |
| `POST` | `/api/pane/replace` | `{"id":"...","type":"...","ticker":"..."}` | ペイン差替え |
| `POST` | `/api/pane/link-group` | `{"id":"...","group":"A"}` | リンクグループ変更 |

#### 接続・データ `/api/connection/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/connection/status` | — | WebSocket 接続状態（exchange ごと） |
| `GET` | `/api/connection/streams` | — | アクティブなストリーム一覧 |

#### 通知 `/api/notification/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/notification/list` | — | 現在の通知一覧 |

---

## API 追加の実装パターン

現在の API は `src/replay_api.rs` でリプレイ + app/save。全機能対応のためにルーティングを拡張する。

### Step 1: コマンド型を汎用化

```rust
// src/replay.rs の ReplayCommand を拡張するか、新しい ApiCommand enum を作成
pub enum ReplayCommand {
    // 既存リプレイ
    GetStatus,
    Toggle,
    Play { start: String, end: String },
    Pause,
    Resume,
    StepForward,
    StepBackward,
    CycleSpeed,
    // アプリ
    SaveState,
    // ... 新規コマンドを追加
}
```

### Step 2: route() を拡張

```rust
fn route(method: &str, path: &str, body: &str) -> Result<ReplayCommand, RouteError> {
    match (method, path) {
        // 既存
        ("GET",  "/api/replay/status")  => Ok(ReplayCommand::GetStatus),
        ("POST", "/api/replay/toggle")  => Ok(ReplayCommand::Toggle),
        // ...
        ("POST", "/api/app/save")       => Ok(ReplayCommand::SaveState),
        // 新規を追加
        _ => Err(RouteError::NotFound),
    }
}
```

### Step 3: main.rs でハンドリング

`Message::ReplayApi` のマッチアームに新規コマンドのケースを追加。

### Step 4: ユニットテスト

`route()` のテストは `#[cfg(test)]` 内に追加（既存パターンを踏襲）。

---

## 実証済みテストシナリオ集

### カテゴリ 1: リプレイ基本ライフサイクル（20項目、全PASS確認済み）

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

# Step 3: Loading → Playing 遷移待ち
for i in $(seq 1 120); do
  ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  [ "$ST" = "Playing" ] && break
  sleep 1
done

# Step 4: 再生中の検証
CT=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
sleep 2
CT2=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# CT2 > CT なら時間が進んでいる

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
# T_AFTER - T_BEFORE == 60000

curl -s -X POST "$API/replay/step-backward" > /dev/null
T_BACK=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# T_BACK == T_BEFORE

# Step 9: Toggle back to Live
TOGGLE=$(curl -s -X POST "$API/replay/toggle")
# mode == "Live", status == null
```

### カテゴリ 2: 永続化テスト（11項目、全PASS確認済み）

```bash
# Test 1: replay 付きテンプレートで起動
# → mode=="Replay", status==null, range_start/range_end 復元

# Test 2: replay フィールドなしで起動（後方互換）
# → mode=="Live", range_start=="", range_end==""

# Test 3: 保存→再起動 往復テスト
# → toggle to Replay → POST /api/app/save → kill → restart → mode still Replay
# → toggle to Live → POST /api/app/save → kill → check saved-state.json replay.mode=="live"
```

### カテゴリ 3: エラーケース

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

## テスト用テンプレート (saved-state.json)

### 最小構成（KlineChart 1枚）

フェッチが速く、基本動作テストに最適:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-Test",
      "dashboard": {
        "pane": {
          "KlineChart": {
            "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
            "kind": "Candles",
            "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
            "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
            "indicators": ["Volume"],
            "link_group": "A"
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

### 最小構成 + リプレイ復元テスト

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-Test",
      "dashboard": {
        "pane": {
          "KlineChart": {
            "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
            "kind": "Candles",
            "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
            "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
            "indicators": ["Volume"],
            "link_group": "A"
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "2026-04-10 09:00",
    "range_end": "2026-04-10 15:00"
  }
}
```

### マルチペイン構成（KlineChart + TimeAndSales + Ladder）

複数ペインの連動・ストリーム接続テスト用:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-MultiPane",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.5,
            "a": {
              "KlineChart": {
                "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                "kind": "Candles",
                "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                "indicators": ["Volume"],
                "link_group": "A"
              }
            },
            "b": {
              "Split": {
                "axis": "Horizontal", "ratio": 0.5,
                "a": {
                  "TimeAndSales": {
                    "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                },
                "b": {
                  "Ladder": {
                    "stream_type": [
                      { "Depth": { "ticker": "BinanceLinear:BTCUSDT", "depth_aggr": "Client", "push_freq": "ServerDefault" } },
                      { "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }
                    ],
                    "settings": { "tick_multiply": 5, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                }
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-MultiPane"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

### マルチレイアウト構成（レイアウト管理テスト用）

```json
{
  "layout_manager": {
    "layouts": [
      {
        "name": "Layout-A",
        "dashboard": {
          "pane": {
            "KlineChart": {
              "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
              "kind": "Candles",
              "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
              "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
              "indicators": ["Volume"],
              "link_group": "A"
            }
          },
          "popout": []
        }
      },
      {
        "name": "Layout-B",
        "dashboard": {
          "pane": {
            "KlineChart": {
              "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
              "kind": "Candles",
              "stream_type": [{ "Kline": { "ticker": "BinanceLinear:ETHUSDT", "timeframe": "M5" } }],
              "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M5" } },
              "indicators": ["Volume"],
              "link_group": "A"
            }
          },
          "popout": []
        }
      }
    ],
    "active_layout": "Layout-A"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

---

## テストデータの選び方

| 項目 | 推奨値 | 理由 |
|------|--------|------|
| ティッカー | `BinanceLinear:BTCUSDT` | データ豊富。`fetch_trades_batched()` は Binance のみ対応 |
| タイムフレーム | `M1` | 本数が多いが fetch_klines は自動ページングで取得 |
| `timezone` | `"UTC"` | API の unix ms との照合が容易 |
| `trade_fetch_enabled` | `false` | ライブ trades フェッチを止めてノイズ削減 |
| ペイン数 | 最小限 | フェッチ対象減でテスト高速化 |
| リプレイ日時 | 過去 24-48h 以内 | Binance API からデータ取得可能な範囲 |
| リプレイ範囲 | テスト目的による | 短い(1-3h)ほど高速。制限なし |

---

## API 追加の判断基準

### 追加する場合

テストで以下を検証したいが API がないとき:

| カテゴリ | 検証したいこと | 追加する API |
|---------|--------------|-------------|
| リプレイ | 指定時刻へのジャンプ | `POST /api/replay/seek {"time": <unix_ms>}` |
| リプレイ | 再生の完全停止 | `POST /api/replay/stop` |
| アプリ設定 | テーマ/TZ/スケール変更 | `/api/app/*` 系 |
| レイアウト | CRUD・切替 | `/api/layout/*` 系 |
| ペイン | 一覧・追加・削除・差替え | `/api/pane/*` 系 |
| 接続 | WebSocket 状態確認 | `/api/connection/*` 系 |

### 追加しない場合

- GUI 描画の検証（スクリーンショット回帰テストの領域）
- WebSocket ストリームの直接操作（内部実装の詳細）
- Exchange アダプターの直接テスト（統合テストで別途実施）
- ログイン/認証（立花証券のセッション管理は手動テスト or モック）

### 実装の参照パターン

既存の `src/replay_api.rs` を拡張する形で追加:

1. コマンド enum にバリアント追加
2. `route()` にパスマッチ追加
3. `main.rs` の `Message::ReplayApi` ハンドラにケース追加
4. `route()` のユニットテストを `#[cfg(test)]` に追加

---

## 検証の指針

| 検証対象 | 方法 | 注意 |
|---------|------|------|
| モード遷移 | `jqn "$STATUS" "d.mode"` で厳密一致 | "Live" or "Replay" |
| current_time 前進 | 2回取得して差分 > 0 | 再生中は厳密一致NG |
| step-forward | pause 後に step → 差分 == 60000 | pause 中なら厳密一致OK |
| speed | cycle 後に期待値一致 | "1x","2x","5x","10x" の順 |
| Loading→Playing | ポーリング（最大120秒） | 即 Playing になる場合あり |
| HTTP ステータス | `-o /dev/null -w "%{http_code}"` | 200/400/404 |
| 永続化復元 | テンプレート配置→起動→status 確認 | playback は常に null |
| 永続化保存 | `POST /api/app/save` → kill → JSON 確認 | taskkill だけでは保存されない |
| range_start/end | `jqn "$STATUS" "d.range_start"` | 常に存在するフィールド |
| レイアウト CRUD | 作成→一覧確認→削除→一覧確認 | 名前の一意性 |
| 設定変更 | POST→GET で反映確認 | 再起動後の永続化も確認 |

### カテゴリ 4: マルチペイン・リプレイ回帰テスト（7項目、全PASS確認済み 2026-04-12）

**背景**: kline/trades 分離修正（trades を `Task::sip` でバックグラウンド実行）の回帰テスト。
単一ペイン構成では trades フェッチの遅延が顕在化しないため、マルチペイン + 長時間レンジで検証する。

**テンプレート**: マルチペイン構成（BTCUSDT KlineChart + ETHUSDT KlineChart + BTCUSDT TimeAndSales）

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-MultiPane-Replay",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.33,
            "a": {
              "KlineChart": {
                "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                "kind": "Candles",
                "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                "indicators": ["Volume"],
                "link_group": "A"
              }
            },
            "b": {
              "Split": {
                "axis": "Vertical", "ratio": 0.5,
                "a": {
                  "KlineChart": {
                    "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                    "kind": "Candles",
                    "stream_type": [{ "Kline": { "ticker": "BinanceLinear:ETHUSDT", "timeframe": "M1" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                    "indicators": ["Volume"],
                    "link_group": "B"
                  }
                },
                "b": {
                  "TimeAndSales": {
                    "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                }
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-MultiPane-Replay"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "YYYY-MM-DD HH:MM",
    "range_end": "YYYY-MM-DD HH:MM"
  }
}
```

**注意**: `range_start` / `range_end` は過去 24-48h 以内、12 時間レンジを推奨。

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
# CT2 > CT1 (use BigInt for comparison)

# Test C: App stability after 10s (trades arriving in background)
sleep 10
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# Still "Playing" — app not crashed by background trades
```

### 検証の指針（追加）

| 検証対象 | 方法 | 注意 |
|---------|------|------|
| Loading→Playing 高速遷移 | 15秒タイムアウト付きポーリング | **マルチペイン構成必須**。単一ペインでは trades 影響が小さく検出不能 |
| trades バックグラウンド | Playing 遷移後 10s 経過しても Playing 継続 | 直接 API なし。間接検証 |
| BigInt 比較 | `node -e "console.log(BigInt(a)>BigInt(b))"` | current_time は大きい数値。JS Number で精度不足の場合あり |

---

## Windows 固有の注意

- **`jq` がインストールされていない** → `node -e` でJSON パースする（上記 `jqn` ヘルパー）
- **`/tmp/` パスは使えない** → `C:/tmp/` を使用
- exe 起動中は `cargo build` が失敗 → `taskkill //f //im flowsurface.exe` で先に停止
- bash から taskkill はスラッシュを `//f //im` にエスケープ
- `$APPDATA` は `C:\Users\{user}\AppData\Roaming`
- **`taskkill //f` は `save_state_to_disk()` を呼ばない** → 永続化テストでは `POST /api/app/save` を先に呼ぶ
