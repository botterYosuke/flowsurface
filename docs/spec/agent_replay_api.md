# Agent 専用 Replay API 仕様書（Phase 4b-1）

> **関連ドキュメント**:
> | 知りたいこと | 参照先 |
> |---|---|
> | 設計判断の背景 | [ADR-0001](../adr/0001-agent-replay-api-separation.md) |
> | 実装計画・サブフェーズ構成 | [plan/phase4b_agent_replay_api.md](../plan/phase4b_agent_replay_api.md) |
> | UI リモコン API | [replay.md §11](replay.md#11-http-制御-api) |
> | ナラティブ API（Phase 4a）| [narrative.md](narrative.md) |
> | Python SDK | [python.md](python.md) |

**最終更新**: 2026-04-22
**対象ブランチ**: `feat/phase4b-subphase-*`（Phase 4b-1 完了時点）

本書は agent 専用 Replay API `/api/agent/session/:id/*` を、実装・利用・運用に十分な
粒度で説明するリファレンス仕様書である。UI リモコン API `/api/replay/*` とは**意図的に分離**
された経路である背景は [ADR-0001](../adr/0001-agent-replay-api-separation.md) を参照。

---

## 目次

1. [概要](#1-概要)
2. [セッション ID 規約](#2-セッション-id-規約)
3. [型契約](#3-型契約)
4. [エンドポイント](#4-エンドポイント)
5. [エラーモデル](#5-エラーモデル)
6. [SessionLifecycleEvent 購読](#6-sessionlifecycleevent-購読)
7. [Getting Started](#7-getting-started)
8. [定数と設計不変条件](#8-定数と設計不変条件)
9. [実装ファイルマップ](#9-実装ファイルマップ)

---

## 1. 概要

Phase 4a で UI リモコン `/api/replay/*` を Python SDK / uAgent から叩いたところ、
型契約の不在・非アトミック観測・wall-time 駆動によって 3 件の silent failure が
発生した（[narrative.md §13.2](narrative.md)）。これを構造的に再発不能にするため、
agent 専用 API を別経路として分離した。

### 1.1 主な特徴

| 特徴 | 内容 |
|---|---|
| 型契約の厳格化 | `Ticker = { exchange, symbol }` 構造体必須、`order_type` 明示必須 |
| 決定論性 | `step` レスポンスに当該 tick の全副作用（fills / updated_narrative_ids）を同梱、polling 不要 |
| Wall-time 非依存 | `advance { until_ms }` で任意区間を instant 実行（Headless 限定） |
| 冪等性 | `client_order_id` による重複発注抑止、body 相違時は 409 |
| セッション契約 | path `:id` を先行確定、Phase 4b-1 では `"default"` 固定 |

### 1.2 非ゴール（Phase 4b-1）

- **UI リモコン API の置き換え**: `/api/replay/*` は Phase 4c まで並存。facade 化は Phase 4c
- **複数 session 並行**: Phase 4c（本 Phase では `"default"` のみ受理、非 default は 501）
- **`step-backward` の agent 契約化**: ADR-0001 でスコープ外と確定（forward-only）
- **GUI ランタイムからの agent API 利用**: `step` / `order` は 501 スタブ、`advance` は 400

---

## 2. セッション ID 規約

**Phase 4b-1 では `:id` は必ず `"default"` を指定すること。** 非 `"default"` は `501 Not Implemented`
で拒否される（`400` でも `404` でもなく `501`）。

```
# OK
POST /api/agent/session/default/step

# NG (501 Not Implemented)
POST /api/agent/session/other/step
POST /api/agent/session/550e8400-e29b-41d4-a716-446655440000/step
```

Phase 4c で複数 session に拡張される際、path 契約は後方互換で維持される。

---

## 3. 型契約

境界型はすべて `src/api/contract/` に集約されている（既存 `SerTicker` 等の内部型とは意図的に分離）。

### 3.1 `TickerContract` — 構造体必須

```json
{"exchange": "HyperliquidLinear", "symbol": "BTC"}
```

- 文字列 `"HyperliquidLinear:BTC"` は 400
- `#[serde(deny_unknown_fields)]`: 未知フィールドは 400
- 空文字 `exchange` / `symbol` は 400（ルート層で検証）

### 3.2 `EpochMs(u64)` — Unix ミリ秒

API 境界で `u64` として扱うタイムスタンプ。レスポンス JSON では透過整数として直接 serialize される。
`i64` への変換は `try_as_i64()` で overflow 検出（silent 負値化を防ぐ）。

### 3.3 `ClientOrderId` — 冪等性キー

- 制約: `1..=64` 文字、`[A-Za-z0-9_-]` のみ
- 違反は 400（regex 不一致・空文字・65 字以上）
- サーバー側は `(session_id, client_order_id)` → `(order_id, request_key)` の in-memory map を保持

### 3.4 `AgentOrderRequestKey` — 冪等性比較キー

`client_order_id` を除く `(ticker, side, qty, order_type)` の構造的等価で比較。
`derive(PartialEq)` + `f64` bit equality（agent が同じ入力で同じ f64 を再生成する前提）。

---

## 4. エンドポイント

| Method | Path | 用途 | 実装サブフェーズ |
|---|---|---|:-:|
| `POST` | `/api/agent/session/:id/step` | 1 bar 進行 + 副作用同梱 | C, D |
| `POST` | `/api/agent/session/:id/advance` | 任意区間 instant 実行（Headless のみ）| G |
| `POST` | `/api/agent/session/:id/order` | 仮想注文（冪等性あり） | E |

### 4.1 `POST /api/agent/session/:id/step`

**リクエスト**: ボディ不要（`{}` でも可）

**レスポンス 200**:

```jsonc
{
  "clock_ms": 1704067260000,
  "reached_end": false,
  "observation": {
    "ohlcv": [
      {"stream": "HyperliquidLinear:BTC:1m", "time": 1704067200000,
       "open": 92100.0, "high": 92800.0, "low": 91900.0, "close": 92500.0, "volume": 1234.5}
    ],
    "recent_trades": [...],
    "portfolio": {"cash": 1000000.0, "unrealized_pnl": 0.0, "realized_pnl": 0.0,
                  "total_equity": 1000000.0, "open_positions": [], "closed_positions": []}
  },
  "fills": [
    {"order_id": "ord_uuid", "client_order_id": "cli_42",
     "fill_price": 92100.5, "qty": 0.1, "side": "buy", "fill_time_ms": 1704067260000}
  ],
  "updated_narrative_ids": ["uuid_a", "uuid_b"]
}
```

**不変条件**:
- `fills` / `updated_narrative_ids` は同期確定（fire-and-forget しない）
- `reached_end: true` で clock は据え置き（それ以上 step しても進まない）
- `fills[].client_order_id` は agent API 経由で発注した注文のみ埋まる（UI 経由発注は `null`）

### 4.2 `POST /api/agent/session/:id/advance`

**リクエスト**:

```jsonc
{
  "until_ms": 1706659200000,             // 必須（EpochMs）
  "stop_on": ["fill", "narrative"],      // オプション。["end"] は 400
  "include_fills": true                  // オプション、デフォルト false
}
```

**レスポンス 200**:

```jsonc
{
  "clock_ms": 1706659200000,
  "stopped_reason": "until_reached",    // | "fill" | "narrative" | "end"
  "ticks_advanced": 43200,
  "aggregate_fills": 12,
  "aggregate_updated_narratives": 8,
  "fills": [...],                        // include_fills=true のときのみ
  "final_portfolio": {...}
}
```

**不変条件**:
- `observation` は返さない（数万 tick のシリアライズコストを避ける）。必要なら直後に `step` or 別 API で取得
- `stop_on` は `["fill", "narrative"]` の部分集合。`"end"` は不正値（範囲終端は常に停止）
- **Headless ランタイムでのみ受理**。GUI で叩くと `400 {"error":"instant mode requires headless runtime (pass --headless)"}`

### 4.3 `POST /api/agent/session/:id/order`

**リクエスト**:

```jsonc
{
  "client_order_id": "cli_42",                                // 必須
  "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"}, // 構造体必須
  "side": "buy",                                              // "buy" | "sell"
  "qty": 0.1,                                                 // 正の有限値
  "order_type": {"market": {}}                                // または {"limit": {"price": 92500.0}}
}
```

**レスポンス 200**:

```jsonc
{
  "order_id": "ord_server_uuid",
  "client_order_id": "cli_42",
  "idempotent_replay": false    // 再送時は true
}
```

**不変条件**:
- Created / IdempotentReplay の両方で **status 200**（body 内 `idempotent_replay` フラグで分岐）
- `client_order_id` 重複 & body 相違 → `409 Conflict`
- `order_type` 省略 → 400（silent market default 防止）
- 文字列 `ticker` → 400（silent 正規化防止）

---

## 5. エラーモデル

| Status | 条件 | Body |
|---|---|---|
| 200 | 新規受付 / 冪等再送 | `{..., "idempotent_replay": bool}` |
| 400 | 型違反（`ticker` 文字列・`order_type` 欠落・`client_order_id` regex 違反等） | `{"error": "<具体的メッセージ>"}` |
| 400 | `advance` が GUI ランタイム | `{"error":"instant mode requires headless runtime (pass --headless)"}` |
| 404 | セッション未初期化 | `{"error":"session not started", "hint":"start a replay session first (see agent_replay_api.md Getting Started)"}` |
| 409 | `client_order_id` 重複 & body 相違 | `{"error":"client_order_id conflict with different request body", "existing_order_id": "..."}` |
| 501 | `session_id != "default"` | `{"error":"multi-session not yet implemented; use 'default' until Phase 4c"}` |
| 503 | セッション loading 中 | `{"error":"session loading"}` |

---

## 6. SessionLifecycleEvent 購読

ADR-0001 の核不変条件: UI リモコン API ハンドラは agent API state を直接触らない。
代わりに `VirtualExchange::session_generation()` カウンタを経由して `SessionLifecycleEvent`
（`Started` / `Reset` / `Terminated`）を購読する。

```
UI /api/replay/play       → virtual_engine.mark_session_started()  → generation + 1
UI /api/replay/step-backward → virtual_engine.mark_session_reset() → generation + 1
UI /api/replay/toggle (Live) → virtual_engine.mark_session_terminated() → generation + 1
```

Agent API の state (`AgentSessionState`) は発注・観測のたびに
`observe_generation(virtual_engine.session_generation())` を呼び、値が変わっていれば
`client_order_id` map を自動クリアする。

**実用的な帰結**:
- UI 経由で `/play` を再実行すると、agent が保持していた `client_order_id → order_id`
  の紐付けはクリアされる（同じ `client_order_id` を別注文に再利用可能）
- `step-backward` / `seek` も同様（巻き戻した tick で異なる戦略を試せる）

---

## 7. Getting Started

Phase 4b-1 時点では、agent API を使う前に **UI リモコン API でリプレイセッションを起動**する必要がある
（本 Phase では agent API 側の `session start` エンドポイントは実装していない — plan §5.1 の妥協点）。

```bash
# 1. アプリを起動（--headless 推奨。advance は headless 限定）
./target/release/flowsurface --headless --ticker BinanceLinear:BTCUSDT --timeframe M1

# 2. リプレイセッションを起動（UI リモコン API 経由）
curl -X POST http://127.0.0.1:9876/api/replay/play \
  -H "Content-Type: application/json" \
  -d '{"start": "2024-01-15 09:00", "end": "2024-01-15 15:30"}'

# 3. agent API を叩く
curl -X POST http://127.0.0.1:9876/api/agent/session/default/step

# 4. 仮想注文（冪等性あり）
curl -X POST http://127.0.0.1:9876/api/agent/session/default/order \
  -H "Content-Type: application/json" \
  -d '{
    "client_order_id": "cli_42",
    "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
    "side": "buy",
    "qty": 0.1,
    "order_type": {"market": {}}
  }'

# 5. Headless のみ: 任意区間 instant 実行
curl -X POST http://127.0.0.1:9876/api/agent/session/default/advance \
  -H "Content-Type: application/json" \
  -d '{"until_ms": 1706659200000, "stop_on": ["fill"]}'
```

### 7.1 Python SDK

```python
import flowsurface as fs

# セッション起動（UI リモコン API 経由、Phase 4b-1 の現状）
fs._client.post("/api/replay/play",
                {"start": "2024-01-15 09:00", "end": "2024-01-15 15:30"})

# agent API（型付きレスポンス）
resp = fs.agent_session.step()
for fill in resp.fills:
    print(fill.client_order_id, fill.fill_price)

fs.agent_session.place_order(
    client_order_id="cli_42",
    ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
    side="buy", qty=0.1,
    order_type={"market": {}},
)
```

---

## 8. 定数と設計不変条件

### 8.1 定数

| 定数 | 値 | 定義箇所 | 意味 |
|---|---|---|---|
| `ClientOrderId` 長 | 1..=64 | `src/api/contract/client_order_id.rs` | regex 検証 |
| `ClientOrderId` charset | `[A-Za-z0-9_-]` | 同上 | |
| R1 narrative sync p95 | 100ms | `src/headless.rs::agent_session_step` | 超過で WARN ログ（非同期化切替の判定基準） |
| `HashMap<ClientOrderId, ...>` 逆引き O(n) WARN | 1000 件 | `src/api/agent_session_state.rs::client_order_id_for` | 超過で WARN ログ（Phase 4c で逆引き index 追加判定） |

### 8.2 設計上の不変条件

| # | 不変条件 | 破壊したときの症状 |
|:-:|---|---|
| 1 | `session_id != "default"` は 501（400 / 黙殺禁止） | 将来の複数 session 対応で silent 退行 |
| 2 | `Ticker` は構造体 JSON 必須、文字列拒否 | Phase 4a silent failure #3 の再発 |
| 3 | `order_type` 省略は 400（silent market default 禁止） | Phase 4a silent failure パターンの再発 |
| 4 | `step` レスポンスの `fills` / `updated_narrative_ids` は同期確定 | polling ループが必要になり決定論が失われる |
| 5 | `advance` は Headless runtime のみ（GUI は 400） | iced 再描画と競合して UI が凍結 |
| 6 | `stop_on` に `"end"` を含めると 400 | 既に常時停止する条件を明示指定する混乱を招く |
| 7 | UI リモコン API ハンドラは agent state を直接触らない | Phase 4c の facade 化で購読経路が壊れる |
| 8 | `/api/replay/*` に新規ルートを追加しない（ADR-0001） | 2 系統 API が定着し facade 化が不可能になる |
| 9 | `client_order_id` UNIQUE は in-memory（session 再起動でクリア） | 再起動後の冪等性保証は不要（仕様上 OK） |

---

## 9. 実装ファイルマップ

| ファイル | 責務 |
|---|---|
| [src/api/mod.rs](../../src/api/mod.rs) | module root |
| [src/api/contract/epoch.rs](../../src/api/contract/epoch.rs) | `EpochMs(u64)` newtype |
| [src/api/contract/ticker.rs](../../src/api/contract/ticker.rs) | `TickerContract { exchange, symbol }` |
| [src/api/contract/client_order_id.rs](../../src/api/contract/client_order_id.rs) | `ClientOrderId` + validation |
| [src/api/order_request.rs](../../src/api/order_request.rs) | `AgentOrderRequest` / `AgentOrderRequestKey` |
| [src/api/advance_request.rs](../../src/api/advance_request.rs) | `AgentAdvanceRequest` / `AdvanceResponse` |
| [src/api/step_response.rs](../../src/api/step_response.rs) | `StepResponse` / `StepFill` / `StepObservation` |
| [src/api/agent_session_state.rs](../../src/api/agent_session_state.rs) | `AgentSessionState` + 冪等性マップ |
| [src/replay/virtual_exchange/mod.rs](../../src/replay/virtual_exchange/mod.rs) | `SessionLifecycleEvent` + `session_generation()` |
| [src/replay_api.rs](../../src/replay_api.rs) | HTTP ルーティング（`AgentSessionCommand`）|
| [src/headless.rs](../../src/headless.rs) | headless ハンドラ（`agent_session_step` / `_order` / `_advance`）|
| [src/app/api/mod.rs](../../src/app/api/mod.rs) | GUI ハンドラ（501 / 400 スタブ） |
| [python/agent_session.py](../../python/agent_session.py) | Python SDK (`fs.agent_session`) |
| [tests/e2e/s5[5-9]_*.py](../../tests/e2e/) | E2E テスト S55〜S59 |
| [tests/python/test_agent_session.py](../../tests/python/test_agent_session.py) | Python integration / offline dataclass tests |
| [docs/adr/0001-agent-replay-api-separation.md](../adr/0001-agent-replay-api-separation.md) | ADR |
| [docs/plan/phase4b_agent_replay_api.md](../plan/phase4b_agent_replay_api.md) | 実装計画 |
| [.github/workflows/adr_guard.yml](../../.github/workflows/adr_guard.yml) | UI リモコン API 新規ルート検知 CI |
