# Phase 3: Headless モード + Python SDK 実装計画

## 目標

`--headless` フラグ付きで flowsurface を起動すると、`iced` GUI を起動せずに tokio ランタイム + HTTP API サーバー（ポート 9876）だけが動作する状態にする。  
Python 側では Gymnasium 互換の `FlowsurfaceEnv` クラスが強化学習ループを HTTP API 経由で実行できるようにする。

---

## Rust 側の設計

### --headless フラグの起動フロー

```
main() → args に "--headless" があれば headless::run() を呼び tokio ランタイムで実行
        → なければ従来の iced::daemon() 起動
```

### HeadlessEngine

`src/headless.rs` に以下を実装する。

```
HeadlessEngine
├── replay_state: ReplayState           # リプレイ制御
├── virtual_engine: VirtualExchangeEngine # 仮想約定エンジン
├── ticker: exchange::Ticker            # CLI --ticker から構築
├── timeframe: exchange::Timeframe      # CLI --timeframe から構築（デフォルト M1）
└── load_result_tx/rx: tokio::sync::mpsc  # kline ロード完了通知
```

### イベントループ（tokio::select!）

```
loop {
    select! {
        API コマンドを受信     → handle_command()
        kline ロード完了       → handle_load_result()
        タイマー（100ms tick） → tick()  // Playing 時のみ有効
    }
}
```

### 対応 API コマンド（headless モード）

| コマンド | 処理 |
|---|---|
| `GET /api/replay/status` | replay_state.to_status() |
| `POST /api/replay/play {"start":"...","end":"..."}` | kline ロード開始 → Loading → Active |
| `POST /api/replay/pause` | clock.pause() |
| `POST /api/replay/resume` | clock.resume() |
| `POST /api/replay/step-forward` | dispatch_tick() を 1 回実行 |
| `GET /api/replay/state` | controller.get_api_state() 相当 |
| `POST /api/replay/order` | virtual_engine.place_order() |
| `GET /api/replay/portfolio` | virtual_engine.portfolio_snapshot() |
| `GET /api/replay/orders` | virtual_engine.pending_orders() |

その他のコマンド（Pane / Auth / Tachibana 系）は `501 Not Implemented` を返す。

### CLI 引数

```
flowsurface --headless --ticker HyperliquidLinear:BTC --timeframe M1
```

| 引数 | 説明 | デフォルト |
|---|---|---|
| `--headless` | GUI なし起動 | なし（必須フラグ） |
| `--ticker` | 銘柄（ExchangeName:Symbol 形式） | なし（必須） |
| `--timeframe` | タイムフレーム（M1〜D1 等） | `M1` |

---

## Python SDK の設計

### パッケージ構成

```
python/
├── pyproject.toml
├── flowsurface_sdk/
│   ├── __init__.py
│   └── env.py          # FlowsurfaceEnv
└── tests/
    └── test_env.py
```

### FlowsurfaceEnv インターフェース

```python
env = FlowsurfaceEnv(
    headless=True,
    ticker="HyperliquidLinear:BTC",
    timeframe="M1",
    binary_path=None,   # None → PATH から flowsurface を検索
    api_port=9876,
)

# reset: Play を発行してロード完了を待つ
obs, info = env.reset(start="2026-01-01 00:00", end="2026-03-31 23:59")

while not done:
    action = agent.predict(obs)  # {"side": "buy"/"sell"/"hold", "qty": float}
    obs, reward, done, truncated, info = env.step(action)

env.close()  # プロセスを終了
```

### obs の構造（GetState API のレスポンス）

```python
{
    "current_time": 1735689600000,
    "klines": [{"stream": "...", "time": ..., "open": ..., "high": ..., "low": ..., "close": ..., "volume": ...}],
    "trades": [...],
}
```

### reward の定義

```python
reward = portfolio["unrealized_pnl"] + portfolio["realized_pnl"]  # 簡易実装
```

---

## 実装ステータス

### Rust / Python 実装
- ✅ `src/headless.rs` — HeadlessArgs + HeadlessEngine + run()（22 ユニットテスト）
- ✅ `src/replay_api.rs` — `pub async fn start_server()` を追加
- ✅ `src/main.rs` — `--headless` フラグによる分岐
- ✅ `python/flowsurface_sdk/env.py` — FlowsurfaceEnv
- ✅ `python/tests/test_env.py` — 単体テスト

### E2E テスト（IS_HEADLESS パターン）
独立スクリプト（s50）を新設するのではなく、既存テストを `IS_HEADLESS=true/false` で両対応化した。

- ✅ `tests/common_helpers.sh` — `headless_play()` / `ensure_replay_mode()` / `pend_if_headless()` / `order_symbol()` / `setup_single_pane()` headless 対応
- ✅ 18 本のテストスクリプトを headless/GUI 両対応に改修（詳細は `docs/plan/phase3_headless_e2e.md`）
- ✅ `.github/workflows/e2e.yml` — S1 / S3 / S27 headless CI ステップ追加

---

## 依存関係

Phase 3 は Phase 2（仮想約定エンジン）が完了済みのため着手可能。  
Phase 4a（ナラティブ基盤）は Phase 3 完了後に独立して着手可能。
