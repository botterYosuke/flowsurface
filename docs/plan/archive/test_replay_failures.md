# test_replay.py — 9 件失敗の原因調査と修正

作成日: 2026-04-21
対象: `tests/python/test_replay.py`

## サマリ

`tests/python/test_replay.py` の 14 件中 9 件が失敗している。原因は 4 カテゴリに分類でき、全てテスト側の前提が現行 API の設計／Windows 上の httpx 挙動と合っていないことが根本。API 側の設計は既存 Rust ユニットテスト（`src/replay/mod.rs` 3 件）・E2E スクリプト（`tests/e2e/*.py`, `.sh`）と整合しており、修正はテスト側に閉じる。

## 各失敗の根本原因と修正方針

### カテゴリ D: httpx 例外階層（1 件）

- **対象**: `test_not_running_error_on_wrong_port`
- **原因**: Windows では未バインドポートへの TCP が即時 RST を返さずタイムアウトする → httpx は `ConnectTimeout` を送出。httpx 1.x で `ConnectTimeout` は `ConnectError` のサブクラスではなく兄弟 → `except ConnectError` で捕まらない。
- **裏取り**: `tests/python/conftest.py` L21 でも既に `except (httpx.ConnectError, httpx.ConnectTimeout)` として両方を catch している → 先例が同じ結論。
- **方針**: テスト側を `except (httpx.ConnectError, httpx.ConnectTimeout)` に拡張する。

### カテゴリ B: 日時フォーマット不一致（6 件、連鎖）

- **対象**: `test_play_returns_dict`, `test_pause_returns_dict`, `test_resume_after_pause`, `test_step_forward_returns_dict`, `test_step_backward_returns_dict`, `test_order_buy_returns_dict`
- **原因**: テスト定数 `START = "2024-01-15 09:00:00"` / `END = "2024-01-15 15:30:00"`（秒あり）を POST するが、`src/replay_api.rs:429` の `validate_datetime_str` は `"%Y-%m-%d %H:%M"`（秒なし）を要求 → 400 Bad Request で各テストが冒頭で死ぬ。
- **裏取り**:
  - サーバ側ユニットテスト `route_post_play_valid_json`（`src/replay_api.rs:1190`）は `"2026-04-01 09:00"`（秒なし）を正解ケースとしている。
  - E2E 全スクリプト（`s1_basic_lifecycle.py`, `e2e_replay_api.sh`, `s29_tachibana_holiday_skip.py` ほか）も全て秒なし形式で POST している。
  - UI のリプレイ範囲入力（`ReplayRangeInput`）も秒なし表示（`src/replay/mod.rs:681` テスト）。
- **方針**: テスト側を秒なし `"2024-01-15 09:00"` に合わせる。サーバ側の API 形式は既に全エコシステム（UI 入力・Rust テスト・既存 E2E）で秒なし統一されているため変更しない。

### カテゴリ A: status フィールド省略の設計（1 件）

- **対象**: `test_status_has_status_field`
- **原因**: `ReplayStatus.status` は `Option<String>` + `skip_serializing_if = "Option::is_none"`（`src/replay/mod.rs:93-94`）。Session が Idle／Live モードでは status は JSON から omit される。
- **裏取り**:
  - 既存 Rust ユニットテストが 3 件この設計を明示検証（`to_status_live_mode_no_clock`, `to_status_includes_range_input`, `to_status_live_serializes_without_optional_fields`）。
  - E2E スクリプト `s1_basic_lifecycle.py:267` も `play_res.get("status", "")` で省略を許容。
- **方針**: テスト側を修正。Active セッション中は `status` が present である、という実際の仕様を検証する。具体的には冒頭で `/api/replay/play` を呼び、セッションを Active／Loading に遷移させてから assert する。

### カテゴリ C: /api/replay/state は Active セッションを要求（1 件）

- **対象**: `test_state_returns_dict`
- **原因**: `GET /api/replay/state` は `self.replay.get_api_state(50)` が `Some` を返すときのみ 200 を返す（`src/app/api/replay.rs:224-272`）。Idle では 400、Loading では 503。テストは `_get` 内で `raise_for_status()` を呼ぶため 400/503 は例外になる。
- **同じ条件で pass している `test_portfolio_returns_dict` / `test_orders_returns_dict` との差**: Portfolio/Orders は `virtual_engine.is_some()` だけで 200 を返す（`src/app/api/replay.rs:209-223, 274-283`）。virtual_engine は Replay モードに入った時点で `Some(VirtualExchangeEngine::new(...))` が作られる（`src/app/mod.rs:58`, `src/app/dashboard.rs:341`）ので、既に Replay モードで起動中なら Session が Idle でも 200 が返る。
  - 一方 `state` は `get_api_state` が `ReplaySession::Active` のときのみ `Some` を返す（`src/replay/controller/api.rs:81`）→ 追加の前提条件がある。
- **方針**: テスト側で冒頭 `/api/replay/play` を呼び、Active 遷移を短いポーリングで待つ。503（Loading）を許容しながら最大 10 秒ポーリング。

## 修正対象と理由の一覧

| # | テスト | カテゴリ | 修正対象 | 理由 |
|---|---|---|---|---|
| 1 | test_not_running_error_on_wrong_port | D | テスト | httpx の例外階層。Windows の実挙動。 |
| 2-7 | play/pause/resume/step-forward/step-backward/order_buy | B | テスト | API 仕様は秒なし。既存 E2E 全件が秒なし統一。 |
| 8 | test_status_has_status_field | A | テスト | `status` は Optional が設計（Rust UT 3 件が明示検証）。 |
| 9 | test_state_returns_dict | C | テスト | Active セッション待ちが必要。仕様通り。 |

## TDD 進捗

### サブフェーズ 1: カテゴリ D（httpx 例外階層）

- [x] RED: 現状の失敗（= app 停止中は skip されるため、ポート 19999 への httpx.get を直接検証）。`conftest.py` L21 が既に `(ConnectError, ConnectTimeout)` を catch している先例あり。
- [x] GREEN: `except (httpx.ConnectError, httpx.ConnectTimeout)` に拡張 → `tests/python/test_replay.py:154`
- [x] REFACTOR: 不要

### サブフェーズ 2: カテゴリ B（日時フォーマット）

- [x] RED: サーバ側 `validate_datetime_str` が `%Y-%m-%d %H:%M` のみ受理することを `src/replay_api.rs:429` / UT `route_post_play_valid_json` で確認。
- [x] GREEN: `START = "2024-01-15 09:00"`, `END = "2024-01-15 15:30"` に変更 → `tests/python/test_replay.py:17-18`
- [x] REFACTOR: 不要

### サブフェーズ 3: カテゴリ A（status 省略）

- [x] RED: `ReplayStatus.status` が Idle/Live で omit される設計であることを `src/replay/mod.rs:93` と UT 3 件で確認。
- [x] GREEN: `test_status_has_status_field` で先に `/api/replay/play` を呼ぶ → `tests/python/test_replay.py:40-46`
- [x] REFACTOR: 不要

### サブフェーズ 4: カテゴリ C（state は Active が必要）

- [x] RED: `GET /api/replay/state` が `ReplaySession::Active` のみ 200 を返すことを `src/app/api/replay.rs:224-272` / `src/replay/controller/api.rs:81` で確認。
- [x] GREEN: `test_state_returns_dict` で play → 503 を許容する短ポーリング → `tests/python/test_replay.py:103-122`
- [x] REFACTOR: 1 箇所のみなのでヘルパー抽出は見送り

## 完了条件

| # | 条件 | 検証コマンド |
|---|---|---|
| 1 | `test_replay.py` 全 14 件 pass | `.venv/Scripts/python.exe -m pytest tests/python/test_replay.py -v` |
| 2 | 他の Python test を壊していない | `.venv/Scripts/python.exe -m pytest tests/python/` |
| 3 | Rust テスト全通過 | `cargo test --lib -p flowsurface` / `cargo test --bin flowsurface` |
| 4 | `cargo fmt --check` pass | `cargo fmt --check` |
| 5 | clippy 新規 warning なし | `cargo clippy --lib --tests -- -D warnings` |

## スコープ外

- `src/replay_api.rs` / `src/app/api/replay.rs` / `src/replay/mod.rs` は変更しない（API 仕様は既存コードと整合しており、変更するとむしろ既存 Rust UT・E2E を壊す）。
- 未コミット変更（`.claude/settings.json` のみ）はそのまま残す。

## コミット分割

1. `fix(test): httpx ConnectTimeout も接続失敗として扱う`（カテゴリ D）
2. `fix(test): /api/replay/play の日時フォーマットを秒なしに統一`（カテゴリ B）
3. `fix(test): status/state は Active セッション前提にそろえる`（カテゴリ A + C）
