# Python E2E テスト & SDK 仕様書

> **関連ドキュメント**:
> | 知りたいこと | 参照先 |
> |---|---|
> | HTTP API エンドポイント全覧 | [replay.md §11](replay.md#11-http-制御-api) |
> | 仮想注文 API・ポートフォリオ API | [order.md §7.6](order.md#76-http-api) |
> | リプレイ状態モデル・StepClock | [replay.md](replay.md) |

**最終更新**: 2026-04-19
**対象ブランチ**: `sasa/develop`

---

## 目次

1. [概要](#1-概要)
2. [ディレクトリ構成](#2-ディレクトリ構成)
3. [環境変数](#3-環境変数)
4. [helpers.py — 共通ヘルパーライブラリ](#4-helperspy--共通ヘルパーライブラリ)
   - [4.1 定数](#41-定数)
   - [4.2 レポートヘルパー](#42-レポートヘルパー)
   - [4.3 API ラッパー](#43-api-ラッパー)
   - [4.4 ポーリングヘルパー](#44-ポーリングヘルパー)
   - [4.5 フィクスチャ](#45-フィクスチャ)
   - [4.6 検証ヘルパー](#46-検証ヘルパー)
   - [4.7 ティッカーヘルパー](#47-ティッカーヘルパー)
   - [4.8 モードヘルパー](#48-モードヘルパー)
   - [4.9 通知ヘルパー](#49-通知ヘルパー)
5. [FlowsurfaceEnv — Gymnasium 互換 RL 環境](#5-flowsurfaceenv--gymnasium-互換-rl-環境)
   - [5.1 概要](#51-概要)
   - [5.2 コンストラクタ引数](#52-コンストラクタ引数)
   - [5.3 Gymnasium API](#53-gymnasium-api)
   - [5.4 プロセス管理](#54-プロセス管理)
6. [テストスクリプト パターン](#6-テストスクリプト-パターン)
   - [6.1 スクリプト構造](#61-スクリプト構造)
   - [6.2 GUI モード vs headless モード](#62-gui-モード-vs-headless-モード)
   - [6.3 テストケース命名規則](#63-テストケース命名規則)
   - [6.4 終了コード](#64-終了コード)
7. [テストスイート一覧](#7-テストスイート一覧)
8. [実行方法](#8-実行方法)
9. [フィクスチャ設計](#9-フィクスチャ設計)
   - [9.1 saved-state.json スキーマ](#91-saved-statejson-スキーマ)
   - [9.2 リプレイ付きフィクスチャ](#92-リプレイ付きフィクスチャ)
10. [既知の制限・注意事項](#10-既知の制限注意事項)

---

## 1. 概要

flowsurface の E2E テストは **Python スクリプト群** として `tests/` に配置されている。
すべてのテストは HTTP API（ポート 9876）経由でアプリを制御し、Playwright などのブラウザ自動化は一切使用しない。

| レイヤー | ファイル | 役割 |
|---|---|---|
| 共通ヘルパー | `tests/helpers.py` | API ラッパー・ポーリング・フィクスチャ生成 |
| テストスクリプト | `tests/s*.py` | 各スイートのシナリオ実装 |
| Gymnasium SDK | `python/env.py` | 強化学習 / AI エージェント向け RL 環境 |
| Python パッケージ | `python/__init__.py` | `from flowsurface import FlowsurfaceEnv` でインポート可能 |

**bash → Python 移行**: 旧バージョンの bash スクリプトは `tests/archive/` に保存済み。
現行スクリプトはすべて Python（`uv run` または `pytest` で実行可能）。

---

## 2. ディレクトリ構成

```
tests/
├── helpers.py              共通ヘルパーライブラリ
├── s1_basic_lifecycle.py   S1: 基本ライフサイクル
├── s2_persistence.py       S2: 設定永続化
├── s3_autoplay.py          S3: 自動再生
├── s5_tachibana_mixed.py   S5: 立花証券混在 (PEND 中)
├── s10_range_end.py〜      S10〜: 各検証シナリオ
├── s34_virtual_order_basic.py  S34: 仮想注文 API 基本
├── s35_virtual_portfolio.py    S35: ポートフォリオ管理
├── s40_virtual_order_fill_cycle.py  S40: 約定サイクル E2E
├── s41_limit_order_round_trip.py〜  S41〜: 注文 API 各種
├── archive/                旧 bash スクリプト群
└── helpers.py              ※ 各スクリプトの冒頭で sys.path.insert して import

python/
├── __init__.py             `from flowsurface import FlowsurfaceEnv`
└── env.py                  FlowsurfaceEnv（Gymnasium 互換）
```

---

## 3. 環境変数

| 変数 | デフォルト | 説明 |
|---|---|---|
| `E2E_TICKER` | `BinanceLinear:BTCUSDT` | テスト対象ティッカー（`ExchangeName:Symbol` 形式） |
| `IS_HEADLESS` | `""` (false) | `"true"` にすると headless モード（GUI なし）で実行 |
| `FLOWSURFACE_BINARY` | PATH 検索 | `flowsurface` バイナリのパスを明示指定 |
| `FLOWSURFACE_API_PORT` | `9876` | HTTP API ポート（`FlowsurfaceEnv` 経由で渡す） |
| `APPDATA` | OS 標準 | `saved-state.json` の格納先ディレクトリ（Windows: `%APPDATA%/flowsurface`） |
| `DEV_IS_DEMO` | `"true"` (SDK) | デモモード。`FlowsurfaceEnv` はデフォルトで設定する |

---

## 4. helpers.py — 共通ヘルパーライブラリ

### 4.1 定数

```python
API_BASE  = "http://127.0.0.1:9876"
TICKER    = os.environ.get("E2E_TICKER", "BinanceLinear:BTCUSDT")
IS_HEADLESS = os.environ.get("IS_HEADLESS", "").lower() == "true"

STEP_M1  = 60_000     # ミリ秒
STEP_M5  = 300_000
STEP_H1  = 3_600_000
STEP_D1  = 86_400_000

DATA_DIR    = Path(os.environ.get("APPDATA", "")) / "flowsurface"
STATE_FILE  = DATA_DIR / "saved-state.json"
STATE_BACKUP = DATA_DIR / "saved-state.json.bak"
```

### 4.2 レポートヘルパー

```python
pass_(label: str) -> None         # PASS カウント +1、"  PASS: {label}" を出力
fail(label: str, detail: str) -> None   # FAIL カウント +1、"  FAIL: {label} — {detail}" を出力
pend(label: str, reason: str) -> None   # PEND カウント +1（未実装 / 条件付きスキップ）
print_summary() -> None           # "PASS: X  FAIL: Y  PEND: Z" を出力
reset_counters() -> None          # グローバルカウンタをリセット
```

> **PEND の用途**: `IS_HEADLESS=true` 時に GUI 専用テストをスキップする、または未実装機能を保留扱いにする場合に使用する。PEND は失敗ではないため `sys.exit(1)` の条件には含まれない。

### 4.3 API ラッパー

```python
api_get(path: str) -> dict
    # GET {API_BASE}{path}、HTTP 200 以外は例外。レスポンス JSON を返す。

api_post(path: str, body: dict | None) -> dict
    # POST {API_BASE}{path}、HTTP 200 以外は例外。レスポンス JSON を返す。

api_get_code(path: str) -> int
    # GET のステータスコードのみ返す（接続失敗時は 0）

api_post_code(path: str, body: Any) -> int
    # POST のステータスコードのみ返す（接続失敗時は 0）
    # body が dict → json=body, str/bytes → data=body

get_status() -> dict
    # GET /api/replay/status のショートカット
```

### 4.4 ポーリングヘルパー

```python
wait_status(want: str, timeout: int = 10) -> bool
    # /api/replay/status の "status" フィールドが want になるまでポーリング

wait_playing(timeout: int = 120) -> bool       # status == "Playing"
wait_paused(timeout: int = 15) -> bool         # status == "Paused"

wait_for_time_advance(ref: int, timeout: int = 30) -> int | None
    # current_time > ref になるまでポーリング。新しい値を返す。タイムアウト時は None。

wait_streams_ready(timeout: int = 30) -> bool
    # pane/list[0].streams_ready == true になるまで待つ

wait_for_pane_count(want: int, timeout: int = 10) -> bool
    # pane/list の配列長が want になるまでポーリング

wait_for_pane_streams_ready(pane_id: str, timeout: int = 30) -> bool
    # 指定 pane_id の streams_ready == true になるまでポーリング

wait_tachibana_session(timeout: int = 120) -> bool
    # GET /api/auth/tachibana/status → session == "present" になるまで待つ
```

### 4.5 フィクスチャ

```python
setup_single_pane(ticker: str, timeframe: str, start: str, end: str) -> None
    # saved-state.json に単一 KlineChart ペインのフィクスチャを書き込む。
    # IS_HEADLESS=true のときは start/end を内部変数に保存するのみ（ファイル書き込みなし）。

write_live_fixture(ticker: str = TICKER, timeframe: str = "M1", name: str = "Test") -> None
    # Live モード起動用フィクスチャ（replay フィールドなし）

tachibana_replay_setup(start: str, end: str) -> bool
    # TachibanaSpot:7203 D1 の saved-state.json を書き込む

backup_state() -> None    # saved-state.json → .bak にコピー
restore_state() -> None   # .bak → saved-state.json に戻す
```

### 4.6 検証ヘルパー

```python
is_bar_boundary(ct: int, step: int) -> bool
    # ct % step == 0 かどうか（バー境界スナップの検証）

advance_within(ct1: int, ct2: int, step: int, max_bars: int = 100) -> bool
    # ct2 - ct1 が step の正の整数倍 かつ 1〜max_bars バー以内か検証

ct_in_range(ct: int, st: int, et: int) -> bool
    # st <= ct <= et

utc_offset(hours: float) -> str
    # UTC 基準で ±hours 時間オフセット。"YYYY-MM-DD HH:MM" 形式。

utc_to_ms(dt_str: str) -> int
    # "YYYY-MM-DD HH:MM" を UTC ミリ秒に変換
```

### 4.7 ティッカーヘルパー

```python
order_symbol() -> str     # "BinanceLinear:BTCUSDT" → "BTCUSDT"
ticker_exchange() -> str  # "BinanceLinear:BTCUSDT" → "BinanceLinear"
primary_ticker() -> str   # TICKER そのまま
secondary_ticker() -> str # 同取引所の別銘柄（例: ETHUSDT）
tertiary_ticker() -> str  # 同取引所の 3 銘柄目（例: SOLUSDT）
get_pane_id(index: int = 0) -> str          # pane/list[index].id
find_other_pane_id(exclude_id: str) -> str  # exclude_id 以外の最初のペイン ID
```

**セカンダリ・ターシャリ ティッカーのマッピング**:

| 取引所 | secondary | tertiary |
|---|---|---|
| HyperliquidLinear/Spot | `{ex}:ETH` | `{ex}:HYPE` |
| BinanceLinear/Spot | `{ex}:ETHUSDT` | `{ex}:SOLUSDT` |
| BybitLinear/Spot | `{ex}:ETHUSDT` | `{ex}:SOLUSDT` |

### 4.8 モードヘルパー

```python
headless_play(start: str = "", end: str = "") -> None
    # headless 時のみ POST /api/replay/play を発行。
    # GUI は saved-state 自動再生のため no-op。

ensure_replay_mode() -> None
    # GUI 時は toggle → Replay モードへ切替。headless は常に Replay のため no-op。

speed_to_10x() -> None
    # CycleSpeed を 3 回呼び出して 1x→2x→5x→10x にする
```

### 4.9 通知ヘルパー

```python
list_notifications() -> dict
    # GET /api/notification/list のレスポンスを返す

has_notification(needle: str) -> bool
    # title または body に needle が含まれる通知があれば True

count_error_notifications() -> int
    # level が "error" または "warning" の通知件数を返す
```

---

## 5. FlowsurfaceEnv — Gymnasium 互換 RL 環境

**ファイル**: `python/env.py`

```python
from flowsurface import FlowsurfaceEnv
# または
from env import FlowsurfaceEnv
```

### 5.1 概要

`gym.Env` を継承した Gymnasium 互換の強化学習環境。flowsurface を `--headless` モードで
サブプロセスとして起動し、HTTP API 経由でリプレイを制御する。

| 概念 | 実装 |
|---|---|
| 観測（observation） | 直近 `kline_limit` 本の OHLC フラット配列（float32） |
| 行動（action） | `{"side": int[0-2], "qty": float[0-1]}`（0=hold, 1=buy, 2=sell） |
| 報酬（reward） | ステップ間の `total_equity` 増減 |
| エピソード終了 | `current_time >= end_time` かつ status=Paused |

### 5.2 コンストラクタ引数

| 引数 | 型 | デフォルト | 説明 |
|---|---|---|---|
| `ticker` | `str` | `"HyperliquidLinear:BTC"` | ティッカー |
| `timeframe` | `str` | `"M1"` | タイムフレーム |
| `binary_path` | `str \| None` | `None`（PATH 検索） | flowsurface バイナリパス |
| `api_port` | `int` | `9876` | HTTP API ポート |
| `initial_cash` | `float` | `1_000_000.0` | 初期キャッシュ（報酬計算の基点） |
| `kline_limit` | `int` | `60` | 観測に含める最大バー本数 |
| `headless` | `bool` | `True` | headless フラグ（`True` 必須） |

### 5.3 Gymnasium API

```python
obs, info = env.reset(start="2024-01-01 00:00", end="2024-01-01 02:00")
# POST /api/replay/play → Paused になるまで待機 → 観測を返す

obs, reward, done, truncated, info = env.step(action)
# 1. side!=0 なら POST /api/replay/order（成行）
# 2. POST /api/replay/step-forward で 1 バー進める
# 3. GET /api/replay/portfolio → total_equity を取得し reward を計算

env.close()
# サブプロセスを terminate → kill
```

**`observation_space`**: `Box(low=0, high=inf, shape=(kline_limit*4,), dtype=float32)`

**`action_space`**: `Dict({"side": Discrete(3), "qty": Box(0,1)})`

### 5.4 プロセス管理

```python
env._start_process()
# cmd: [binary, "--headless", "--ticker", ticker, "--timeframe", timeframe]
# env: DEV_IS_DEMO=true, FLOWSURFACE_API_PORT={api_port}
# /api/replay/status が 200 を返すまで 30s 以内待機
```

バイナリ探索順序:
1. `FLOWSURFACE_BINARY` 環境変数
2. `PATH` 上の `flowsurface`
3. `target/debug/flowsurface.exe`
4. `target/release/flowsurface.exe`
5. `target/debug/flowsurface`
6. `target/release/flowsurface`

---

## 6. テストスクリプト パターン

### 6.1 スクリプト構造

すべてのテストスクリプトは以下の構造に従う:

```python
#!/usr/bin/env python3
"""sXX_name.py — Suite SXX: 説明"""
from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    api_get, api_post, backup_state, restore_state,
    pass_, fail, pend, print_summary,
    setup_single_pane, utc_offset, wait_status,
    # ... 必要なものだけインポート
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv


def run_sXX(start: str, end: str) -> None:
    """テスト本体。FlowsurfaceEnv とは独立して実行できる。"""
    print("=== SXX: スイート名 ===")

    # TC-A: ...
    result = api_post("/api/...")
    if <条件>:
        pass_("TC-A: ...")
    else:
        fail("TC-A", f"detail={result}")


def test_sXX_name() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    import helpers; helpers._PASS = helpers._FAIL = helpers._PEND = 0
    start = utc_offset(-3); end = utc_offset(-1)
    run_sXX(start, end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed"


def main() -> None:
    start = utc_offset(-3); end = utc_offset(-1)

    backup_state()
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_sXX(start, end)
    finally:
        env.close()
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
```

### 6.2 GUI モード vs headless モード

| 項目 | GUI モード | headless モード |
|---|---|---|
| 起動方法 | `saved-state.json` 書き込み後に `env._start_process()` | 同左（`--headless` フラグあり） |
| リプレイ開始 | saved-state に `"replay"` フィールドを含める → 自動 Play | `headless_play(start, end)` を明示的に呼ぶ |
| ストリーム待機 | `wait_streams_ready()` が必要 | 不要（`wait_status("Playing")` で十分） |
| モード初期値 | `Live` | `Replay` |
| StepBackward | 動作する | 未実装（PEND 扱い） |
| スクリーンショット | 可能 | 不可 |

### 6.3 テストケース命名規則

```
TC-{スイート番号}-{連番}     例: TC-S1-01, TC-S34-E
TC-{アルファベット}           例: TC-A, TC-B（簡潔なスイートで使用）
TC-{スイート番号}-H{連番}    例: TC-S1-H09（HTTP API 検証）
```

### 6.4 終了コード

```python
if _FAIL > 0:
    sys.exit(1)   # CI で FAIL 検出
```

PEND は終了コードに影響しない。CI では PEND が多すぎないか定期的にレビューする。

---

## 7. テストスイート一覧

| スイート | ファイル | 概要 |
|---|---|---|
| S1 | `s1_basic_lifecycle.py` | リプレイ基本ライフサイクル（Play/Pause/Resume/Speed/Step） |
| S2 | `s2_persistence.py` | 設定の永続化（saved-state.json） |
| S3 | `s3_autoplay.py` | 起動時自動再生 |
| S5 | `s5_tachibana_mixed.py` | 立花証券+Binance 混在（PEND 中） |
| S6 | `s6_mixed_timeframes.py` | 複数タイムフレーム |
| S7 | `s7_mid_replay_pane.py` | リプレイ中のペイン操作 |
| S8 | `s8_error_boundary.py` | エラー境界（不正操作） |
| S9 | `s9_speed_step.py` | 速度ステップ |
| S10 | `s10_range_end.py` | 範囲終端動作 |
| S11 | `s11_bar_step_discrete.py` | バーステップ離散性 |
| S12 | `s12_pre_start_history.py` | 開始前履歴 |
| S13 | `s13_step_backward_quality.py` | StepBackward 品質 |
| S14 | `s14_autoplay_event_driven.py` | イベント駆動オートプレイ |
| S16 | `s16_replay_resilience.py` | リプレイ耐久性 |
| S17 | `s17_error_boundary.py` | エラー境界 |
| S18 | `s18_endurance.py` | 長時間耐久 |
| S23 | `s23_mid_replay_ticker_change.py` | リプレイ中ティッカー変更 |
| S24 | `s24_sidebar_select_ticker.py` | サイドバーティッカー選択 |
| S26 | `s26_ticker_change_after_replay_end.py` | リプレイ終了後ティッカー変更 |
| S27 | `s27_cyclespeed_reset.py` | 速度リセット |
| S28 | `s28_ticker_change_while_loading.py` | ロード中ティッカー変更 |
| S30 | `s30_mixed_sample_loading.py` | 混在サンプルロード |
| S31 | `s31_replay_end_restart.py` | リプレイ終端から再スタート |
| S32 | `s32_toyota_candlestick_add.py` | トヨタ銘柄ローソク足追加 |
| S33 | `s33_sidebar_split_pane.py` | サイドバーペイン分割 |
| S34 | `s34_virtual_order_basic.py` | 仮想注文 API 基本動作 |
| S35 | `s35_virtual_portfolio.py` | 仮想ポートフォリオ管理 |
| S36 | `s36_sidebar_order_pane.py` | サイドバー注文パネル |
| S37 | `s37_order_panels_integrated.py` | 注文パネル統合 |
| S39 | `s39_buying_power_portfolio.py` | 買付余力・ポートフォリオ |
| S40 | `s40_virtual_order_fill_cycle.py` | 仮想注文約定サイクル（buy→fill→PnL） |
| S41 | `s41_limit_order_round_trip.py` | 指値注文ラウンドトリップ |
| S42 | `s42_naked_short_cycle.py` | ネイキッドショートサイクル |
| S43 | `s43_get_state_endpoint.py` | GET /api/replay/state エンドポイント |
| S44 | `s44_order_list.py` | 注文一覧 |
| S45 | `s45_order_correct_cancel.py` | 注文訂正・取消 |
| S46 | `s46_wrong_password.py` | パスワード誤り |
| S47 | `s47_outside_hours.py` | 営業時間外 |
| S48 | `s48_invalid_issue.py` | 無効銘柄 |
| S49 | `s49_account_info.py` | 口座情報 |

---

## 8. 実行方法

```bash
# 個別スクリプトを直接実行（アプリが別途起動済みの場合）
python tests/s1_basic_lifecycle.py

# 環境変数でティッカーを指定
E2E_TICKER=HyperliquidLinear:BTC python tests/s1_basic_lifecycle.py

# headless モード（GUI なし）
IS_HEADLESS=true python tests/s34_virtual_order_basic.py

# uv で実行（依存解決付き）
uv run tests/s1_basic_lifecycle.py

# pytest で実行（プロセス起動は外部で行うこと）
pytest tests/s34_virtual_order_basic.py -v

# 複数スクリプトを順次実行
for f in tests/s{34,35,40}.py; do python "$f"; done
```

**依存ライブラリ** (E2E テスト):
```
requests
```

**依存ライブラリ** (Gymnasium SDK):
```
gymnasium
numpy
requests
```

---

## 9. フィクスチャ設計

### 9.1 saved-state.json スキーマ

```json
{
  "layout_manager": {
    "layouts": [
      {
        "name": "Test-M1",
        "dashboard": {
          "pane": {
            "KlineChart": {
              "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
              "kind": "Candles",
              "stream_type": [
                { "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }
              ],
              "settings": {
                "tick_multiply": null,
                "visual_config": null,
                "selected_basis": { "Time": "M1" }
              },
              "indicators": ["Volume"],
              "link_group": "A"
            }
          },
          "popout": []
        }
      }
    ],
    "active_layout": "Test-M1"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

### 9.2 リプレイ付きフィクスチャ

起動時に自動再生させる場合は `"replay"` フィールドを追加する:

```json
{
  "...": "...(上記と同様)...",
  "replay": {
    "mode": "replay",
    "range_start": "2024-01-01 00:00",
    "range_end": "2024-01-01 02:00"
  }
}
```

**headless 時の注意**: `IS_HEADLESS=true` の場合、`setup_single_pane()` はファイルを書き込まず、`headless_play()` から自動的に `range_start` / `range_end` が POST される。

---

## 10. 既知の制限・注意事項

| # | 注意点 |
|---|---|
| 1 | `StepBackward` は headless モードで未実装。`IS_HEADLESS=true` 時は `pend()` でスキップする |
| 2 | `wait_streams_ready()` は GUI 起動後にのみ必要。headless は `wait_playing()` で十分 |
| 3 | pytest 実行時はプロセス起動を外部で行うこと（`main()` はスタンドアロン用） |
| 4 | `helpers.py` のグローバルカウンタは pytest 並列実行で競合する。`helpers._PASS = helpers._FAIL = helpers._PEND = 0` でリセットすること |
| 5 | `api_post_code()` に `dict` 以外（`str` / `bytes`）を渡す場合は `Content-Type: application/json` が自動付与される |
| 6 | `secondary_ticker()` / `tertiary_ticker()` は取引所ベースの固定マッピング。新取引所追加時は `helpers.py` のマッピングを更新する |
| 7 | 立花証券スイート（S5・S19〜S22・S29）は実認証が必要。`wait_tachibana_session()` で最大 120s 待機する |
