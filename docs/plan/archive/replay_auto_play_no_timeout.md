---
name: Replay auto-play タイムアウト廃止（イベント駆動化）
description: 起動時 auto-play の 30 秒タイマーを撤廃し、metadata 到着イベント駆動で ResolveStreams を push する設計
type: project
---

# Replay auto-play タイムアウト廃止 設計プラン

**作成日**: 2026-04-13
**対象**: [src/main.rs](../../src/main.rs), [src/replay/mod.rs](../../src/replay/mod.rs)
**状態**: 全 Phase 完了・E2E 3 シナリオ PASS（2026-04-13）
**関連**: [replay_fixture_direct_boot.md](./replay_fixture_direct_boot.md), [replay_header.md](../replay_header.md)

## 背景

[replay_fixture_direct_boot.md](./replay_fixture_direct_boot.md) で導入した
`pending_auto_play` は、`saved-state.json` に `replay` 構成を含めた直接起動時、
全ペインの streams が Ready になった瞬間に `ReplayMessage::Play` を発火する仕組み。

現状、保険として **30 秒のデッドライン** が設定されており、期限内に streams が解決しなければ
`Replay auto-play timed out: streams did not resolve within 30s` トーストを出して flag を落とす。

### 現れている問題

`TachibanaSpot:7203|TOYOTA` を含む replay state で起動すると、**ほぼ確実にタイムアウトする**:

1. 起動直後 `pending_auto_play = true`、30 秒タイマー始動
   （[src/main.rs:220-233](../../src/main.rs#L220-L233)）
2. ペインの `ResolveStreams` イベントは [src/main.rs:524](../../src/main.rs#L524) で
   `has_any_ticker_info == false` のうちはスキップされる
3. Tachibana の銘柄マスタ（`tickers_info`）は **ログイン → `start_master_download()`**
   （[src/main.rs:2107](../../src/main.rs#L2107)）完了後に初めて揃う
4. 未ログイン状態では永久に揃わず、ログイン済みでもマスタ取得が 30 秒超なら間に合わない
5. [src/main.rs:363-369](../../src/main.rs#L363-L369) でタイムアウト → エラートースト

## 根本原因

**auto-play が時間駆動になっている**ことが問題の本質。

- `pending_auto_play` は「条件が揃うまで待つ」フラグのはずだが、
  待ち時間の上限を時計で区切っているため、非同期な metadata 到着と衝突する
- ペインの `ResolveStreams` 再送は 2 秒ごとのポーリング
  （[src/screen/dashboard/pane.rs:1882](../../src/screen/dashboard/pane.rs#L1882) の
  `due_streams_to_resolve`）であり、metadata 到着を直接トリガにしていない
- 結果として「タイマー切れ = 環境が悪い」ではなく、
  「**Tachibana ログインより先に 30 秒経った場合は必ず失敗**」という意味のない失敗モードが生まれる

## 方針: イベント駆動 auto-play

`pending_auto_play` を **条件成立まで無期限に待つ状態フラグ** に戻す。
タイマーを廃止し、代わりに「metadata が到着した瞬間に Waiting ペインへ ResolveStreams を push する」
イベントチェーンを張る。

### 基本設計

```
[Login 成功 or Session 復元]
   ↓
[start_master_download → UpdateMetadata(venue, map)]
   ↓
[Sidebar::TickersTable::UpdateMetadata ハンドラ]
   ↓ (新規) ticker metadata 更新後に dispatch
[Dashboard 全 Waiting ペインに対して ResolveStreams を push]
   ↓
[既存の ResolveStreams ハンドラ（main.rs:518-589）が解決 → Ready 昇格]
   ↓
[全ペイン Ready 判定 → ReplayMessage::Play 発火]
```

この経路のどこにもタイマーは無い。すべて非同期イベントで繋がる。

### 手動介入でのクリア

ユーザーが手動で replay を操作した時点で auto-play の責務は終わる:

- `ReplayMessage::ToggleMode` — 既存の `toggle_mode()` 内で `pending_auto_play = false` にリセット済み
  （[src/replay/mod.rs:148](../../src/replay/mod.rs#L148)）。そのまま維持
- `ReplayMessage::Play` — **新規**: ハンドラ先頭で `pending_auto_play = false` に落とす
- `SessionRestoreResult(None)` 経路 — **新規**: ログイン画面を表示する際、
  `pending_auto_play = false` に落とし、ユーザーに「リプレイはログイン後に手動再開してください」
  トースト（info）を表示

### 未ログイン時のユーザー通知

タイマーが無くなると、ユーザーが気付かず「なぜか replay が始まらない」という詰み状態が懸念される。
以下の 2 箇所で明示的にハンドリングする:

1. **Session 復元失敗 → ログイン画面表示時**
   - 「`saved-state.json` は replay 構成だが、ログインが必要なため自動再生を保留しました」旨を info トースト
   - このタイミングで `pending_auto_play = false`
2. **ログイン成功 → transition_to_dashboard 経由**
   - `pending_auto_play` は残したまま。master download 完了 → UpdateMetadata → ResolveStreams push
     → Ready → Play の順で自然に発火する

## 変更点まとめ

### 削除

| 対象 | 詳細 |
| --- | --- |
| [src/replay/mod.rs:78](../../src/replay/mod.rs#L78) | `pending_auto_play_deadline: Option<Instant>` フィールド |
| [src/replay/mod.rs:130](../../src/replay/mod.rs#L130) | `Default::default` の該当行 |
| [src/replay/mod.rs:149](../../src/replay/mod.rs#L149) | `toggle_mode()` 内の deadline リセット |
| [src/main.rs:232-233](../../src/main.rs#L232-L233) | `new()` での deadline 初期化 |
| [src/main.rs:361-372](../../src/main.rs#L361-L372) | `Tick` ハンドラ内の timeout チェック一式とエラートースト |
| [src/main.rs:575](../../src/main.rs#L575) | auto-play 発火時の `deadline = None` 代入 |
| [src/replay/mod.rs:612-635](../../src/replay/mod.rs#L612-L635) | `pending_auto_play_deadline` を参照している既存テスト（代替テストを追加） |

### 追加・変更

1. **`ReplayMessage::Play` ハンドラ冒頭で flag クリア**
   - [src/main.rs:859](../../src/main.rs#L859) の `ReplayMessage::Play` 分岐先頭で
     `self.replay.pending_auto_play = false;` を実行
   - 理由: ユーザーが UI から Play を叩いた場合でも、auto-play 経路と二重発火しないようにする

2. **Sidebar `UpdateMetadata` 後の ResolveStreams push**
   - [src/main.rs:787](../../src/main.rs#L787) の `Message::Sidebar` ハンドラで、
     `sidebar.update()` の処理結果を確認し、`UpdateMetadata` 系のメッセージだった場合に
     dashboard 全ペインへ `ResolveStreams` 相当を再送する
   - 実装方針:
     - `sidebar::Message` が `TickersTable(UpdateMetadata(..))` であることを match で検知
       （update() 呼び出し前に判定）
     - 処理後に `self.active_dashboard_mut().refresh_waiting_panes(main_window_id)` のような
       ヘルパーを新設
     - ヘルパーは各ペインの `streams.due_streams_to_resolve(Instant::now())` を強制的に評価し、
       `Waiting` なペインに対して `Action::ResolveStreams` を返す
     - もしくは既存の pane tick 経路を強制するショートカットとして、
       各ペインの `streams.mark_resolution_due()` を呼んで次の Tick で即解決させる
   - どちらを採るかは Red テスト段階で決定する（下記 TDD 節参照）

3. **`SessionRestoreResult(None)` 経路で flag クリア + info トースト**
   - [src/main.rs:300](../../src/main.rs#L300) 近傍で、`self.replay.pending_auto_play == true` なら
     `false` に落とし、info トースト「Replay auto-play was deferred: please log in to resume」を push

4. **ログ整備**
   - `[auto-play]` プレフィクスの info ログを以下の各分岐に追加:
     - metadata push で ResolveStreams を再投入した時
     - session restore 失敗で auto-play を放棄した時
     - ResolveStreams ハンドラで Play 発火した時（既存、維持）

## TDD フェーズ

### Phase 1: Red — 失敗テストを書く

#### 1.1 `src/replay/mod.rs`

- 既存 `toggle_replay_to_live_clears_pending_auto_play`
  （[mod.rs:622](../../src/replay/mod.rs#L622)）を改訂:
  `pending_auto_play_deadline` の assert を削除
- 新規 `state_has_no_deadline_field`: コンパイル時担保。`ReplayState` に該当フィールドが無いこと
  （これは構造体定義の変更で自動的に満たされるため、厳密にはテスト不要。
  Red にするためのガードとして `#[test] fn`  で `pending_auto_play_deadline` を参照するテストを
  書いて **コンパイルエラーで RED 確認** → 該当フィールドを削除して GREEN）

#### 1.2 `src/main.rs`（auto-play 経路の統合テスト的ユニット）

現状の `main.rs` には pending_auto_play 関連の単体テストが無いため、以下を追加する:

- `replay_play_message_clears_pending_auto_play`
  - `ReplayState` を `pending_auto_play = true` で構築
  - `Message::Replay(ReplayMessage::Play)` を dispatch するパスを直接叩く
    （Flowsurface::update を呼ぶのは重いので、`ReplayState` レベルのヘルパー
    `on_manual_play_requested(&mut self)` を切り出して単体テスト）
  - After: `pending_auto_play == false`

- `session_restore_failure_clears_pending_auto_play`
  - 同様に `ReplayState::on_session_unavailable(&mut self)` ヘルパーを切り出してテスト

#### 1.3 `src/screen/dashboard.rs`

`refresh_waiting_panes` のユニットテスト:

- 2 つのペインを用意（1 つは Ready、1 つは Waiting）
- `refresh_waiting_panes` を呼ぶ
- Waiting ペインに対して `ResolveStreams` アクションが生成されることを検証
  （Task の中身を直接検査できなければ、
  `streams.mark_resolution_due()` が呼ばれた副作用で `due_streams_to_resolve(now)` が
  `Some(..)` を返すことで代替検証）

### Phase 2: Green — 最小実装

上記テストを 1 つずつ PASS させる:

1. `ReplayState::pending_auto_play_deadline` を削除 → コンパイル通す
2. `ReplayState::on_manual_play_requested` / `on_session_unavailable` を追加
3. `Dashboard::refresh_waiting_panes` を追加
4. `main.rs` の呼び出しを配線

### Phase 3: Refactor

- `Tick` ハンドラから timeout ブロックを削除（デッドコード化確認後）
- `docs/replay_header.md` の記述を改訂（該当表・フロー図から deadline を消す）
- `.claude/skills/e2e-test/SKILL.md:117` 周辺の説明を更新（タイムアウト言及を削除）

### Phase 4: E2E 検証

以下 3 シナリオで **手動 + E2E スクリプト** の両方で確認:

| シナリオ | 期待結果 |
| --- | --- |
| Tachibana 構成の replay state + 正常ログイン | master download 完了後に自動で replay 開始 |
| Tachibana 構成の replay state + session 復元失敗 → ログイン画面 | info トースト表示、auto-play 放棄、ログイン後は手動で replay 実行可能 |
| Binance のみ の replay state（既存 E2E fixture） | 既存通り、auto-play が即座に発火 |

## リスクと対処

| リスク | 対処 |
| --- | --- |
| auto-play が永遠に発火しないまま残留し、後からログインした瞬間に予期せず再生開始 | `SessionRestoreResult(None)` 経路で flag を落とすため、ログイン画面を経た後は auto-play が生き残らない |
| `UpdateMetadata` が頻繁に発火するアダプタで ResolveStreams が暴走 | `refresh_waiting_panes` は Waiting 状態のペインのみ対象にする。Ready ペインには影響しない |
| 既存の Binance 系 E2E で挙動が変わる | Binance は metadata がほぼ即時に揃うため既存経路と見分けがつかない。念のため E2E で回帰確認 |
| `refresh_waiting_panes` の追加によってテスト境界が増える | `Dashboard` レベルでユニット化して、挙動を明確に分離する |

## 既知の未決事項

- `refresh_waiting_panes` を **ペインごとの `streams` に `mark_resolution_due()` を生やす方式**
  にするか、**`Dashboard::refresh_streams()` を直接呼ぶ方式**にするかは、
  Red テストを書く段階で実装コストが低い方を採用する
- SKILL.md に書いてある「タイムアウト 30s」を前提にしている E2E 手順があれば合わせて改訂する
  （現時点で該当箇所は見当たらないが、Phase 3 で grep 確認）

## 参考: 削除行のリスト（実装時チェックリスト）

- ✅ [src/replay/mod.rs:78](../../src/replay/mod.rs#L78) `pending_auto_play_deadline` フィールド
- ✅ [src/replay/mod.rs:130](../../src/replay/mod.rs#L130) Default 初期化
- ✅ [src/replay/mod.rs:149](../../src/replay/mod.rs#L149) `toggle_mode()` リセット
- ✅ [src/replay/mod.rs:612-635](../../src/replay/mod.rs#L612-L635) 関連テスト（deadline assertions を削除）
- ✅ [src/main.rs:232-233](../../src/main.rs#L232-L233) `new()` 初期化
- ✅ [src/main.rs:360-372](../../src/main.rs#L360-L372) `Tick` ハンドラ timeout
- ✅ [src/main.rs:575](../../src/main.rs#L575) 発火時の deadline リセット
- ✅ [docs/replay_header.md] ReplayState 構造体定義から `pending_auto_play_deadline` を削除
- ✅ [docs/replay_header.md] §6.2 Tick ループのタイムアウトチェック行を削除
- ✅ [docs/replay_header.md] §6.3 Auto-play フロー図をイベント駆動モデルに刷新
- ✅ [docs/replay_header.md] §6.6 Live 復帰フローから `deadline = None` を削除
- ✅ [docs/replay_header.md] §12.1 定数表の `auto-play timeout` 行を削除

## 実装完了ログ（2026-04-13）

### 追加したメソッド

| メソッド | ファイル | 役割 |
| --- | --- | --- |
| `ReplayState::on_manual_play_requested()` | `src/replay/mod.rs` | Play 押下時に `pending_auto_play = false` |
| `ReplayState::on_session_unavailable()` | `src/replay/mod.rs` | session 復元失敗時に `pending_auto_play = false` |
| `ResolvedStream::mark_resolution_due()` | `src/connector/stream.rs` | `last_attempt = None` にしてすぐ解決させる |
| `Dashboard::refresh_waiting_panes()` | `src/screen/dashboard.rs` | Waiting ペインの `mark_resolution_due()` を一括呼び出し |

### main.rs の配線

- `ReplayMessage::Play` — 先頭に `on_manual_play_requested()` 追加
- `SessionRestoreResult(None)` — `on_session_unavailable()` + info トースト追加
- `Message::Sidebar(UpdateMetadata)` — `pending_auto_play` が立っている場合に `refresh_waiting_panes()` を呼び出し

### TDD 結果

- 4 サイクル（Red→Green）× 4 メソッド
- 最終 `cargo test`: **145 passed, 0 failed**
- `cargo clippy`: 新規 warning なし

### E2E 結果（2026-04-13）

`C:/tmp/e2e_phase4.sh` + `C:/tmp/e2e_phase4_fix.sh` 実行結果:

| シナリオ | fixture | 結果 |
| --- | --- | --- |
| C: Binance のみ | `e2e_fixture_c.json`（BinanceLinear:BTCUSDT, M1, UTC, range: 2026-04-11~12） | ✅ PASS 4/4 — API 即時 Ready、~1s で Playing、time advancing、StepForward +60000ms |
| A: Tachibana + session 復元成功 | `e2e_fixture_tachibana.json`（TachibanaSpot:7203\|TOYOTA, D1, UTC） | ✅ PASS 5/5 — mode=Replay 復元、master download 完了後に auto-play 発火 |
| B: Tachibana + session 復元失敗 | ※ 現環境は有効セッションあり → シナリオ A に遷移 | unit test 143 PASS で代替確認済み |

**発見した落とし穴**（SKILL.md / fixtures.md に反映済み）:
- `"timezone": "Asia/Tokyo"` は `UserTimezone` の valid 値外 → serde parse 失敗 → `saved-state_old.json` に rename → デフォルト Live 起動
- アプリログは `stderr` ではなく `$DATA_DIR/flowsurface-current.log` に書かれる（`e2e_debug.log` は常に空）
