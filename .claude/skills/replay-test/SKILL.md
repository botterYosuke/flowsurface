---
name: replay-test
description: "flowsurface リプレイ機能のテスト手法。saved-state カスタマイズ、E2E API テスト、ユニットテストのテクニック集。"
allowed-tools: Read Grep Glob Bash Write Edit
user-invocable: true
---

# リプレイ機能テスト手法

仕様: `docs/plan/replay_header.md`

## テストデータの制御: saved-state.json カスタマイズ

アプリの起動状態（ペイン構成・ティッカー等）は `saved-state.json` で決まる。テストに都合の良い構成を注入することで再現性を確保する。

### ファイルパスと環境分離

```
本番: C:\Users\{user}\AppData\Roaming\flowsurface\saved-state.json
```

本番データを汚さず、テスト専用ディレクトリを使う:

```bash
export FLOWSURFACE_DATA_PATH=/tmp/flowsurface-test
mkdir -p $FLOWSURFACE_DATA_PATH
# テスト用 JSON を配置してから起動
cp test-fixture.json $FLOWSURFACE_DATA_PATH/saved-state.json
cargo run --release
```

### 全フィールドに `#[serde(default)]` — 空 JSON `{}` でも起動する

破損時は自動で `saved-state_old.json` にバックアップされる。テストで壊しても安全。

### テスト目的別テンプレート

**最小構成（KlineChart 1 枚）** — フェッチが速く、リプレイ再生の基本動作テストに最適:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "Test",
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
    "active_layout": "Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

**KlineChart + TimeAndSales** — Trades 再生の目視確認用:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "Test",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.7,
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
              "TimeAndSales": {
                "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                "link_group": "A"
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

**KlineChart + Ladder** — 「Replay: Depth unavailable」表示テスト用:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "Test",
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
        },
        "popout": []
      }
    }],
    "active_layout": "Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

### カスタマイズのポイント

| 項目 | 推奨値 | 理由 |
|------|--------|------|
| ティッカー | `BinanceLinear:BTCUSDT` | データ豊富。`fetch_trades_batched()` は Binance のみ対応 |
| タイムフレーム | `M1` | 6h = 360 本で `fetch_klines` の 1000 本制限に余裕 |
| `timezone` | `"UTC"` | API の `current_time`（Unix ms）との照合が容易 |
| `trade_fetch_enabled` | `false` | ライブ時の自動 trades フェッチを止めてノイズ削減 |
| ペイン数 | 最小限 | フェッチ対象が減りテスト高速化 |
| リプレイ日時 | 過去 24-48h 以内 | Binance API からデータを確実に取得できる範囲 |

## E2E API テスト手法

### 起動待ちのパターン

`sleep` 秒数はマシン依存。ポーリングで起動完了を検知する:

```bash
cargo run --release &
for i in $(seq 1 30); do
  curl -s http://127.0.0.1:9876/api/replay/status && break
  sleep 1
done
```

### API エンドポイント一覧

| メソッド | パス | 用途 |
|---------|------|------|
| `GET` | `/api/replay/status` | 現在状態の JSON 取得（常に応答。ログイン画面中も可） |
| `POST` | `/api/replay/toggle` | Live↔Replay 切替 |
| `POST` | `/api/replay/play` | 再生開始（JSON body: `{"start":"...","end":"..."}` 必須） |
| `POST` | `/api/replay/pause` | 一時停止 |
| `POST` | `/api/replay/resume` | 再開 |
| `POST` | `/api/replay/step-forward` | +60s ジャンプ |
| `POST` | `/api/replay/step-backward` | -60s ジャンプ（start_time クランプ） |
| `POST` | `/api/replay/speed` | 速度サイクル（1x→2x→5x→10x→1x） |

### テスト時の注意点

- **操作順序の制約**: `play` の前に `toggle` で Replay モードにする必要がある。Live モードで `play` すると日時パースエラーになる
- **current_time は厳密一致で検証しない**: 再生中は毎フレーム進むため、増減の方向で判定する
- **Loading → Playing 遷移**: データフェッチ完了まで Loading のまま。未来の日時だとデータがなくても Loading で止まる（API 操作自体は正常動作）
- **エラーレスポンス**: 不正パス → 404、不正 JSON body → 400 `{"error":"Bad Request: invalid JSON body"}`
- **ポート変更**: `FLOWSURFACE_API_PORT=9877 cargo run --release` で別ポート起動可能。複数インスタンスの競合回避に使う

### Windows 固有の注意

- exe 起動中は `cargo build` が「アクセスが拒否されました」で失敗する → `taskkill //f //im flowsurface.exe` で先に停止
- bash から taskkill を呼ぶ場合はスラッシュを `//f //im` にエスケープ

## ユニットテスト手法

```bash
cargo test --bin flowsurface replay   # replay モジュールのみ
cargo test --bin flowsurface          # 全テスト
```

### テスト設計のパターン

| パターン | 例 | 手法 |
|---------|---|------|
| 時刻依存テスト | `format_current_time` (Live) | 値ではなくフォーマット（文字列長 19 = "YYYY-MM-DD HH:MM:SS"）で検証 |
| 境界値テスト | `parse_replay_range` | ちょうど 6h (OK) vs 6h1m (NG) の両方をテスト |
| カーソル前進テスト | `TradeBuffer::drain_until` | 連続呼び出しで cursor が正しく前進することを検証 |
| クランプテスト | `advance_time` | `end_time` を超えないことを検証 |
| サイクルテスト | `cycle_speed` | 4 回呼んで元に戻ることを検証 |
| ヘルパー関数 | `test_trade(time: u64)` | price/qty は固定値でテスト対象外のフィールドを簡略化 |
