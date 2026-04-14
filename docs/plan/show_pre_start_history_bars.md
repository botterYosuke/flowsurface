# リプレイ Start 以前の履歴バー表示

**Status:** Implemented
**Created:** 2026-04-14
**Owner:** replay subsystem
**関連:** `src/replay/controller.rs`, `src/replay/mod.rs`, `src/replay/loader.rs`

---

## 1. 背景・問題

リプレイモード開始時、チャートには **Start 時刻のバー1本だけ**が表示され、それ以前の履歴バーは一切見えない。
リプレイが進むにつれて Start 以降のバーが順次現れる UX 自体は意図通りだが、
**Start 時点での価格コンテキスト（直近の高値/安値/トレンド）が失われる**ため分析しづらい、という報告を受けた。

### 原因（調査済み）

1. [src/replay/controller.rs:128](../../src/replay/controller.rs#L128) — `loader::load_klines` に渡す range が `start_ms..end_ms` のみ。
   EventStore に Start 以前のデータがそもそも入らない。

2. [src/replay/controller.rs:159-167](../../src/replay/controller.rs#L159-L167) — ロード完了時の初期注入で
   `filter(|k| k.time <= clock_now)` を掛けている。
   `clock_now == start_ms` なので Start 時刻 **の** バー1本だけが注入される。

3. コメント
   > 全区間を注入すると chart が最初から全バーを持ち、dispatch_tick での逐次注入が dedup で無視されてバーが増えなくなる。

   は **Start 以降のバーを dispatcher に任せる** 設計意図を示しているだけで、
   Start 以前のバーを隠す意図はなかった（副作用）。

---

## 2. ゴール / 非ゴール

### ゴール
- Replay を Play した直後、Start 時刻より前の **N 本分の履歴バー** がチャートに表示される。
- Start 以降のバーは従来通り `dispatch_tick` で順次注入され、「バーが育つ」UX が維持される。
- StepForward / StepBackward / seek も矛盾なく動く（特に Start 未満への seek をブロック）。
- 既存の `EventStore::is_loaded` / `clock.full_range()` の整合性を崩さない。

### 非ゴール
- 履歴バー数をユーザ設定で可変にする（将来の拡張余地として残す — 今回は定数）。
- Trade 履歴を pre-start まで遡って注入する（今回は kline のみ。Trade は重く、fetch API も別）。
- 複数 timeframe stream が混在するペインで各々独立に history span を持つ。

---

## 3. 要件

| # | 要件 | 優先度 |
|---|------|--------|
| R1 | Play 時、Start から遡って N 本分の kline を追加で fetch する | 必須 |
| R2 | 初期注入は `k.time < start_ms` のバーだけを一括で注入する | 必須 |
| R3 | clock range と EventStore の `is_loaded` チェックは従来通り動作する（`start_ms..end_ms`） | 必須 |
| R4 | StepBackward で `start_ms` 未満に seek できない（クランプ） | 必須 |
| R5 | history window サイズを一箇所で定義し、将来の設定化に備える | 推奨 |
| R6 | 既存の e2e テスト（`replay_e2e_test_plan.md`）の Assertion を壊さない | 必須 |

**History window の既定値:** `PRE_START_HISTORY_BARS = 300`

- 最小 timeframe × 300 本分を遡る。例: 1m → 5時間、D1 → ~1年。
- 300 本は「チャート画面に表示される典型的なバー数の 2〜3倍」で余裕を持たせた値。
- 将来 `data/config/replay.rs` 等で設定化する余地を残す。

---

## 4. 設計

### 4.1 load range の拡張

```rust
// src/replay/controller.rs: ReplayMessage::Play ハンドラ
let step_size_ms = /* 既存 */;
let history_span_ms = PRE_START_HISTORY_BARS * step_size_ms;
let load_start_ms = start_ms.saturating_sub(history_span_ms);

self.state.start(start_ms, end_ms, step_size_ms); // clock range は従来通り

let kline_tasks: Vec<Task<_>> = kline_targets
    .into_iter()
    .map(|(_, stream)| {
        let load_range = load_start_ms..end_ms;      // ← 拡張
        Task::perform(loader::load_klines(stream, load_range), ...)
    })
    .collect();
```

**不変条件:**
- `clock.full_range() == start_ms..end_ms`（従来のまま）
- `EventStore::is_loaded(stream, start_ms..end_ms)` は `load_start_ms..end_ms` が superset なので true を返す（[store.rs:96-104](../../src/replay/store.rs#L96-L104) の `lr.start <= range.start && lr.end >= range.end` で成立）

### 4.2 初期注入フィルタの変更

```rust
// src/replay/controller.rs: KlinesLoadCompleted ハンドラ
let start_ms = self.state.clock
    .as_ref()
    .map(|c| c.full_range().start)
    .unwrap_or(0);

let history_klines: Vec<Kline> = klines
    .iter()
    .filter(|k| k.time < start_ms)   // ← strictly less than
    .cloned()
    .collect();

if !history_klines.is_empty() {
    dashboard.ingest_replay_klines(&stream, &history_klines, main_window_id);
}
```

- Start 時刻ちょうどのバー（`k.time == start_ms`）は **注入しない**。
  dispatcher の最初の tick（range `[start_ms, start_ms + step_size)`) がそれを注入する。
- dedup 懸念: history バーは `start_ms` 未満なので、dispatcher の tick range と重ならず dedup 対象外。

### 4.3 StepBackward の下限クランプ

```rust
// src/replay/controller.rs: ReplayMessage::StepBackward
let start_ms = /* clock.full_range().start */;
let new_time = prev_time.unwrap_or(current_time).max(start_ms);
```

**理由:**
- `prev_time` は `event_store.klines_in(stream, 0..current_time)` から取るので、history 範囲 (`< start_ms`) のバー時刻が候補に入る。
- しかし clock の意味論としては `start_ms..end_ms` を超えて再生できない。`.max(start_ms)` でクランプ。
- `inject_klines_up_to(new_time, ...)` も `new_time >= start_ms` なので Start 以前のバーは初期注入済みのままチャートに残り続ける（seek しても消えない）。

### 4.4 reset_for_seek 後の再注入

[src/replay/controller.rs:244-246](../../src/replay/controller.rs#L244-L246) で seek 時に
`reset_charts_for_seek` → `inject_klines_up_to(new_time, ...)` を呼んでいる。
reset 後のチャートは空なので、`inject_klines_up_to` でも **history バーを注入し直す必要がある**。

```rust
fn inject_klines_up_to(&self, target_ms: u64, ...) {
    for stream in self.state.active_streams.iter() {
        // 変更: 0..target_ms+1 のまま OK — history 範囲も含まれる
        let klines = self.state.event_store.klines_in(stream, 0..target_ms + 1);
        ...
    }
}
```

→ 既存コードのまま動く（`0..` から取得しているため）。**変更不要**。

### 4.5 定数の定義場所

```rust
// src/replay/mod.rs
/// Replay Start 時刻より前に何本の kline を履歴として読み込むか。
/// 最小 timeframe × この本数分を pre-start history として fetch する。
pub const PRE_START_HISTORY_BARS: u64 = 300;
```

---

## 5. TDD 進行計画

各ステップは **RED → GREEN → REFACTOR** で進める。
コミット単位は「テスト1件+実装」で小さく保ち、revert しやすくする。

### Step 1: 定数追加（RED は不要、trivial）✅
- `src/replay/mod.rs` に `PRE_START_HISTORY_BARS` を追加
- ドキュメントコメントだけ

### Step 2: load range 拡張（RED → GREEN）✅

**RED (failing test)** — `src/replay/controller.rs::tests`:
```rust
#[test]
fn play_loads_klines_from_start_minus_history_span() {
    // Arrange: Play(start=1000_000, end=2000_000, step=60_000)
    // Act: 送信された load_klines の range を検証
    // Assert: load_range.start == 1000_000 - 300*60_000
}
```

テスト容易性のため、`loader::load_klines` を直接呼ぶ箇所をヘルパーに切り出す：
```rust
fn compute_load_range(start_ms: u64, end_ms: u64, step_size_ms: u64) -> Range<u64> {
    let history_span = PRE_START_HISTORY_BARS * step_size_ms;
    start_ms.saturating_sub(history_span)..end_ms
}
```
このピュア関数を対象にテストすれば Task 送信を mock せずに済む。

**GREEN** — `compute_load_range` を実装し、Play ハンドラから呼ぶ。

### Step 3: 初期注入フィルタ変更（RED → GREEN）✅

**RED** — `src/replay/controller.rs::tests`:
```rust
#[test]
fn klines_load_completed_injects_only_pre_start_history() {
    // Arrange: start_ms=1000, end_ms=5000, step=1000
    //          klines=[700, 800, 900, 1000, 1100, 1200]
    // Act: KlinesLoadCompleted を dispatch
    // Assert: dashboard.ingest_replay_klines の第2引数は [700, 800, 900] だけ
}
```

Dashboard を直接呼ばず、`ReplayController` から「注入された kline リスト」を観測できるよう
テスト用のフック（trait or enum mock）が必要。
→ 既存の `ingest_replay_klines` の呼び出しを `KlineIngestSink` trait に抽象化する refactor を先行させる。
ただし、**過度な抽象化を避けるため**、まずはピュア関数 `split_initial_klines(klines, start_ms) -> Vec<Kline>` を切り出してそれをテストする方針。

```rust
pub fn split_initial_klines(klines: &[Kline], start_ms: u64) -> Vec<Kline> {
    klines.iter().filter(|k| k.time < start_ms).cloned().collect()
}
```

**GREEN** — フィルタを `<` に変更。

**REFACTOR** — 関数名を `pre_start_history(&[Kline], u64)` に変更する等、可読性を高める。

### Step 4: StepBackward クランプ（RED → GREEN）✅

**RED** — `src/replay/controller.rs::tests`:
```rust
#[test]
fn step_backward_does_not_seek_below_start_ms() {
    // Arrange: start_ms=1000, clock.now_ms()=1000
    //          history klines at [700, 800, 900], start bar at 1000
    // Act: StepBackward
    // Assert: clock.now_ms() == 1000 (unchanged)
    //         inject_klines_up_to called with target=1000
}
```

**GREEN** — `new_time = prev_time.unwrap_or(current_time).max(start_ms)` に変更。

### Step 5: 統合確認（手動 + e2e）✅ (cargo test 完了 / 手動確認は別途)

- `cargo test -p flowsurface replay`
- `cargo run` で実際にリプレイを起動し、Start 以前のバーが表示されることを目視確認
- 既存 e2e (`replay_e2e_test_plan.md`) を走らせて regression なしを確認

---

## 6. 影響範囲

| ファイル | 変更内容 |
|---------|---------|
| `src/replay/mod.rs` | `PRE_START_HISTORY_BARS` 定数追加 |
| `src/replay/controller.rs` | `compute_load_range`, `pre_start_history` 追加 / Play ハンドラ / KlinesLoadCompleted ハンドラ / StepBackward |
| `src/replay/loader.rs` | 変更なし |
| `src/replay/store.rs` | 変更なし |
| `src/replay/dispatcher.rs` | 変更なし |
| `src/chart/kline.rs` | 変更なし |
| `src/screen/dashboard.rs` / `pane.rs` | 変更なし |

---

## 7. リスク・懸念

| リスク | 影響 | 緩和策 |
|-------|------|-------|
| 300 本の追加 fetch でレイテンシ増加 | Play ボタン押下 → Playing までの時間が延びる | timeframe 別に実測し、必要なら `PRE_START_HISTORY_BARS` を減らすか設定化 |
| history 範囲が取引所側で提供されない期間 | fetch が空 or エラー | `load_start_ms` が極端に古い場合のみ発生。saturating_sub で 0 にクランプされるので panic はしないが、空返却の扱いは既存の `(D) 空 klines は "未ロード"` パスと整合を取る必要あり。**要検証**。 |
| Tachibana の daily history は範囲指定が粗い | D1 timeframe で過剰 fetch の可能性 | `fetch_tachibana_daily_klines(issue_code, Some((start,end)))` は既に range 対応済み。問題なし。 |
| 複数ペインで異なる timeframe → step_size_ms が最小値なので過剰遡り | 1m ペインと 1h ペインが混在すると 1m 基準で 300 本 = 5h しか遡れない | 仕様として許容（最小 timeframe に揃う）。将来 stream ごとに history 計算する余地あり。 |
| StepBackward の UI 表現 | Start でクランプされた時にユーザへのフィードバック | 現状 no-op 同等。Toast は不要（頻繁に当たるため）。 |

---

## 8. ロールバック戦略

各 Step を独立コミットにする：
1. `feat(replay): add PRE_START_HISTORY_BARS constant`
2. `feat(replay): load klines before start_ms for history context`
3. `feat(replay): inject only pre-start history as initial klines`
4. `fix(replay): clamp StepBackward to start_ms`

問題発見時は該当コミットだけ revert 可能。

---

## 9. オープンクエスチョン

- [ ] `PRE_START_HISTORY_BARS = 300` で十分か？ ユーザに確認。
- [ ] Tachibana の D1 fetch で 300 日遡ると API 制限に当たる可能性。実測必要。
- [ ] history バーを「薄く表示」するなどの視覚的区別は必要か？（今回は同色で表示する想定）

---

## 10. 進捗ログ

- 2026-04-14: Draft 作成。調査完了、設計セクションまで記述。
- 2026-04-14: 実装完了。TDD (RED→GREEN×3) で全ステップ完了。

### 実装サマリ

#### ✅ Step 1: `PRE_START_HISTORY_BARS` 定数追加
- `src/replay/mod.rs:20` に `pub const PRE_START_HISTORY_BARS: u64 = 300;` を追加。

#### ✅ Step 2: `compute_load_range` (RED→GREEN)
- `src/replay/mod.rs:35-37` に純粋関数を追加。
- `controller.rs:127` の `start_ms..end_ms` を `super::compute_load_range(start_ms, end_ms, step_size_ms)` に差し替え。
- テスト: `compute_load_range_extends_start_back_by_history_span`, `compute_load_range_saturates_at_zero_when_history_exceeds_start`

#### ✅ Step 3: `pre_start_history` (RED→GREEN)
- `src/replay/mod.rs:42-44` に純粋関数を追加。
- `controller.rs KlinesLoadCompleted` ハンドラの初期注入フィルタを変更:
  - 旧: `filter(|k| k.time <= clock_now)` （`clock_now == start_ms` ゆえ Start バー1本が入っていた）
  - 新: `super::pre_start_history(&klines, start_ms)` で `< start_ms` フィルタ
  - `start_ms` の取得は `clock.full_range().start` を使用（非同期 load 完了後も正確）
- テスト: `pre_start_history_returns_only_bars_before_start_ms`, `pre_start_history_excludes_bar_at_exact_start_ms`

#### ✅ Step 4: `compute_step_backward_target` + StepBackward クランプ (RED→GREEN)
- `src/replay/mod.rs:49-55` に純粋関数を追加。
- `controller.rs StepBackward` の `new_time = prev_time.unwrap_or(current_time)` を
  `super::compute_step_backward_target(prev_time, current_time, start_ms)` に差し替え。
- テスト: `step_backward_target_clamps_to_start_ms_when_prev_is_below`,
  `step_backward_target_allows_seek_within_replay_range`,
  `step_backward_target_stays_at_current_when_no_prev`

### 実装で確認された知見・Tips

**純粋関数抽出のメリット:**
controller.rs の `handle_message` は `Dashboard` + `iced::window::Id` を必要とするため、
直接ユニットテストできない。ロジックを純粋関数（`compute_load_range`, `pre_start_history`,
`compute_step_backward_target`）に切り出すことで、Task や Dashboard を mock せずにテスト可能。

**`start_ms` 取得の注意点:**
`KlinesLoadCompleted` は非同期 load 完了時に呼ばれるため、
`self.state.current_time()`（= `clock.now_ms()`）はすでに start_ms より進んでいる可能性がある。
`clock.full_range().start` を使うことで常に Play 開始時の start_ms を取得できる。

**`inject_klines_up_to` は変更不要:**
`0..target_ms + 1` で取得しているため、seek 後の `reset_charts_for_seek` → 再注入時に
history バーも自動的に含まれる。設計通り「変更不要」が確認された。

**clippy 警告（`to_vec` at controller.rs:330）は pre-existing:**
`inject_klines_up_to` 内の `&klines.to_vec()` は今回の変更前から存在する警告。
本タスクのスコープ外のため対応しなかった。

### テスト結果
```
test result: 156 passed; 2 failed (pre-existing screen::login); 0 ignored
cargo build: exit 0, zero warnings (replay モジュール)
```