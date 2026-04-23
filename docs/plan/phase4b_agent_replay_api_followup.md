# Phase 4b-1 follow-up: 自動再生機構の廃止と agent session API への統合

**親計画**: [phase4b\_agent\_replay\_api.md](https://www.google.com/search?q=phase4b_agent_replay_api.md)
**関連 ADR**: [ADR-0001 Agent 専用 Replay API の新設と自動再生機構の廃止](https://www.google.com/search?q=../adr/0001-agent-replay-api-separation.md)（本計画で実装）

> **ADR 運用メモ**: 当初 ADR-0002 として起案したが、ADR-0001 が `proposed` 段階であったため同日内に ADR-0001 に統合して単一 ADR として確定した。経緯は ADR-0001 冒頭参照。
> **起案日**: 2026-04-22
> **TDD 方針**: `.claude/skills/tdd-workflow/SKILL.md` に準拠（RED → GREEN → REFACTOR）

-----

## 0\. ゴール

自動再生機構（wall-clock 駆動 tick ループ + StepClock 状態機械 + pause/resume/speed/step-backward(1step) 機能）を **全廃** し、セッション内の時刻操作を agent session API に一本化する。[ADR-0001 §2〜§8](https://www.google.com/search?q=../adr/0001-agent-replay-api-separation.md) の実装。

### Definition of Done

  - [x] `/api/replay/{play, pause, resume, speed, step-forward, step-backward}` ルートが全削除され、404 を返す
  - [x] `POST /api/replay/toggle`（Live→Replay）が body `{start, end}` で session を初期化できる（klines ロード + Active 遷移、auto-tick なし）
  - [x] `POST /api/agent/session/:id/rewind-to-start` が新設され、未初期化時は body `{start, end}` で session を初期化し、初期化済みなら clock を range.start に巻き戻す
  - [x] `POST /api/agent/session/:id/advance` が GUI ビルドでも利用できる（要件精査 `phase4b_agent_replay_api.md` DoD \#7 を参照し、サブフェーズ V で文書化 + 再評価）
  - [x] `ReplayCommand::{Pause, Resume, StepForward, CycleSpeed, StepBackward, Play}` が enum から削除される
  - [x] `StepClock` から speed / pause / resume 状態と `now_ms()` / `tick_until()` 以外の旧制御 API が除去される
  - [x] `ReplayController::is_paused()` / `ReplayState::is_paused()` 削除。呼び出し元全て除去
  - [x] iced subscription / tokio timer の自動 tick 経路削除
  - [x] UI: `▶` = step, `⏭` = advance, `⏮` = rewind-to-start, `⏹` / 速度切替ボタン撤去
  - [x] 起動時 fixture 自動 Play (`pending_auto_play`) 廃止。Replay モードで起動した場合 session は初期化 + 停止
  - [x] `src/headless.rs` の `step_forward` / `pause` / `resume` / `cycle_speed` ハンドラ削除
  - [x] `headless_mode` runtime ゲート（advance 用）削除
  - [x] Python SDK: `fs.replay.*` から step\_backward / step\_forward / pause / resume / speed 撤去。`fs.agent_session.rewind_to_start()` 追加
  - [ ] `SessionLifecycleEvent::{Started, Reset, Terminated}` の発火点と購読側（agent state / narrative / UI / `client_order_id` UNIQUE map）の配線が確立され、UI リモコンハンドラが agent API state を直接触る経路が残っていない
  - [ ] ADR-0001 §6 状態×コマンド受理マトリックス（3 状態×全コマンド）を網羅する unit test が追加され PASS
  - [ ] `client_order_id` UNIQUE map が `SessionLifecycleEvent::Reset` 購読経由でクリアされる
  - [x] `pending_auto_play` 経路が全削除され、Replay モード起動時に session 初期化 + 停止
  - [ ] `cargo fmt` / `cargo clippy -D warnings` / `cargo test` 全 PASS
  - [x] E2E 調査完了。壊れたテストを書き換え or 削除

-----

## 1\. スコープ・非スコープ

### スコープ

  - ADR-0001 §2〜§8 の Rust 実装
  - UI 配線の agent session API への繋ぎ替え
  - Python SDK 更新
  - E2E テスト調査と書き換え・削除
  - Spec / Wiki / ADR ドキュメント更新

### 非スコープ

  - `heatmap.rs` 系の widget 独自 `is_paused`（Replay と無関係）
  - 複数 session 並行（Phase 4c）
  - uAgents / Agentverse ブリッジ（Phase 4b-2）
  - OpenAPI 自動生成

-----

## 2\. 事前確認結果（2026-04-22）

| 項目 | 結論 |
|---|---|
| `ReplayCommand::StepBackward` 改称 | **削除**（RewindToStart variant は作らず、agent session ハンドラが直接 controller メソッドを呼ぶ） |
| `is_paused()` 残置 | **撤去**。呼び出し元は UI 分岐 / headless / session.rs / tests のみで、いずれも本計画で解体 |
| `ReplaySession::{Loading, Active}` 統合 | **しない**。Pause/play は `StepClock` 側の状態で、`ReplaySession` enum は機能的に分離されているので現状維持 |
| `/api/replay/toggle` と `/api/replay/play` | `toggle` のみ維持。`play` は **削除** |
| `rewind-to-start` HTTP パス | `/api/agent/session/:id/rewind-to-start` に新設（`/api/replay/*` には置かない） |
| 初期化トリガー | `toggle(Live→Replay)` と `rewind-to-start` の冒頭で遅延初期化（専用 start エンドポイントなし） |
| `rewind-to-start` の初期化済み時 | clock のみ巻き戻し（klines 再ロードなし） |
| `step` / `advance` の未初期化時 | 400 を返す |
| 起動時 fixture 自動 Play | 廃止。Replay モードで起動時は session 初期化 + 停止 |

-----

## 3\. サブフェーズ分割（TDD）

各サブフェーズで `cargo fmt` / `cargo clippy -D warnings` / `cargo test` を通す。E2E は該当時のみ Windows 実機確認し、結果を本計画書に記録。

### サブフェーズ L: 削除対象ルートの RED + §6 マトリックス網羅テスト

**目的**: 削除対象の `/api/replay/{play,pause,resume,speed,step-forward,step-backward}` が 404 を返すこと、および ADR-0001 §6 状態×コマンド受理マトリックスを期待するテストを先に書く。

  - [ ] `src/replay_api.rs` のルーティングテーブルに対する unit test（`tests/routing.rs` 相当）で、各パスが `None` or 404 相当を返すことを検証
  - [ ] **ADR-0001 §6 マトリックス網羅 unit test** を追加：`{Idle, Loading, Active}` × `{toggle(body あり), toggle(body なし), step, advance, rewind(body あり), rewind(body なし), status}` の全セルで期待レスポンス（200 / 400 / 409 / 501）を検証。特に以下を漏らさない
      - Loading 中の全状態遷移系コマンドが 409（キューイングしない）
      - 未初期化での `step` / `advance` が 400 (`session not initialized`)
      - `rewind-to-start` (body なし + Idle) が 400 (`body required for init`)
  - [ ] 期待が失敗することを確認（RED）
  - [ ] Python 側 `tests/python/test_replay.py` で既存 `_post("/api/replay/play", ...)` 呼び出しが 404 で失敗することを確認する補足テストを書く（最終的には削除されるテストだが RED 確認用）

### サブフェーズ M: ReplayCommand enum 整理

  - [x] `ReplayCommand::{Pause, Resume, StepForward, CycleSpeed, StepBackward, Play}` variant を削除
  - [x] 関連する match アームを `src/app/api/replay.rs` / `src/headless.rs` / `src/replay_api.rs` から除去
  - [x] コンパイルが通り、L で追加した RED テストが GREEN 化することを確認

### サブフェーズ N: 自動 tick 購読経路の解体 + 起動時 auto-play 廃止

  - [x] iced subscription から Replay 専用 tick 経路（`iced::time::every(100ms)` headless fallback + `iced::window::frames()` の replay 分岐）を削除
  - [x] `src/app/handlers.rs::handle_tick` から Replay 進行ブロックを削除（*描画 tick 配送は維持*。ハンドラ自体は `iced::window::frames()` 由来の dashboard 描画更新に必要なため残す）
  - [x] `src/app/api/replay.rs` の `GetStatus` CI auto-tick hack 削除
  - [x] `src/replay/controller/tick.rs` の `ReplayController::tick` / `TickOutcome` を削除
  - [x] tokio timer の自動 tick 発火（headless の `tick_interval`）を除去
  - [x] `pending_auto_play` 経路（起動時 fixture 復元 → 全ペイン Ready で自動 Play）を廃止。関連フィールド / getter・setter / 呼び出し元 / `on_manual_play_requested` / `on_session_unavailable` / ユニットテスト 4 件を削除。Replay モードで起動しても session は Idle のまま停止
  - [x] 既存ユニットテストが全 PASS（560 件）

### サブフェーズ O: StepClock 縮退（P と 1 PR で纏める）

> **ビルド順序の注意**: O 完了の瞬間に P 未着手だと `src/app/view.rs` が `speed_label()` / `is_paused()` を参照してコンパイル不能になる。*O と P は 1 PR で纏める*（先に P-a: UI 参照削除、次に O: StepClock 縮退、最後に P-b: 新 Message 配線）。

  - [x] **P-a（先行）**: `src/app/view.rs` から `speed_label()` / `is_paused()` 参照および `⏹` / 速度ボタン生成ロジックを削除（新 Message 配線はまだしない）
  - [x] `StepClock` から `speed` / `paused` / `status()` / `pause()` / `resume()` / `cycle_speed()` / `speed_label()` 削除
  - [x] `now_ms()` と `tick_until(target_ms)` のみに縮退
  - [x] `format_speed_label` / `cycle_speed_value` / `SPEEDS` 定数削除
  - [x] `ReplayController::is_paused()` / `ReplayState::is_paused()` 削除
  - [x] `src/replay/mod.rs` の `is_paused_returns_true_when_clock_is_paused` 等の関連テスト削除

### サブフェーズ P: UI ボタン配線の繋ぎ替え

  - [x] `src/app/view.rs` の `▶` / `⏭` / `⏮` ボタン生成ロジックを書き換え
      - `▶` → `Message::Agent(AgentMessage::Step)` 相当（新規 Message variant）
      - `⏭` → advance 発行。*UI 発火 advance は ADR-0001 §5 に従い `until_ms = min(range.end, now_ms + UI_ADVANCE_CAP_MS)` を渡す。UI\_ADVANCE\_CAP\_MS は `3_600_000`（1 時間）で確定*
      - `⏮` → rewind-to-start 発行
  - [x] `ReplayUserMessage::{Pause, Resume, StepForward, StepBackward, CycleSpeed}` を削除
  - [x] `AgentMessage` 新設 or 既存 Message へ追加
  - [x] Windows 実機で手動確認（ボタンクリックで HTTP 経由と同等の動作）→ 結果記録

### サブフェーズ Q: `advance` の GUI ガード削除 + SessionLifecycleEvent 配線

  - [x] `src/replay_api.rs` / `src/main.rs` の `headless_mode` runtime ゲート（advance 用）を削除
  - [x] GUI ビルドで advance を叩く E2E を新規追加（従来 S57 の仕様反転）
  - [ ] **`SessionLifecycleEvent::{Started, Reset, Terminated}` を `VirtualExchange` から発火**
      - `Started`: `toggle(Live→Replay)` 完了時 / `rewind-to-start(body あり, 未初期化)` 完了時
      - `Reset`: `rewind-to-start(初期化済み)` 実行時
      - `Terminated`: `toggle(Replay→Live)` 実行時
  - [ ] **購読側の配線**: agent state の `client_order_id` UNIQUE map / narrative state / UI（チャート再描画）が `Reset` を購読して一括初期化する経路を確立。UI リモコンハンドラが agent API state を直接触るコードがないことを `grep` で確認
  - [ ] ADR-0001 §4 リセット不変条件表の各項目（open orders 全件キャンセル / fills 全件破棄 / 仮想残高 / position / narrative / client\_order\_id map / UI 再描画）をカバーする unit test or integration test を追加
  - [x] 親計画 `phase4b_agent_replay_api.md` の DoD \#7 取消線追記（サブフェーズ V で対応、ここでは触れない）

### サブフェーズ R: `is_paused()` 呼び出し元の書き換え

  - [x] `src/app/view.rs:53` 等の UI 側 `is_paused()` 参照削除（サブフェーズ P で概ね解決済のはず。取りこぼし確認）
  - [x] `src/headless.rs:373,473` は対応するハンドラごとサブフェーズ S で削除されるため実質対象外
  - [x] `src/replay/controller/session.rs:226` の `is_paused` 依存コードを解体
  - [x] chart widget 側 `self.anchor.is_paused()` は **対象外**（widget スクロール状態）

### サブフェーズ S: headless の重複実装削除

  - [x] `src/headless.rs` の以下ハンドラ削除
      - `ApiCommand::Replay(ReplayCommand::Pause)` → 削除
      - `ApiCommand::Replay(ReplayCommand::Resume)` → 削除
      - `ApiCommand::Replay(ReplayCommand::StepForward)` → 削除
      - `ApiCommand::Replay(ReplayCommand::CycleSpeed)` → 削除
      - `ApiCommand::Replay(ReplayCommand::StepBackward)` → 削除
      - `ApiCommand::Replay(ReplayCommand::Play)` → 削除
  - [x] `headless_mode` runtime 判定経路の簡素化
  - [ ] Headless E2E（`IS_HEADLESS=true`）が agent session API 経由で動作することを確認

### サブフェーズ T: 既存 E2E 調査

Windows 実機実行が必要。grep ベースで 98 ファイル規模の依存が判明しており、全件の取り扱いを机上で決めるのは ROI が悪い。*カテゴリごとの処分方針* を先に確定し、個別ファイル一覧は本サブフェーズ冒頭で grep 走査して作成する。

**カテゴリ別処分方針（ADR-0001 §2〜§8 の確定事項に基づく）**:

1.  **自動再生依存テスト** → **削除**
    対象: `s3_autoplay*`, `s9_speed_step*`, `s14_autoplay_event_driven*`, `s27_cyclespeed_reset*`, `tests/e2e/archive/s9_speed_step.sh`, `tests/e2e/archive/x2_buttons.sh` 等。自動再生機構が消えるため存在意義がなくなる。

2.  **step-backward(1 step 戻る) 依存テスト** → **削除 or 書き換え**

      - `s13_step_backward_quality.py` → **rewind-to-start テストへ書き換え**（意味的に「巻き戻し操作の品質」なので後継として再利用）
      - その他 step-backward 呼び出しは削除

3.  **play / pause / resume / speed 呼び出しを含むセットアップ** → **機械置換**
    `_post("/api/replay/play", {"start":..., "end":...})` を `_post("/api/replay/toggle", {"start":..., "end":...})` に機械置換。pause / resume / speed 呼び出しは除去（自動再生の概念がないため）。

4.  **advance GUI 拒絶を期待するテスト (S57)** → **仕様反転で書き直し**
    GUI で advance が受理されることを検証するテストへ書き直し。

5.  **agent session 系 (S55 / S56 / S58 / S59)** → **影響確認のみ**
    初期化経路の変更（play 削除）に合わせて冒頭を toggle に置換。本体ロジックは据え置き。

6.  **Python SDK / Jupyter notebook** → **サブフェーズ U で同時更新**

**作業**:

  - [x] 上記カテゴリに従って対象ファイルを grep で全走査し一覧化
  - [x] 削除 / 書き換え / 機械置換の対応を一覧にチェックインしながら実施
  - [x] `.github/workflows/e2e.yml` の該当スクリプト行を整理

### サブフェーズ U: Python SDK 更新

  - [x] `python/fs/replay.py` から `step_forward` / `step_backward` / `pause` / `resume` / `speed` / `play` を削除（現行配置は `python/replay.py`）
  - [x] `python/fs/agent_session.py` に `rewind_to_start(start: str, end: str)` 追加（現行配置は `python/agent_session.py`）
  - [x] `python/fs/replay.py::toggle` の body に `{start, end}` を受け入れる署名変更（現行配置は `python/replay.py`）
  - [x] Python 側のテスト `tests/python/test_agent_session.py` に rewind-to-start ケース追加
  - [x] docstring / type hints 更新

### サブフェーズ V: ドキュメント更新

  - [x] `docs/adr/0001-agent-replay-api-separation.md` – サブフェーズ V 完了時に Status を `proposed` → `accepted` に上げる
  - [x] `docs/plan/phase4b_agent_replay_api.md` – DoD \#7（advance GUI 400）に取消線追加 + 「ADR-0001 統合改訂により撤回」注記
  - [x] `docs/spec/agent_replay_api.md` – `rewind-to-start` 追加、GUI advance 許容を反映、現行実装の未初期化時 404 を明記
  - [x] `docs/spec/replay.md` – pause/resume/speed/step-backward(1step)/step-forward セクション削除、StepClock 縮退を反映
  - [x] `docs/wiki/replay.md` – ユーザー向け操作説明を新 UI（Step / Advance / Rewind-to-start）に書き換え
  - [x] `.github/pull_request_template.md` – 「`/api/replay/*` 新規ルート禁止」条項の維持、「削除は許容」を追記
  - [x] `.github/workflows/adr_guard.yml` – grep lint の対象ルートリスト更新
  - [x] `README.md:47` – 再生制御の記述を「Step / Advance / RewindToStart」に修正

-----

## 4\. 進捗

| サブフェーズ | 状態 | 日付 | コミット | メモ |
|---|---|---|---|---|
| L: 削除対象ルート RED + §6 マトリックス | 🚀 | 2026-04-22 | 08198dd | **一部完了**。削除ルート 404 期待テスト 6 件のみ追加。他は後続で追加 |
| M: ReplayCommand enum 整理 | ✅ | 2026-04-22 | 08198dd | enum variant 6 個削除 + route 行 + parse\_play\_command 削除 |
| N: 自動 tick 解体 + auto-play 廃止 | ✅ | 2026-04-22 | 836e82b | main.rs subscription から replay 専用 tick 経路削除。pending\_auto\_play 廃止 |
| O: StepClock 縮退 | ✅ | 2026-04-23 | ea24979 | speed/ClockStatus/pause 等削除。now\_ms/tick\_until のみに縮退 |
| P: UI ボタン配線繋ぎ替え | 🚀 | 2026-04-23 | ea24979 | **一部完了**。AgentMessage::{Step, Advance, RewindToStart} 新設と UI 配線 |
| Q: advance GUI ガード削除 + rewind reset 不変条件 | ✅ | 2026-04-23 | 0a1f406 | **ADR-0001 §3 / §4 + hotfix follow-up 完了**。GUI advance の 400 ガード削除、rewind reset の実装等 |
| R: is\_paused 呼び出し元書き換え | ✅ | 2026-04-23 | - | ReplayController / ReplayState 系の参照なしを grep で確認。chart widget の anchor pause は対象外 |
| S: headless の重複実装削除 | 🚀 | 2026-04-23 | - | 旧 ReplayCommand variant ハンドラ削除は確認済み。Headless E2E 実機確認のみ残 |
| T: E2E 調査 | ✅ | 2026-04-23 | - | スクリプト一括置換および obsolete なテストの削除完了。e2e.yml 更新済 |
| U: Python SDK 更新 | ✅ | 2026-04-23 | - | `fs.replay` は toggle/status/save/set-mode のみ。`fs.agent_session.rewind_to_start()` と `AgentRewindResponse` 追加 |
| V: ドキュメント更新 + ADR Status accepted | ✅ | 2026-04-23 | - | ADR accepted、親計画 DoD \#7 撤回、spec/wiki/README/PR template/adr_guard 更新 |

進捗があり次第 ⏳ → 🚀 → ✅ で更新。

### 4.1 今回意図的に残す項目

以下は今回のホットフィックス完了条件から外し、次の専用サブフェーズで扱う。

  - ADR-0001 §6 の 3 状態 × 全コマンド受理マトリックス unit test（現行実装は未初期化 step/advance を 404 で返しており、計画書上の 400 と差分があるため先に仕様決定が必要）
  - `SessionLifecycleEvent` enum そのものの導入。現行コードは `VirtualExchange::session_generation()` + `AgentSessionState::observe_generation()` で reset/started/terminated 相当を購読している
  - Headless E2E 実機確認と `cargo clippy -D warnings` 全通過。既存警告前提のため、本変更では新規警告を増やさない範囲の確認に留める
