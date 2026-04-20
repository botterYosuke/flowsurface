# replay/controller.rs モジュール分割リファクタリング計画

## 目的

`src/replay/controller.rs`（1823行）を責務ごとに分割し、各ファイルを管理しやすいサイズに保つ。
動作は一切変更しない。`main.rs` の公開インターフェースは維持する。

## 分割後の構造

```
src/replay/controller/
├── mod.rs      — 構造体定義・コンストラクタ・getter/setter・handle_message ディスパッチ
├── api.rs      — ApiStateData・API クエリメソッド（get_api_state 等）
├── session.rs  — handle_user_message・handle_load_event・seek ヘルパー群
└── tick.rs     — TickOutcome・tick()・handle_system_event・synthetic_trades
```

## ファイル別メソッド分配

### controller/mod.rs（コア + getter/setter）

| メソッド | 種別 |
|---|---|
| `struct ReplayController` + `From<ReplayState>` + `from_saved` | 構造体・コンストラクタ |
| `is_replay/playing/paused/loading`, `has_clock`, `is_at_end` | 状態 getter |
| `mode`, `speed_label`, `range_input_start/end` | 状態 getter |
| `is_auto_play_pending`, `clear_pending_auto_play` | auto-play |
| `set_range_start/end`, `play_with_range` | setter/コマンド |
| `on_session_unavailable`, `to_status`, `format_current_time` | 委譲/変換 |
| `handle_message` (dispatcher only) | ディスパッチ |

テスト: P3（セッション状態遷移）+ P2（play_with_range）+ アイドル状態テスト

### controller/api.rs（API レスポンス生成）

| メソッド | 種別 |
|---|---|
| `const TRADE_WINDOW_MS` | 定数 |
| `pub struct ApiStateData` | 型定義 |
| `last_close_price`, `current_time_ms` | 状態読み取り |
| `get_api_state` | API レスポンス生成 |
| `active_kline_streams`, `active_stream_debug_labels` | ストリーム情報 |

テスト: `get_api_state_*` 全テスト（~250行）

### controller/session.rs（ユーザー操作 + ロードイベント）

| メソッド | 種別 |
|---|---|
| `handle_user_message` | ユーザー操作処理 |
| `handle_load_event` | 非同期ロード結果処理 |
| `seek_to`, `handle_range_input_change` | seek ヘルパー |
| `reset_session`, `inject_klines_up_to` | 内部ヘルパー |

テスト: B-3（StepBackward）+ StepForward + CycleSpeed + StartTimeChanged/EndTimeChanged + P1（seek_to）+ P4（load_event）

### controller/tick.rs（Tick 処理 + システムイベント）

| メソッド | 種別 |
|---|---|
| `pub struct TickOutcome` | 型定義 |
| `tick()` | リアルタイム Tick 処理 |
| `handle_system_event` | SyncReplayBuffers/ReloadKlineStream |
| `synthetic_trades_at_current_time` | 合成トレード生成 |

テスト: なし（handle_system_event の単純な委譲テストは省略）

## 実装手順

1. ✅ 計画書作成
2. ✅ controller/ ディレクトリ作成・mod.rs として全コードをコピー
3. ✅ TDD Red: api.rs にスタブテスト → cargo test → コンパイルエラー確認
4. ✅ api.rs へ実装・テストを移動（Green）→ cargo check ✓
5. ✅ session.rs へ実装・テストを移動（Green）→ cargo check ✓
6. ✅ tick.rs へ実装を移動（Green）→ cargo check ✓
7. ✅ 最終検証: cargo test（356 passed）/ cargo clippy -- -D warnings（0 warnings）/ cargo fmt --check（clean）

## 設計メモ

### Rust のモジュール可視性について

- `controller/api.rs` は `replay::controller::api` モジュール（`controller` の子モジュール）
- Rust では private フィールドは「定義モジュールとその子モジュール」からアクセス可能
- したがって `api.rs`, `session.rs`, `tick.rs` から `self.state` に直接アクセスできる ✓
- `super::` = `replay::controller`、`super::super::` = `replay`

### pub re-export について

- `ApiStateData` を `replay_api.rs` 等が使う場合は `pub use api::ApiStateData;` を `mod.rs` に追加
- 現状 `ApiStateData` は `controller.rs` 内のみで使われているため再エクスポート不要

### インポート戦略

各サブファイルでは以下を使用:
```rust
use super::{ReplayController, /* controller local types */};
use super::super::{ReplayLoadEvent, ReplayMessage, ReplaySession, ...};
// または
use crate::replay::{ReplayLoadEvent, ReplayMessage, ReplaySession, ...};
```

### 制約・注意点

- `seek_to` は `handle_user_message`（session.rs）から呼ばれるため、session.rs に置く
- `inject_klines_up_to` は `seek_to` と `handle_user_message` 両方から呼ばれるため session.rs に置く  
- `handle_range_input_change` は `handle_user_message` から呼ばれるため session.rs に置く
- `reset_session` は `handle_load_event` から呼ばれるため session.rs に置く
- `tick()` は `handle_system_event` と独立しているが、どちらも Tick/システム系のため tick.rs に置く

## 予想ファイルサイズ

| ファイル | 実装行数 | テスト行数 | 合計 |
|---|---|---|---|
| `mod.rs` | ~250 | ~160 | ~410 行 ✓ |
| `api.rs` | ~140 | ~250 | ~390 行 ✓ |
| `session.rs` | ~423 | ~367 | ~790 行 △ |
| `tick.rs` | ~205 | 0 | ~205 行 ✓ |

`session.rs` が目標 500 行を超える見込み。実装部分自体は ~423 行で目標内。
`handle_user_message` の `Play` アーム（~150行）が大きいため、これを分割すれば更に削減可能だが、
関数の論理的まとまりを壊すため本リファクタリングのスコープ外とする。

## Tips

- `cargo check` を各ステップ後に実行してコンパイルエラーを早期検出する
- sub-module の `impl ReplayController` ブロックは同一型への追加実装として問題なく機能する
- テストの `use super::*;` は `mod.rs` で定義した型と `pub use api::ApiStateData` を取り込む
- 各サブモジュールのテストブロックで `use super::super::*` か `use crate::replay::*` を使う
