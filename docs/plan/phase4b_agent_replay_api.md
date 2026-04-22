# Phase 4b-1: Agent 専用 Replay API 実装計画

**親計画**: [🔄ai_agent_platform_roadmap.md](🔄ai_agent_platform_roadmap.md)
**関連 ADR**: [ADR-0001 Agent 専用 Replay API の分離](../adr/0001-agent-replay-api-separation.md)
**前提フェーズ**: Phase 1・2・3・4a 完了済み（2026-04-21 時点）
**起案日**: 2026-04-22
**TDD 方針**: `.claude/skills/tdd-workflow/SKILL.md` に準拠（RED → GREEN → REFACTOR）

---

## 0. 位置づけ

ロードマップ上の **Phase 4b（ASI Alliance 統合）** は以下 2 段に分割する:

| 段 | 内容 | 本計画 |
|---|---|:-:|
| **4b-1** | Agent 専用 Replay API を `/api/agent/session/:id/*` に新設（ADR-0001 の実装） | ✅ 本計画 |
| **4b-2** | uAgents / Agentverse ブリッジ（Python 側でナラティブを A2A 送受信） | 次計画 |

4b-2 の前提として **agent 契約が型厳格で決定論的である** ことが必要。したがって 4b-1 を先に完了させる。

---

## 1. ゴール

Python SDK（ひいては uAgent）が polling なし・型契約ありで flowsurface のリプレイを駆動できる HTTP API を提供する。Phase 4a で踏んだ silent failure 3 件（[narrative.md §13.2](../spec/narrative.md)）のクラスを構造的に再発不能にする。

### 成功条件（Definition of Done）

- [ ] `POST /api/agent/session/default/step` が当該 tick の `observation` / `fills` / `updated_narrative_ids` / `clock_ms` を同梱で返す（polling 不要）
- [ ] `POST /api/agent/session/default/advance { until_ms, stop_on }` が wall-time 非依存で任意区間を Headless ビルドで instant 実行できる
- [ ] `POST /api/agent/session/default/order` が `client_order_id` 必須・`(session_id, client_order_id)` 重複で idempotent replay を返す
- [ ] `Ticker = { exchange: String, symbol: String }` の構造体 JSON のみ受理。文字列 `"Exchange:Symbol"` は 400
- [ ] `EpochMs(u64)` newtype を API 境界に導入し、`exchange/` 層との型境界を明示
- [ ] `session_id` が `"default"` 以外の値は `501 Not Implemented` + 明示的な error body で拒否
- [ ] `advance` を GUI ビルドで叩くと 400 `{"error":"instant mode requires headless"}` を返す
- [ ] 既存 `/api/replay/*`（UI リモコン API）は無改修で動作し続ける（既存 E2E `tests/s*.sh` が全 PASS）
- [ ] Python SDK `fs.agent_session.*` で新 API をラップ
- [ ] E2E テスト S55〜S59 追加、Rust ユニットテストカバレッジ 80% 以上
- [ ] CI（`e2e.yml`・`format.yml`・`clippy -D warnings`）全 PASS

---

## 2. スコープ・非スコープ

### スコープ（本計画で実装する）

- 新エンドポイント群 `/api/agent/session/:id/*`（詳細 §4）
- `Ticker` / `EpochMs` / `ClientOrderId` の型新設と JSON スキーマ
- 既存 `VirtualExchange`（"default" session）への共有アクセス実装
- `PATCH .github/pull_request_template.md` で UI リモコン API への新規機能追加禁止を明記
- `src/replay_api.rs` の UI リモコンルート範囲検知 grep lint（CI）
- Python SDK ラッパー `python/agent_session.py`
- E2E テスト S55〜S59（詳細 §7）

### 非スコープ（次計画以降）

- **UI リモコン API の facade 化**（Phase 4c）— 本計画では新旧 2 系統並存
- **複数 session の並行実行**（Phase 4c）— 本計画は `"default"` 固定
- **uAgents / Agentverse ブリッジ**（Phase 4b-2）
- **DeltaV / SingularityNET 統合**（将来）
- `step-backward` の agent 契約化（ADR-0001 でスコープ外と確定）
- OpenAPI スキーマの自動生成（手書きの仕様書 `agent_replay_api.md` で代替）

---

## 3. 型契約

**方針宣言**: agent API 境界型は新規モジュール **`src/api/contract/`** に集約する。既存 `src/exchange/` や既存 Rust 型（`SerTicker` 等）には混ぜない — Phase 4a の `SerTicker` 文字列正規化 silent failure（narrative.md §13.2 #3）の教訓に従う。

```
src/api/
├── mod.rs
└── contract/
    ├── mod.rs
    ├── epoch.rs           # EpochMs
    ├── ticker.rs          # TickerContract
    └── client_order_id.rs # ClientOrderId
```

### 3.1 `EpochMs` newtype（`src/api/contract/epoch.rs`）

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMs(pub u64);

impl EpochMs {
    pub fn as_u64(self) -> u64 { self.0 }

    /// `i64` への変換は overflow 時にエラー。silent な負値化を防ぐ。
    pub fn try_as_i64(self) -> Result<i64, std::num::TryFromIntError> {
        i64::try_from(self.0)
    }
}
```

**適用範囲**: 新 API の境界のみ。既存 `u64 ms` / `chrono::DateTime` の内部実装は触らない（外周で変換）。`as i64` による暗黙キャストは禁止（silent な overflow を封じる）。

### 3.2 `TickerContract` 構造体（`src/api/contract/ticker.rs`）

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickerContract {
    pub exchange: String,  // 例: "HyperliquidLinear"
    pub symbol: String,    // 例: "BTC"
}
```

**禁則**: JSON 上で `"ticker": "HyperliquidLinear:BTC"` のような文字列は `400 Bad Request: ticker must be object, not string` で拒否。

### 3.3 `ClientOrderId`（`src/api/contract/client_order_id.rs`）

```rust
pub struct ClientOrderId(pub String);  // 1..=64 chars, [A-Za-z0-9_-]
```

サーバーは `(session_id, client_order_id)` UNIQUE を **プロセス内 `HashMap<(SessionId, ClientOrderId), OrderId>`** で管理（SQLite は使わない：永続化不要）。

**ライフサイクル境界でのクリア**:

map のクリアは UI リモコン API ハンドラ内では行わない（ADR-0001 の canonical 性維持のため、UI ハンドラが agent API 専用 state を知る構造を避ける）。代わりに **`VirtualExchange` がセッション状態遷移イベント `SessionLifecycleEvent` を発火し、agent API 側がそれを購読して map をクリアする**。

```rust
pub enum SessionLifecycleEvent {
    Started { session_id: SessionId },  // 新セッション開始（play / toggle → replay）
    Reset,                              // seek / step-backward で内部状態が巻き戻った
    Terminated,                         // toggle → live、アプリ終了
}
```

| イベント | 発火タイミング | map 挙動 |
|---|---|---|
| `Started` | `VirtualExchange::start_session` 呼び出し（UI `/play` / 将来の agent `start` 両方からトリガ） | 全クリア |
| `Reset` | `VirtualExchange::seek` / `rewind` 呼び出し | 全クリア（巻き戻した tick で同一 `client_order_id` を別注文に流用したいため）|
| `Terminated` | `VirtualExchange::stop_session` | 全クリア |
| 通常 tick 進行 | — | 保持 |

**設計意図**: UI リモコン `/api/replay/play` が agent API の state を直接触らない。将来 Phase 4c で UI リモコン API を facade 化する際、このイベント購読経路はそのまま温存できる。

#### 重複判定キー

deserialize 後の Rust struct 等価で判定する（`#[derive(PartialEq)]` に委ねる）。

- `ticker` / `side` / `order_type` enum は struct の構造的等価
- `qty` / `price` は **`f64` の bit 等価**（`f64::to_bits` 一致）。agent が同じリクエストを送れば同じ f64 が再生成される前提で十分。丸めルールを契約に持ち込まない（他言語 SDK 実装者の負担を避ける）
- JSON key 順・空白差異は serde deserialize 後には残らないため、そもそも問題にならない

**重複時の挙動**:
- 正規化キー完全一致 → `200` + `{ "order_id": ..., "idempotent_replay": true }`（§4.5 の 200 挙動に一致）
- 正規化キー差異 → `409 Conflict`（§4.5 参照）

---

## 4. API シェイプ

### 4.1 エンドポイント一覧

| Method | Path | 用途 | ボディ |
|---|---|---|---|
| `POST` | `/api/agent/session/:id/step` | 1 bar 進行 + 副作用同梱 | `{}` または省略可 |
| `POST` | `/api/agent/session/:id/advance` | 任意区間 instant 実行（Headless のみ） | `{ until_ms, stop_on?: ["fill","narrative"], include_fills?: bool }` |
| `GET`  | `/api/agent/session/:id/observation` | 現時点の観測スナップショット | — |
| `POST` | `/api/agent/session/:id/order` | 仮想注文（`client_order_id` 必須） | `OrderRequest`（§4.4）|
| `GET`  | `/api/agent/session/:id/portfolio` | ポジション・PnL | — |

**path 規則**: `:id` は現状 `"default"` のみ。非 `"default"` は全エンドポイントで `501 Not Implemented`。

### 4.2 `POST .../step` レスポンス

```jsonc
{
  "clock_ms": 1704067260000,           // tick 後の仮想時刻
  "reached_end": false,
  "observation": {                      // GET .../observation と同一スキーマ
    "ohlcv": [...],
    "recent_trades": [...],
    "portfolio": { "cash": ..., "equity": ..., "positions": [...] }
  },
  "fills": [                            // この tick で発火した FillEvent
    { "order_id": "ord_xxx", "client_order_id": "cli_42", "fill_price": 92100.5, "qty": 0.1, "side": "buy", "fill_time_ms": 1704067260000 }
  ],
  "updated_narrative_ids": ["uuid_a", "uuid_b"]  // outcome が本 tick で更新されたもの
}
```

**不変条件**:
- `fills` / `updated_narrative_ids` は `step` レスポンス同期で確定（fire-and-forget しない）。narrative outcome 更新が間に合わなければ step 自体を遅延させる。
- `reached_end: true` で `advance` 不能になる前の最後の状態を含む。

### 4.3 `POST .../advance` レスポンス

```jsonc
{
  "clock_ms": 1706659200000,
  "stopped_reason": "until_reached" | "fill" | "narrative" | "end",
  "ticks_advanced": 43200,
  "aggregate_fills": 12,                // 件数（常に返す）
  "aggregate_updated_narratives": 8,
  "fills": [ ... ],                     // include_fills=true のときのみ配列で同梱
  "final_portfolio": { ... }            // 終端時点のみ
}
```

**不変条件**:
- `observation` は返さない（数万 tick 分の OHLCV 窓は巨大）。呼び手が必要なら直後に `GET .../observation` を叩く。
- `stop_on` は `["fill", "narrative"]` の部分集合。`"end"` は値として受理しない（リプレイ区間終端は常に停止し `stopped_reason: "end"` を返すため、明示指定の意味がない）。不正値は 400。
- `stop_on` が空配列または省略なら `until_ms` 到達まで走り `stopped_reason: "until_reached"`。`["fill"]` を含めれば最初の FillEvent で停止し `stopped_reason: "fill"`。
- `include_fills: true` のときのみ `fills` フィールドを同梱（§4.2 の `step` レスポンス内 `fills` と同スキーマ）。デフォルト `false` は件数のみ（数万 tick 分のシリアライズコストを避ける）。Phase 4b-2 で uAgent が fill 詳細を narrative に使うケースで opt-in 利用を想定。
- **Headless ランタイム**でのみ受理。GUI ランタイム（`--headless` フラグなしの起動）で叩かれたら `400 { "error": "instant mode requires headless runtime (pass --headless)" }`。

### 4.4 `POST .../order` リクエスト

```jsonc
{
  "client_order_id": "cli_42",          // 必須
  "ticker": { "exchange": "HyperliquidLinear", "symbol": "BTC" },  // 構造体必須
  "side": "buy" | "sell",
  "qty": 0.1,
  "order_type": { "market": {} } | { "limit": { "price": 92500.0 } }  // 明示必須
}
```

**禁則**:
- `order_type` 省略 → 400（`/api/replay/order` の silent market デフォルトと異なる）。
- `ticker` が文字列 → 400。
- `client_order_id` 欠落 → 400。

### 4.5 エラーモデル

| Status | 条件 | Body |
|---|---|---|
| 400 | 型不整合（`ticker` 文字列、`order_type` 欠落、`client_order_id` 欠落） | `{ "error": "<具体的メッセージ>" }` |
| 400 | `advance` が GUI ランタイム（`--headless` なし） | `{ "error": "instant mode requires headless runtime (pass --headless)" }` |
| 400 | `client_order_id` が regex `^[A-Za-z0-9_-]{1,64}$` に違反 | `{ "error": "client_order_id must match [A-Za-z0-9_-]{1,64}" }` |
| 404 | セッション未初期化 | `{ "error": "session not started", "hint": "start a replay session first (see agent_replay_api.md Getting Started)" }` |
| 409 | `client_order_id` 重複だが既存とボディが異なる | `{ "error": "client_order_id conflict with different request body" }` |
| 501 | `session_id != "default"` | `{ "error": "multi-session not yet implemented; use 'default' until Phase 4c" }` |
| 503 | セッション loading 中 | `{ "error": "session loading" }` |

冪等 replay（同一 `client_order_id` + 同一ボディ）は `200` + `{ "order_id": ..., "idempotent_replay": true }`。

---

## 5. 既存実装との統合点

### 5.1 `VirtualExchange` 共有 とセッション起動経路

ADR-0001 の決定通り、UI リモコン API と agent API は **同一 `VirtualExchange`（"default" session）** を共有する。

- 新規ロック層を追加しない（既存 `Arc<Mutex<..>>` に委ねる）
- `/api/replay/play` でセッションが起動済みの場合のみ agent API が動作する（未起動なら 404）
- agent API 側に `session create` 系エンドポイントは **本計画で設けない**

#### 技術的負債としての認識

agent API が 404 メッセージでも内部的にもセッション起動を UI リモコン `/api/replay/play` に依存しており、これは将来 UI 側を facade 化する際の循環参照リスクを抱える。**選択肢**:

- **(a) 採用**: 4b-1 では UI リモコン `/play` への依存を明示的な妥協として受容する。404 レスポンスに `hint` フィールドで `docs/spec/agent_replay_api.md` の Getting Started を指す（endpoint 名を直接書かないことで UI リモコン API への hard link を避ける）。agent introspection としても機能する。
- (b) 不採用: agent API 側に `POST /api/agent/session/default/start` を先出し。**却下理由**: §2 スコープと矛盾し、start と `/play` の両経路が並存する新たな silent failure 温床になる。

**申し送り**: Phase 4c の facade 化時に `/api/replay/play` の内部実装を agent API 側の `start` に統合する（現在は agent API に `start` がないが、そのときは UI リモコン側の受け口を残したまま agent API 側を canonical にする）。この TODO は `docs/adr/0001-agent-replay-api-separation.md` の Consequences にも追記を検討。

### 5.2 FillEvent → step レスポンス経路

`update_outcome_from_fill`（[src/narrative/service.rs:442](../../src/narrative/service.rs#L442)）は既に `pub async fn` として定義済み。GUI/headless の 3 経路（[handlers.rs:253](../../src/app/handlers.rs#L253) / [dashboard.rs:394](../../src/app/dashboard.rs#L394) / [headless.rs:332,432](../../src/headless.rs#L332)）は `Task::perform` / `tokio::spawn` で fire-and-forget しているだけで、関数自体は await 可能。

本計画では:

- agent API `step` ハンドラが `on_tick` の戻り値（`DispatchResult` + `fills`）を**直接**受け取り、`update_outcome_from_fill(..).await` を **同期的に** 呼んだ上でレスポンスを組み立てる（新規 refactor 不要）。
- GUI 側の既存 fire-and-forget 経路は温存（UI は polling で narrative を拾えれば足りる）。
- 本 tick 中に更新された narrative の id を集めるため、`update_outcome_from_fill` の戻り値（`Result<usize, _>`）を使うのではなく、**呼び出し前に `(linked_order_id → narrative_id)` の逆引き**を実装する（小規模な追加 query が必要）。

### 5.3 `EpochMs` の変換境界

新 API 境界型 `EpochMs` は内部実装（`u64` / `chrono`）との変換が必要。サブフェーズ C / G で実際に消費される箇所を以下に明示し、dead code 化を防ぐ:

| 変換方向 | 消費箇所 | 消費サブフェーズ |
|---|---|:-:|
| `EpochMs → u64` | `StepClock::tick_until(u64 ms)` に `advance.until_ms.as_u64()` を渡す | G |
| `EpochMs → u64` | step レスポンス組み立て時 `clock.now_ms()` を `EpochMs::from(u64)` でラップ | C |
| `u64 → EpochMs` | `EventStore::trades_in(start, end)` の戻り値 `trade.time_ms` を `EpochMs` 化してシリアライズ | C |
| `EpochMs → i64` | narrative ストア (`timestamp_ms: i64`) との境界で `try_as_i64()` を使用。overflow 時 500 | D |

上記は §6 C/D/G の GREEN 実装時に参照。`try_as_i64()` は D で narrative outcome 更新時に実使用される（dead code ではない）。

### 5.4 Headless 判定（ランタイム）

`advance` は `cfg(feature = "headless")` ではなく、**起動時に `--headless` フラグが立っているか** で分岐（既存 [src/main.rs](../../src/main.rs) の分岐を踏襲）。同じビルドを GUI ランタイム・Headless ランタイムで切り替えられる構造なので、feature フラグよりランタイム判定が適切。DoD #7 / サブフェーズ G 参照。

実装上は `ApiCommand` を発行する前段（`src/replay_api.rs`）で `is_headless: bool` フラグを保持し、`advance` コマンドのみこれをチェックして 400 を返す。

---

## 6. 実装サブフェーズ（TDD）

DoD 番号は §1 の成功条件チェックリスト上から 1 始まりで対応:
1=step 同梱 / 2=advance / 3=order client_order_id / 4=Ticker 構造体 / 5=EpochMs / 6=501 / 7=advance GUI 400 / 8=UI リモコン互換 / 9=Python SDK / 10=E2E + 80% カバレッジ / 11=CI

| # | 内容 | RED 起点テスト | DoD 消化 |
|:-:|---|---|:-:|
| **A** | `EpochMs` / `TickerContract` / `ClientOrderId` newtype + serde + validation テスト | `src/api/contract/ticker.rs` / `epoch.rs` / `client_order_id.rs` 各モジュール内 `#[cfg(test)]` | 4, 5 |
| **B** | `POST .../step` ルーティング + `session_id != default` で 501 | `replay_api::tests::step_rejects_non_default_session` | 6 |
| **C** | step レスポンスに observation + fills を同梱（`EpochMs` 消費点）| `replay_api::tests::step_returns_fills_inline` | 1 (partial) |
| **D** | step レスポンスに `updated_narrative_ids` を同梱（同期 await） | `replay_api::tests::step_updates_narrative_synchronously` | 1 (final) |
| **E** | `POST .../order` + `client_order_id` idempotency（正規化キー比較） | `replay_api::tests::order_idempotent_replay` | 3 |
| **F** | `Ticker` 文字列拒否 / `order_type` 省略拒否 / `client_order_id` regex 違反 400 | `replay_api::tests::order_rejects_string_ticker` 他 | 4 |
| **G** | `POST .../advance` + headless ランタイムガード（`EpochMs::try_as_i64` 消費点）| `replay_api::tests::advance_rejects_gui_runtime` | 2, 7 |
| **H** | `advance` `stop_on: fill` での停止 + `include_fills` opt-in | `replay_api::tests::advance_stops_on_fill` | 2 |
| **I** | Python SDK `fs.agent_session.*`（`python/agent_session.py` 新規） | `tests/python/test_agent_session.py`（narrative と同パス規則） | 9 |
| **J** | E2E S55〜S59 + 既存 `tests/s*.sh` が全 PASS | `tests/e2e/s55_agent_session_step.py` 等 | 8, 10 |
| **K** | PR テンプレ更新 + UI リモコン API 新規ルート検知 lint | CI grep ベース（下記） | 11 |

各サブフェーズで `cargo fmt` → `cargo clippy -D warnings` → `cargo test` を通す。

#### K の具体コマンド

diff 行の正規表現で検知する方式は、既存の `match (method, path) { ("POST", "/api/replay/play") => ... }` という **match arm の tuple リテラル形式** と新規ルート追加の tuple リテラルが区別できないため採用しない（既存ルートの軽微な並べ替えでも誤検知する）。

代わりに **main とブランチの両方で `/api/replay/` ルートを列挙し、集合差分で判定** する方式を取る。

`.github/workflows/adr_guard.yml`（新規）:

```yaml
- name: UI remote API freeze check (ADR-0001)
  run: |
    set -euo pipefail
    git fetch origin main --depth=1

    # /api/replay/ で始まる path リテラルを列挙する関数。
    # src/replay_api.rs の match arm / route() 呼び出し双方を拾い、path 部分のみを抽出して sort -u。
    extract_routes() {
      local ref="$1"
      git show "$ref:src/replay_api.rs" \
        | grep -oE '"/api/replay/[A-Za-z0-9_/{}:-]*"' \
        | sort -u
    }

    extract_routes origin/main > /tmp/routes_main.txt
    extract_routes HEAD        > /tmp/routes_head.txt

    ADDED=$(comm -13 /tmp/routes_main.txt /tmp/routes_head.txt || true)
    if [ -n "$ADDED" ]; then
      echo "::error::New route(s) added to /api/replay/* — forbidden by ADR-0001:"
      echo "$ADDED"
      echo "Add new routes to /api/agent/session/:id/* instead."
      exit 1
    fi
```

**利点**:
- 集合差分なので行の並び替え・インデント変更では誤検知しない
- match arm と route() ヘルパの両方を同時に拾える
- 既存ルートの内部実装変更（handler の中身）は通過する

**副次的検証**: サブフェーズ K で「既存ルートの削除」も集合差分で検知できる（`comm -23` で main 側にしかない path を列挙）。削除は本計画スコープ外だが lint ログに出しておけば事故防止になる。

PR テンプレート（`.github/pull_request_template.md`）には以下を追記:

```markdown
- [ ] `/api/replay/*` への新規ルート追加なし（ADR-0001 により禁止 / 既存ルートの内部実装変更は可）
```

---

## 7. E2E テスト（S55 以降）

| S# | ファイル | 内容 |
|:-:|---|---|
| S55 | `s55_agent_session_step.py` | `step` が observation + fills + narrative_ids を 1 RTT で返す（4 TC） |
| S56 | `s56_agent_session_order.py` | `client_order_id` idempotency / 型厳格化 400 / 409 conflict（6 TC） |
| S57 | `s57_agent_session_advance.py` | headless で 1 ヶ月分 instant / GUI で 400 / `stop_on: fill`（4 TC） |
| S58 | `s58_agent_session_session_id.py` | `"default"` 以外で 501、空文字で 400、未起動で 404（3 TC） |
| S59 | `s59_concurrent_ui_and_agent.py` | UI リモコンと agent API の混在整合（下記 TC） |

#### S59 の TC 具体化

| TC | シナリオ | 期待値 |
|:-:|---|---|
| TC1 | UI `/api/replay/play` → agent `/step` を 1 回 → 直後に UI `GET /api/replay/state` と agent `GET /api/agent/session/default/observation` を取得 | 両者の `clock_ms` が厳密に一致。OHLCV 末尾バーの close が一致 |
| TC2 | UI `/api/replay/play` → agent `/order` で `client_order_id=cli_1` 発注 → UI `GET /api/replay/orders` | UI のリストに発注済みが現れ、`order_id` が agent レスポンスと一致 |
| TC3 | UI `/api/replay/play` → agent `/order` で発注 → UI `/api/replay/toggle`（Live 戻し）→ 直後に agent `/order` を叩き **404** を確認 → 再度 `/api/replay/play` → agent `/order` で **同一** `client_order_id=cli_1` 発注 | toggle 直後は 404（`Terminated` でセッション終了）。2 回目の play 後の発注は **新規 `order_id`** を返す（§3.3 `Started` イベントで map クリア検証） |
| TC4 | UI `/api/replay/step-backward` → agent `/order` で発注 | agent 側は受理（§3.3 `Reset` イベントで map クリア検証）|

---

## 8. リスクとオープン質問

| # | 内容 | 扱い |
|:-:|---|---|
| R1 | step 同期化で narrative ストア書き込みが遅い場合に step が遅延する | **計測手順**: サブフェーズ D の RED テスト中で `std::time::Instant::now()` を `step` ハンドラ前後に挿入し `cargo test -- --nocapture` で測定（narrative 100 件紐付け時・SQLite の fsync 有無両方）。**判定基準**: p95 で 100ms 超なら非同期化 + `updated_narrative_ids` を「発火予定 id」として返す案に切替。測定値は §10 実装ログに記録 |
| R2 | `advance` 中に UI から `/api/replay/pause` が入った場合の挙動 | **GUI では advance 不能**（400）なので問題は Headless のみ。Headless では UI リモコンは叩かれない想定で OK |
| R3 | `client_order_id` のメモリ内 UNIQUE がアプリ再起動で消える | 4a narrative と違い、order 重複は再起動後はそもそも起きない（session 自体がリセット）ので問題なし。明記するのみ |
| Q1 | `observation` の OHLCV 窓サイズのデフォルト | 既存 `/api/replay/state` と揃える。明示指定はクエリ `?window=N` で拡張可能にする |
| Q2 | `stop_on: "narrative"` の意味論 | narrative 作成 or outcome 更新のどちら？ → **どちらも**（agent 視点では区別不要）|
| Q3 | UI リモコン `/api/replay/step-forward` との違いを PR 説明でどう訴求するか | README / `agent_replay_api.md` で比較表を用意 |
| Q4 | `advance` で発火した fills の個別情報（order_id・price 等）を一部でも返すか | **現案**: `aggregate_fills: count` のみ。**代替**: `stop_on: ["fill"]` を含む場合のみ最終 1 件の FillEvent 詳細を同梱。巨大区間で `stop_on` なし実行時はやはり件数のみ。実装時に計測してから決定 |

---

## 9. ドキュメント生成物

本計画完了時に以下を作成・更新する:

- **新規**: `docs/spec/agent_replay_api.md` — API リファレンス仕様書（narrative.md と同粒度）
- **更新**: `docs/spec/replay.md` §11 に「agent 契約は `agent_replay_api.md` 参照」の注記を追加
- **更新**: `docs/wiki/replay.md`（[既存](../wiki/replay.md) 確認済み）に「agent から操作する場合は別 API あり」注記
- **更新**: `.github/pull_request_template.md` に UI リモコン API 拡張禁止条項
- **更新**: `docs/plan/🔄ai_agent_platform_roadmap.md` Phase 4b を 4b-1 / 4b-2 に分割

---

## 10. 実装ログ（作業者追記）

### サブフェーズ E レビュー反映（2026-04-22）

- **HTTP ステータス統一**: 新規注文 201 → 200 に変更。冪等リプレイと同じ 200 に揃え、Python SDK 側は `idempotent_replay` フラグだけで分岐できるようにした。
- **503 Loading ケース**: `place_order_returns_503_when_session_loading` を追加して 404/503 対称のカバレッジを担保。
- **`observe_generation` の呼び出し箇所整理**: `agent_session_step` ハンドラ入口で 1 回だけ呼ぶ方式に変更。fill 逆引きパスからは削除（単一 step 内で世代が変化することはないため、冗長かつコメントが誤解を招いていた）。

### サブフェーズ E 後送り TODO（次サブフェーズまたは Phase 4c で対応）

| # | 項目 | 起案 | 対応時期 |
|---|---|---|---|
| T1 | **GUI 側の `mark_session_*` 発火が未配線**。現状 `src/headless.rs` のみ。Phase 4b-1 では GUI 側 agent command は 501 スタブなので実害なし。GUI で agent API を有効化した瞬間に TC3/TC4 相当の lifecycle 伝播が抜ける潜在バグ | E レビュー #2 | Phase 4c（facade 化と同時） |
| T2 | **`AgentSessionState::client_order_id_for` が O(n) 線形探索**。1 tick に fill が多数来る高頻度取引シナリオでは `HashMap<order_id, ClientOrderId>` を並走させる。§8 R1 に準じ計測して切替判定 | E レビュー #4 | 計測 WARN 発火後 |
| T3 | **Conflict 分岐で事前採番 UUID が捨てられる**。`place_or_replay` の引数を `impl FnOnce() -> String` に変更すれば遅延生成可能。コスト低なので後続 refactor | E レビュー #5 | 任意（改善余地） |
| T4 | **`SessionLifecycleEvent` enum は現在ログ出力のみの消費**。`#[derive(Clone, PartialEq, Eq)]` は Phase 4c で broadcast channel 化する際の足場として残置 — 削除しないこと | E レビュー #6 | Phase 4c で channel 化 |
| T5 | **`VirtualOrder.ticker` が `TickerContract.symbol` のみ保持し `exchange` を落とす**。現行 `VirtualOrder.ticker: String` が symbol 単体を期待する Phase 2 仕様のため。複数 exchange を扱う Phase 4b-2 で意味論が壊れる可能性 | E レビュー #7 | Phase 4b-2 着手時（`VirtualOrder` 型拡張） |

### R1 性能計測（プラン §8）

サブフェーズ D で仕込んだ `log::debug!("agent_session_step: total {total_ms}ms (narrative sync {narrative_elapsed_ms}ms)")` の実測はまだ取得していない。E2E（サブフェーズ J）で 1 ヶ月分連続 step を走らせたときの p95 を記録する予定。
