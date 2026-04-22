# ADR-0001: Agent 専用 Replay API の新設と自動再生機構の廃止

**Date**: 2026-04-22
**Status**: proposed
**Deciders**: sasaicco@gmail.com
**Reviewers**: Claude（設計レビュー・下書き補助）

> **経緯**: 本 ADR は 2026-04-22 に「Agent 専用 Replay API を UI リモコン API から分離する」という初稿で起票された。実装着手前のレビューで、ADR-0001 の一部条項（GUI で `advance` を 400 拒否する制約、`step-backward` をスコープ外とする宣言）が自動再生機構との競合を前提にした消極的設計であると判明した。自動再生機構そのものを廃止すれば競合原因が消滅し、設計が大幅に単純化できる。したがって同日内に「自動再生機構の廃止」方針を本 ADR に統合する形で改訂した。完全な履歴は git log を参照。

## Context

flowsurface の `/api/replay/*` HTTP API は当初 E2E テスト用の「UI リモコン」として設計され、人間が GUI で行う操作（F5 トグル、▶ ボタン、`YYYY-MM-DD HH:MM` 入力）をそのまま HTTP に写したものである。Phase 4a でこの API を Python SDK / uAgent が直接叩いたところ、短期間に 3 件の silent failure が発生した（`narrative.md §13.2`）:

1. `step_forward` が FillEvent を捨てて outcome が永遠 null
2. 512 KB 超のボディを握り潰し（narrative で踏んだがリプレイ API 共通）
3. `POST /api/replay/order` が `Exchange:Symbol` 形式をサイレント受理し全注文 Pending

原因は共通して「UI 前提の仕様が agent の要求する決定論・アトミック性・型契約を満たしていない」こと。Wall-clock 駆動の `StepClock::tick(wall_now)`、`GET /state` と `POST /order` の非アトミック性、`ticker: String` の暗黙正規化、`session_id` 不在による複数 agent 同時実行不能、などが構造的に噛み合わない。

さらに以下が判明した:

- **wall-clock 駆動の自動 tick ループ**（iced subscription + tokio timer）は agent の決定論要求と構造的に衝突する。自動 tick を残したまま agent 操作を許すと race が避けられず、`advance` を Headless 限定にせざるを得ない消極的設計に追い込まれる。
- 実運用で **ユーザーは自動再生を使っていない**（step 単位の精査 or agent 駆動が主）。pause / resume / speed / step-backward(1step) は UI 上の死機能。
- セッション初期化の入口が `/api/replay/play` 1 本に固定されていると、agent が session を扱うのに UI リモコン API を叩く必要があり、API 境界の責務が混線する。

Phase 4b で ASI Alliance / uAgent 統合に進むには、この API を agent 契約として再設計し、**同時に自動再生機構を廃止してセッション内時刻操作を agent session API に一本化する** 必要がある。

## Decision

### §1. Agent 専用 API の新設（`/api/agent/session/:id/*`）

Agent 向け操作は `/api/agent/session/:id/*` に分離した **新 API** を canonical として設計し、既存 `/api/replay/*`（UI リモコン API）は session のモード切替と status 取得のみに責務を縮小する。以下を不変条件として確定する。

- **エンドポイント分離**: `step`（1 bar + observation 返却、決定論）と `advance { until_ms }`（instant モード、observation 非返却）を別エンドポイントにする。bool フラグによる動作分岐は採用しない。
- **副作用同梱**: `step` レスポンスは当該 tick の `fills` / `updated_narrative_ids` / `clock_ms` / `observation` を同梱し、agent 側の polling を不要にする。
- **型契約の厳格化**: `Ticker = { exchange, symbol }` を構造体 JSON で受理（文字列結合は拒否）。`EpochMs(u64)` newtype を API 境界に導入。
- **冪等性**: `POST .../order` は `client_order_id` 必須、`(session_id, client_order_id)` で重複発注を抑止。
- **セッション**: Phase 4b では `session_id` を path に刻むがサーバー側は `"default"` 固定のみ受理。非 `"default"` は `501 Not Implemented` で明示拒否（`400` や黙殺にしない）。複数 agent 同時実行は Phase 4c。
- **基盤の統合（排他ではなく共有）**: Phase 4b の `session_id = "default"` 期間中、agent API と UI リモコン API は **同一 `VirtualExchange` インスタンス（"default" session）を共有** する。
- **セッション状態遷移はイベント経由**: `VirtualExchange` がセッション状態遷移を `SessionLifecycleEvent`（`Started` / `Reset` / `Terminated`）として発火し、agent API 側の state（`client_order_id` UNIQUE map 等）はこれを購読してリセットする。UI リモコン API ハンドラが agent API の state を直接触る構造は禁止する。

### §2. 自動再生機構の全廃

- iced subscription / tokio timer による wall-clock 駆動の自動 tick ループを削除
- `StepClock` から `speed` / `pause` / `resume` / 状態機械を撤去。`now_ms()` と `tick_until()` のみに縮退
- `ReplayCommand::{Pause, Resume, StepForward, CycleSpeed, StepBackward, Play}` variant を削除
- `ReplayController::is_paused()` / `ReplayState::is_paused()` を削除
- 「playing / paused」状態の概念そのものを廃止（セッションは `Idle` / `Loading` / `Active` の 3 状態のみ）

### §3. HTTP API ルート確定表

| ルート | 状態 |
|---|---|
| `POST /api/replay/play` | **削除**。初期化は `toggle` / `rewind-to-start` の冒頭で遅延実行 |
| `POST /api/replay/pause` | **削除** |
| `POST /api/replay/resume` | **削除** |
| `POST /api/replay/speed` | **削除** |
| `POST /api/replay/step-forward` | **削除**（UI は agent step を発行） |
| `POST /api/replay/step-backward` | **削除**（意味変更なし、単純撤去） |
| `POST /api/replay/toggle` | **維持**。Live→Replay 遷移時、body `{start, end}` で session を初期化（klines ロード + Active 遷移）。auto-tick は発火しない |
| `GET /api/replay/status` | 維持 |
| `POST /api/app/set-mode` | 維持 |
| `POST /api/agent/session/:id/step` | **新設**。1 bar 進行。未初期化時は 400 |
| `POST /api/agent/session/:id/advance` | **新設**。`until_ms` まで進行。GUI / Headless 両方で受理。未初期化時は 400。`until_ms` 必須 |
| `POST /api/agent/session/:id/rewind-to-start` | **新設**。詳細は §4、状態別受理は §6 マトリクス |
| `POST /api/agent/session/:id/order` | **新設**。`client_order_id` 必須 |

### §4. `rewind-to-start` のセマンティクスと不変条件

**意図的な二重責務**: agent の利便性のため、以下を明示的に兼ねる。

- **初期化済みの場合**: 現在の session を保持したまま clock を `range.start` に巻き戻す。**klines は再ロードしない**。body `{start, end}` が付いていても **無視** する（Active 中の range 変更は silent behavior change の温床になるため受理しない。range を変えたい場合は一度 `toggle(Replay→Live)` で session を破棄してから `toggle(Live→Replay)` で再初期化する）。
- **未初期化の場合**: body `{start, end}` で session を新規初期化（`toggle(Live→Replay)` の初期化パスを内部的に再利用）。body なしは 400。

**リセット不変条件** — initialized → rewind の場合、以下を実施する。

| 対象 | 動作 |
|---|---|
| `StepClock.now_ms` | `range.start` にセット |
| `EventStore` cursor | 先頭へ巻き戻し（データ再投入はしない） |
| `VirtualExchange` の open orders | **全件キャンセル**（`SessionLifecycleEvent::Reset` 経由） |
| `VirtualExchange` の fills 履歴 | **全件破棄** |
| 仮想残高 / position | 初期値にリセット |
| `NarrativeState` | `Reset` イベントで購読側を初期化 |
| `client_order_id` UNIQUE map | クリア |
| UI 側のチャート | 新 session 扱いで再描画 |

`rewind-to-start` は agent にとって `env.reset()` 相当の操作として扱う。backend 側は既存の `SessionLifecycleEvent::Reset` を発火し、購読者（agent state / narrative / UI）に伝播する。

### §5. `advance` の `until_ms` 上限ポリシー

`advance` 呼び出しには以下の不変条件を課す。

- **`until_ms` は必須**。省略不可。`until_ms` 省略時に `range.end` まで暗黙進行する挙動は **禁止** する（silent failure の温床、および 1 クリック UI フリーズ回避）。
- **有限値であること**。`u64::MAX` 等の実質無限値は 400 で拒否。
- **上限制約**: UI から発火される `advance` は `until_ms <= min(range.end, now_ms + UI_ADVANCE_CAP_MS)` を満たすこと（UI_ADVANCE_CAP_MS はサブフェーズ P で固定。初期値は `3_600_000` = 1 時間を候補とし、UX プロトタイプで調整）。HTTP 経由で直接叩く agent 利用では `UI_ADVANCE_CAP_MS` は適用せず、`range.end` のみが上限。
- UI 具体仕様（モーダル vs 固定ステップ vs スライダー）は実装計画のサブフェーズ P で確定する。ただし上記不変条件はどの UI 形態でも構造的に守ること。

### §6. 状態 × コマンド受理マトリクス

| コマンド | Idle | Loading | Active |
|---|---|---|---|
| `POST /api/replay/toggle` (Live→Replay, body あり) | **Loading へ遷移（初期化）** | 409 Conflict (`loading in progress`) | 既に Replay の場合は 400 (`already in replay mode`) |
| `POST /api/replay/toggle` (Replay→Live) | n/a | 409 Conflict | **Idle へ遷移（session 破棄）** |
| `POST /api/agent/session/:id/step` | 400 (`session not initialized`) | 409 Conflict (`loading`) | **1 bar 進行** |
| `POST /api/agent/session/:id/advance` | 400 | 409 Conflict | **進行** |
| `POST /api/agent/session/:id/rewind-to-start` (body あり) | **Loading へ遷移（初期化）** | 409 Conflict | **Reset 発火 + clock 巻き戻し** |
| `POST /api/agent/session/:id/rewind-to-start` (body なし) | 400 (`body required for init`) | 409 Conflict | **Reset 発火 + clock 巻き戻し** |
| `GET /api/replay/status` | 200 (Idle 状態を返す) | 200 | 200 |

**原則**:
- **Loading 中は状態遷移系コマンドを全て 409 で拒否**。キューイングしない（非決定性の温床になるため）。
- **未初期化での step/advance は 400**（黙殺せず、agent に明示エラー）。
- **`rewind-to-start` のみ body の有無で初期化を兼ねる**（意図的なオーバーロード）。

### §7. UI ボタン配線

| ボタン | 旧挙動 | 新挙動 |
|---|---|---|
| `▶` | play / pause トグル | `POST /api/agent/session/default/step`（1 bar 進行） |
| `⏸` | pause | **撤去** |
| `⏭` | step-forward（1 bar 進む） | `POST /api/agent/session/default/advance`（§5 の上限ポリシー適用） |
| `⏮` | step-backward（1 bar 戻る） | `POST /api/agent/session/default/rewind-to-start`（先頭戻し） |
| 速度切替 | cycle speed | **撤去** |

### §8. 起動時 fixture 自動 Play の廃止

起動時 fixture 復元で自動的に Play を発火する経路（`pending_auto_play`）を廃止。Replay モードで起動した場合、session を初期化した Active 状態（klines ロード済み・clock は `range.start`）で静止して step を待つ。ユーザー / E2E は明示的に step / advance / rewind-to-start を叩いて進行させる。

## Alternatives Considered

### Alternative 1: 既存 `/api/replay/*` を拡張する（最小変更）
- **Pros**: エンドポイント数が増えない。既存 E2E テストが無改修で動く。
- **Cons**: UI 前提の仕様（文字列日時・単一グローバル状態・bool フラグ肥大化）の上に agent 要件を積むため、silent failure の温床が残る。
- **Why not**: `narrative.md §13.2` の silent failure 3 件はすべてこの「後付け拡張」パターンで生まれた。同じ轍を踏む。

### Alternative 2: 自動再生を残し speed 可変機能のみ維持
- **Pros**: 「UI でリプレイを眺める」ユースケースに 1 機能残せる。既存 E2E（s9 等）の書き換えが不要。
- **Cons**: agent との競合リスクが残り、`advance` の GUI ガードを撤回できない。StepClock の状態機械を温存する分、コードベースの複雑度が下がらない。
- **Why not**: 実運用で使われていない機能のために構造的負債を残す合理性がない。

### Alternative 3: `/api/replay/play` を残し「セッション開始」だけ担わせる（auto-tick なし）
- **Pros**: 既存 E2E テストの初期化パスが無改修。
- **Cons**: セッション初期化の入口が UI リモコン API と agent session API に分散し、責務境界が混線する。
- **Why not**: 「初期化トリガーを 1 本化したい」という改修目的に反する。

### Alternative 4: `rewind-to-start` を純粋な巻き戻しに限定し、`start` エンドポイントを別設
- **Pros**: 責務が単一。
- **Cons**: エントリポイントが増え、agent 側で「session が初期化済みか」判定が必須になる。`env.reset()` 慣習と噛み合わない。
- **Why not**: 利便性を優先し意図的にオーバーロード（§4 で不変条件を明記して曖昧さを回避）。

### Alternative 5: `advance` の `until_ms` 省略時に `range.end` まで進行
- **Pros**: UI の「末尾まで再生」ワンクリックが簡単に作れる。
- **Cons**: 1 クリックで UI フリーズを招く。省略可な引数は silent failure の温床。
- **Why not**: §5 で明示必須と確定。

### Alternative 6: 分離 + 自動再生廃止（採用）
- **Pros**: agent 契約を型厳格に設計し、かつ UI / agent が同じ進行コマンドを共有する。`advance` の GUI ガードが不要になり、StepClock の状態機械が消滅。session 操作入口が agent session API に一本化される。
- **Cons**: 破壊的変更。既存 E2E 大量書き換え。`step-backward`（1 step 戻り）の機能退行。
- **Why not**: — 採用。

## Consequences

### Positive

- Agent 側の型契約が静的に固定され、`ticker` 文字列結合・`order_type` 省略などの silent failure カテゴリが構造的に発生不能になる。
- `step` レスポンス同梱により Python SDK / uAgent が polling ループを書かずに済み、決定論的バックテストが可能になる。
- `advance` が GUI / Headless を問わず同一動作になり、UI と agent の差分が「初期化の発火源」だけになる。
- StepClock の状態機械と自動 tick 購読経路が消え、replay 周りのコード量と複雑度が有意に減る。
- セッション操作の入口が agent session API に一本化され、Phase 4c の複数 session 並行化時に UI リモコン API を触らずに拡張できる。
- `rewind-to-start` が `env.reset()` 相当として機能し、RL / agent 駆動バックテストのループが自然に書ける。
- 実運用で使われない機能（pause/resume/speed/step-backward 1step）のためのテスト・ドキュメント・E2E が消え、保守コストが下がる。

### Negative

- **破壊的変更（Breaking Change）**: `/api/replay/{play, pause, resume, speed, step-forward, step-backward}` を叩いている外部スクリプトは動かなくなる。flowsurface は単体デスクトップアプリで外部利用者は未想定、内部 E2E のみ。
- **明確な機能退行: `step-backward`（1 step 戻る）の廃止**
  - 過去にコストを投じて実装・テスト・品質計測した機能（`s13_step_backward_quality.py`、`tests/e2e/archive/s9_speed_step.sh`、`tests/e2e/archive/x2_buttons.sh` 等）を削除する。
  - 「1 bar 戻って再検証する」操作は `rewind-to-start` + step 繰り返しで代替可能だが、**直前 bar だけ戻りたい場合のコストが O(n) に増大**する（長時間 replay で顕著）。これを受容する。
  - 将来的に checkpoint + restore（特定 clock_ms へジャンプ）を agent session API に追加する余地は残すが、本 ADR のスコープ外。
- **pause/resume/speed 可視機能の喪失**: 「リプレイを流し見する」利用が不可能になる。視覚的な動作観察をしたいユーザーは `⏭` を連打するか外部スクリプトで step を連続発火させる必要がある。
- `EpochMs` newtype 導入で `exchange/` 配下の `u64 ms` / `chrono::DateTime` 境界に変換層が必要。
- Python SDK `fs.replay.*` のメソッド群が縮小し、`fs.agent_session.rewind_to_start` 等が追加される（SDK ユーザー視点での破壊的変更）。
- **`session_id = "default"` 固定制約のドキュメント負債**: path に `:id` が刻まれるが Phase 4b では `"default"` しか受理されない。`agent_replay_api.md` 冒頭および OpenAPI スキーマの path パラメータ description に「Phase 4b では必ず `default` を指定すること。非 `default` は 501」を明記する必要がある。

### Risks

- **2 系統 API の同一 `VirtualExchange` への同時アクセスによる状態競合**: agent が `step` 実行中に UI リモコン経由で `toggle` が入る等の競合。
  - **Mitigation**: `VirtualExchange` 側の既存ロック（`Arc<Mutex<..>>`）に委ねる。セッション状態遷移は `SessionLifecycleEvent` で agent state に伝播するため、UI 経由の `toggle` も agent 側から観測可能。E2E で両入口を同時に叩くテストを追加して回帰検知。
- **`rewind-to-start` での VirtualExchange リセット範囲の誤り**: §4 の不変条件表のうち、fills 破棄 or orders キャンセルの実装漏れにより agent が矛盾した observation を受け取る。
  - **Mitigation**: `SessionLifecycleEvent::Reset` 購読経路を単一の entry point とし、ここから個別の state（orders / fills / balance / narrative / client_order_id map）を一括リセットする。各 state リセットの E2E 検証を追加。
- **UI から `advance` を誤爆する危険**: `⏭` ボタン 1 クリックで長時間 instant 実行 → UI フリーズ。
  - **Mitigation**: §5 で `until_ms` 必須 + UI 発火時の `UI_ADVANCE_CAP_MS` 上限を API 契約として固定。UI 実装は不変条件違反を構造的に起こせない形にする。
- **Loading 中の 409 実装漏れ**: §6 のマトリクスのうち Loading 中の全コマンド 409 拒否が漏れると、非決定的な race 発生。
  - **Mitigation**: `ReplaySession::Loading { pending_count }` 判定を各ハンドラ冒頭で明示的に行い、unit test で 3 状態 × 全コマンドのマトリクスを網羅。
- **起動時 fixture 自動 Play 廃止による E2E boot テストの破壊**: fixture で起動直後に Play 済み状態を期待するテストが失敗する。
  - **Mitigation**: 該当テストを洗い出し、テスト冒頭に `POST /api/replay/toggle` + `POST /api/agent/session/default/step` の明示呼び出しを追加する（実装計画のサブフェーズ T で棚卸し）。
- **`session_id = "default"` 固定の誤用**: 複数 session 対応時に path はあるがサーバーが受理しない期間に agent が誤解する。
  - **Mitigation**: 非 `"default"` は `501 Not Implemented` + `{"error": "multi-session not yet implemented; use 'default' until Phase 4c"}` のレスポンス本文で明示拒否する。
