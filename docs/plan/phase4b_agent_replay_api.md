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

| トリガ | 挙動 |
|---|---|
| `POST /api/replay/play`（UI リモコン経由のセッション起動） | map 全クリア（新セッション開始） |
| `POST /api/replay/toggle`（Live ↔ Replay 切替） | map 全クリア |
| seek / step-backward（UI リモコン経由） | map 全クリア（巻き戻した tick の同一 `client_order_id` で別注文を発注したい要求に応えるため） |
| 通常 tick 進行 | 保持 |

重複時は **リクエストボディが完全一致なら** 既存 `order_id` を返しつつ `idempotent_replay: true`、**差異があれば** `409 Conflict`（§4.5 参照）。

---

## 4. API シェイプ

### 4.1 エンドポイント一覧

| Method | Path | 用途 | ボディ |
|---|---|---|---|
| `POST` | `/api/agent/session/:id/step` | 1 bar 進行 + 副作用同梱 | `{}` または省略可 |
| `POST` | `/api/agent/session/:id/advance` | 任意区間 instant 実行（Headless のみ） | `{ until_ms, stop_on?: ["fill","narrative","end"] }` |
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
  "aggregate_fills": 12,                // 件数のみ（配列は含まない）
  "aggregate_updated_narratives": 8,
  "final_portfolio": { ... }            // 終端時点のみ
}
```

**不変条件**:
- `observation` は返さない（数万 tick 分の OHLCV 窓は巨大）。呼び手が必要なら直後に `GET .../observation` を叩く。
- `stop_on` が空配列なら `until_ms` 到達まで走る。`["fill"]` を含めれば最初の FillEvent で停止し `stopped_reason: "fill"` を返す。
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
| 404 | セッション未初期化 | `{ "error": "session not started" }`（セッション起動手順は §5.1 参照） |
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

- **(a) 採用**: 4b-1 では UI リモコン `/play` への依存を明示的な妥協として受容する。404 のレスポンス本文には起動手順を含めない（循環言及を避ける）。代わりに `docs/spec/agent_replay_api.md` の「Getting Started」で `/api/replay/play` をまず叩く手順を書く。
- (b) 不採用: agent API 側に `POST /api/agent/session/default/start` を先出し。**却下理由**: §2 スコープと矛盾し、start と `/play` の両経路が並存する新たな silent failure 温床になる。

**申し送り**: Phase 4c の facade 化時に `/api/replay/play` の内部実装を agent API 側の `start` に統合する（現在は agent API に `start` がないが、そのときは UI リモコン側の受け口を残したまま agent API 側を canonical にする）。この TODO は `docs/adr/0001-agent-replay-api-separation.md` の Consequences にも追記を検討。

### 5.2 FillEvent → step レスポンス経路

`update_outcome_from_fill`（[src/narrative/service.rs:442](../../src/narrative/service.rs#L442)）は既に `pub async fn` として定義済み。GUI/headless の 3 経路（[handlers.rs:253](../../src/app/handlers.rs#L253) / [dashboard.rs:394](../../src/app/dashboard.rs#L394) / [headless.rs:332,432](../../src/headless.rs#L332)）は `Task::perform` / `tokio::spawn` で fire-and-forget しているだけで、関数自体は await 可能。

本計画では:

- agent API `step` ハンドラが `on_tick` の戻り値（`DispatchResult` + `fills`）を**直接**受け取り、`update_outcome_from_fill(..).await` を **同期的に** 呼んだ上でレスポンスを組み立てる（新規 refactor 不要）。
- GUI 側の既存 fire-and-forget 経路は温存（UI は polling で narrative を拾えれば足りる）。
- 本 tick 中に更新された narrative の id を集めるため、`update_outcome_from_fill` の戻り値（`Result<usize, _>`）を使うのではなく、**呼び出し前に `(linked_order_id → narrative_id)` の逆引き**を実装する（小規模な追加 query が必要）。

### 5.3 Headless 判定（ランタイム）

`advance` は `cfg(feature = "headless")` ではなく、**起動時に `--headless` フラグが立っているか** で分岐（既存 [src/main.rs](../../src/main.rs) の分岐を踏襲）。同じビルドを GUI ランタイム・Headless ランタイムで切り替えられる構造なので、feature フラグよりランタイム判定が適切。

実装上は `ApiCommand` を発行する前段（`src/replay_api.rs`）で `is_headless: bool` フラグを保持し、`advance` コマンドのみこれをチェックして 400 を返す。

---

## 6. 実装サブフェーズ（TDD）

| # | 内容 | RED 起点テスト |
|:-:|---|---|
| **A** | `EpochMs` / `TickerContract` / `ClientOrderId` newtype + serde テスト | `src/types/ticker_contract.rs` serde roundtrip |
| **B** | `POST .../step` ルーティング + `session_id != default` で 501 | `replay_api::tests::step_rejects_non_default_session` |
| **C** | step レスポンスに observation + fills を同梱 | `replay_api::tests::step_returns_fills_inline` |
| **D** | step レスポンスに `updated_narrative_ids` を同梱（同期 await） | `replay_api::tests::step_updates_narrative_synchronously` |
| **E** | `POST .../order` + `client_order_id` idempotency | `replay_api::tests::order_idempotent_replay` |
| **F** | `Ticker` 文字列拒否 / `order_type` 省略拒否 | `replay_api::tests::order_rejects_string_ticker` |
| **G** | `POST .../advance` + headless ガード | `replay_api::tests::advance_rejects_gui_build` |
| **H** | `advance` `stop_on: fill` での停止 | `replay_api::tests::advance_stops_on_fill` |
| **I** | Python SDK `fs.agent_session.*`（`python/agent_session.py` 新規） | `tests/python/test_agent_session.py`（narrative と同パス規則） |
| **J** | E2E S55〜S59 | `tests/e2e/s55_agent_session_step.py` 等（既存 S51-S53 と同ディレクトリ）|
| **K** | PR テンプレ更新 + UI リモコン API 新規ルート検知 lint | CI grep ベース（下記） |

各サブフェーズで `cargo fmt` → `cargo clippy -D warnings` → `cargo test` を通す。

#### K の具体コマンド

`.github/workflows/adr_guard.yml`（新規）に以下のジョブを追加:

```yaml
- name: UI remote API freeze check (ADR-0001)
  run: |
    # main との diff で /api/replay/* への新規ルート追加を検知。
    # 検知ロジック: src/replay_api.rs の routing table に新規 POST/GET/PATCH が追加されていたら fail。
    git fetch origin main
    ADDED=$(git diff origin/main -- src/replay_api.rs | \
            grep -E '^\+.*"(POST|GET|PATCH|PUT|DELETE)\s+/api/replay/' || true)
    if [ -n "$ADDED" ]; then
      echo "::error::New route(s) added to /api/replay/* — forbidden by ADR-0001."
      echo "$ADDED"
      echo "Add new routes to /api/agent/session/:id/* instead."
      exit 1
    fi
```

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
| TC3 | UI `/api/replay/play` → agent `/order` で発注 → UI `/api/replay/toggle`（Live 戻し）→ 再度 `/api/replay/play` → agent `/order` で **同一** `client_order_id=cli_1` 発注 | 2 回目の発注は **新規 `order_id`** を返す（§3.3 の「play で map クリア」検証） |
| TC4 | UI `/api/replay/step-backward` → agent `/order` で発注 | agent 側は受理（§3.3「seek で map クリア」検証）|

---

## 8. リスクとオープン質問

| # | 内容 | 扱い |
|:-:|---|---|
| R1 | step 同期化で narrative ストア書き込みが遅い場合に step が遅延する | まず実装して計測。100ms 超なら非同期化 + `updated_narrative_ids` を「発火予定」として返す案に切替 |
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

_（サブフェーズ着手時に追記）_