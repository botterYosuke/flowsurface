# ADR-0001: Agent 専用 Replay API を UI リモコン API から分離する

**Date**: 2026-04-22
**Status**: proposed
**Deciders**: sasaicco@gmail.com
**Reviewers**: Claude（設計レビュー・下書き補助）

## Context

flowsurface の `/api/replay/*` HTTP API は当初 E2E テスト用の「UI リモコン」として設計され、人間が GUI で行う操作（F5 トグル、▶ ボタン、`YYYY-MM-DD HH:MM` 入力）をそのまま HTTP に写したものである。Phase 4a でこの API を Python SDK / uAgent が直接叩いたところ、短期間に 3 件の silent failure が発生した（`narrative.md §13.2`）:

1. `step_forward` が FillEvent を捨てて outcome が永遠 null
2. 512 KB 超のボディを握り潰し（narrative で踏んだがリプレイ API 共通）
3. `POST /api/replay/order` が `Exchange:Symbol` 形式をサイレント受理し全注文 Pending

原因は共通して「UI 前提の仕様が agent の要求する決定論・アトミック性・型契約を満たしていない」こと。Wall-clock 駆動の `StepClock::tick(wall_now)`、`GET /state` と `POST /order` の非アトミック性、`ticker: String` の暗黙正規化、`session_id` 不在による複数 agent 同時実行不能、などが構造的に噛み合わない。Phase 4b で ASI Alliance / uAgent 統合に進むには、この API を agent 契約として再設計する必要がある。

## Decision

Agent 向け操作は `/api/agent/session/:id/*` に分離した **新 API** を canonical として設計し、既存 `/api/replay/*`（UI リモコン API）は段階的に新 API の薄い facade へ移行する。以下を不変条件として確定する:

- **エンドポイント分離**: `step`（1 bar + observation 返却、決定論）と `advance { until_ms, stop_on }`（instant モード、observation 非返却）を別エンドポイントにする。bool フラグによる動作分岐は採用しない。
- **副作用同梱**: `step` レスポンスは当該 tick の `fills` / `updated_narrative_ids` / `clock_ms` / `observation` を同梱し、agent 側の polling を不要にする。
- **型契約の厳格化**: `Ticker = { exchange, symbol }` を構造体 JSON で受理（文字列結合は拒否）。`EpochMs(u64)` newtype を API 境界に導入。
- **冪等性**: `POST .../order` は `client_order_id` 必須、`(session_id, client_order_id)` で重複発注を抑止。
- **セッション**: Phase 4b では `session_id` を path に刻むがサーバー側は `"default"` 固定のみ受理。非 `"default"` は `501 Not Implemented` で明示拒否（`400` や黙殺にしない）。複数 agent 同時実行は Phase 4c。
- **基盤の統合（排他ではなく共有）**: Phase 4b の `session_id = "default"` 期間中、agent API と UI リモコン API は **同一 `VirtualExchange` インスタンス（"default" session）を共有** する。片方をロックする排他ではなく、両方が同じ session への窓口として並存する。排他制御が必要になるのは Phase 4c（複数 session 並行）から。
- **実行環境制約**: `advance`（instant モード）は Headless ビルドのみ許可。GUI ビルドでは 400 で拒否し、iced 再描画との競合を設計で排除。
- **スコープ境界（step-backward）**: agent API は **forward-only**（`step-backward` を含めない）。理由は RL の `env.step()` 慣習に合わせるため。`step-backward` の巻き戻し意味論（PnL / outcome / narrative の遡及）は本 ADR のスコープ外とし、UI リモコン API 側の既存挙動は現状維持する。

## Alternatives Considered

### Alternative 1: 既存 `/api/replay/*` を拡張する（最小変更）
- **Pros**: エンドポイント数が増えない。既存 E2E テストが無改修で動く。
- **Cons**: UI 前提の仕様（文字列日時・単一グローバル状態・bool フラグ肥大化）の上に agent 要件を積むため、silent failure の温床が残る。型契約を後付けで厳格化すると後方互換が崩れる。
- **Why not**: `narrative.md §13.2` の silent failure 3 件はすべてこの「後付け拡張」パターンで生まれた。同じ轍を踏む。

### Alternative 2: UI リモコン API を完全置き換え（UI からも新 API を直接叩く）
- **Pros**: 入口が 1 本化され、silent failure リスクが最小。
- **Cons**: GUI 側の `Message::Replay(..)` ディスパッチ経路を同時に書き換える必要があり、Phase 4b スコープが破裂する。既存 E2E テスト全面書き換え。
- **Why not**: ビッグバン移行はリスクが高い。段階的 facade 化（採用案）の方が刻める。

### Alternative 3: 分離 + facade（採用）
- **Pros**: agent 契約を型厳格に設計できる。UI リモコン API は既存クライアント（E2E スクリプト）互換のまま、内部実装だけ新 API に寄せられる。移行コストを Phase 4b / 4c に分割できる。
- **Cons**: 一時的に 2 系統の入口が存在する期間が発生する。facade 化までの規律（新規機能は新 API のみに追加）が必要。
- **Why not**: — 採用。

## Consequences

### Positive
- Agent 側の型契約が静的に固定され、`ticker` 文字列結合・`order_type` 省略などの silent failure カテゴリが構造的に発生不能になる。
- `step` レスポンス同梱により Python SDK / uAgent が polling ループを書かずに済み、決定論的バックテストが可能になる。
- `advance` 分離により 6 ヶ月分の instant 実行が wall-time 非依存で現実的な時間で完了する（Phase 3 Headless モードの実益が出る）。
- `session_id` を path に刻む契約が先行確定するため、Phase 4c の複数 agent 同時実行が後方互換で拡張できる。
- UI リモコン API と agent API が同一 `VirtualExchange` を共有するため、既存 E2E テスト（`tests/s*.sh`）は Phase 4b 期間中も無改修で動作する。

### Negative
- API 表面積が一時的に 2 倍になる。ドキュメント（`replay.md` / 新設 `agent_replay_api.md`）の二重管理が発生。
- `EpochMs` newtype 導入で `exchange/` 配下の `u64 ms` / `chrono::DateTime` 境界に変換層が必要（ただしこれは単体の利益でもある）。
- Headless 専用制約（`advance`）のため、E2E テストの一部は `IS_HEADLESS=true` 必須になる。
- **`session_id = "default"` 固定制約のドキュメント負債**: path に `:id` が刻まれるが Phase 4b では `"default"` しか受理されない。`agent_replay_api.md` 冒頭および OpenAPI スキーマの path パラメータ description に「Phase 4b では必ず `default` を指定すること。非 `default` は 501」を明記する必要がある。開発者の混乱を防ぐ DX 負債。

### Risks
- **2 系統 API の同一 `VirtualExchange` への同時アクセスによる状態競合**: agent が `step` 実行中に UI リモコン経由で `play` が入る等の競合。
  - **Mitigation**: Phase 4b では `VirtualExchange` 側の既存ロック（`Arc<Mutex<..>>`）に委ねる。新たな排他層は追加しない。E2E で両入口を同時に叩くテスト（S60 系）を追加して回帰検知。
- **facade 化の先送りリスク**: Phase 4c で facade 化されずに 2 系統が定着する。
  - **Mitigation**: `.github/pull_request_template.md` に「`/api/replay/*` への新規機能追加は本 ADR で禁止。新規は `/api/agent/session/*` のみ」を明記。CI に `src/replay_api.rs` の特定範囲（UI リモコンルーティング table）に新規ルートが追加されたら警告する lint チェック（grep ベース）を追加する（Phase 4b 末に設置）。
- **`session_id = "default"` 固定の誤用**: 複数 session 対応時に path はあるがサーバーが受理しない期間に agent が誤解する。
  - **Mitigation**: 非 `"default"` は `501 Not Implemented` + `{"error": "multi-session not yet implemented; use 'default' until Phase 4c"}` のレスポンス本文で明示拒否する。`400` や黙殺にしない。
