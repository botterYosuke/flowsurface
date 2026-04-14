# リプレイ機能 再設計プラン

**作成日**: 2026-04-13
**対象**: `src/replay.rs`, `src/chart/kline.rs`, `src/screen/dashboard.rs`, `src/screen/dashboard/pane.rs`, `src/main.rs`
**状態**: 設計提案（未着手）
**前提ドキュメント**: [replay_header.md](../replay_header.md), [pane_crud_api.md](pane_crud_api.md), [fix_replay_pending_deadlock.md](fix_replay_pending_deadlock.md)

## 目的

現状リプレイ実装は機能要件を満たしているが、設計が段階的拡張で形成されたため複雑度が高く、
バグ修正 (`fix_replay_pending_deadlock.md`) や E2E 検証 (`pane_crud_api.md`) に必要な労力が肥大化している。
本プランでは**現状コードを踏襲せず**、「チャートをリプレイする」という機能本質に即した最小構成を再設計する。

ゴールは次の 4 点：

1. **時間管理を単一の真実源に集約**する（`virtual_elapsed_ms` + `current_time` の二重管理を廃止）
2. **チャートを Live / Replay 非対称から解放**する（`replay_kline_buffer` / `enable_replay_mode` を廃止）
3. **mid-replay CRUD をイベント駆動にする**（`SyncReplayBuffers` ceremony を廃止）
4. **FireStatus の 3 値分岐を廃止**する（Ready/Pending/Terminal の概念自体を不要にする）

## 現状設計の問題点

### P1. 時刻が二重管理されている

[src/replay.rs:50-53](../../src/replay.rs#L50) の `virtual_elapsed_ms` はフレーム単位の連続累積値、
[src/replay.rs:42](../../src/replay.rs#L42) の `PlaybackState::current_time` はバー境界でのみ更新される離散値。
両者の整合は [src/replay.rs:434-490](../../src/replay.rs#L434) `process_tick` 内で threshold 比較と減算で維持され、
「どちらが真の現在時刻か」がコードを読むまで分からない。

### P2. FireStatus の 3 値が責務漏洩している

[src/replay.rs:374-378](../../src/replay.rs#L374) の `FireStatus::{Ready, Pending, Terminal}` はチャート側状態に依存する。
[src/screen/dashboard.rs:1134-1168](../../src/screen/dashboard.rs#L1134) で Dashboard が各ペインの
`replay_kline_chart_ready()` / `replay_next_kline_time()` を走査して集約するが、
「kline バッファがバックフィル中」という**チャート内部の事情**が replay clock の状態遷移を決めている。
結果として clock 側と chart 側の整合が壊れると pending deadlock (`fix_replay_pending_deadlock.md`) が発生する。

### P3. チャートが Live / Replay で非対称

[src/chart/kline.rs:163-191](../../src/chart/kline.rs#L163) `ReplayKlineBuffer` と
[src/chart/kline.rs:345-397](../../src/chart/kline.rs#L345) `enable_replay_mode()` により、
チャートは `replay_kline_buffer` の有無で挙動を変える。
[src/chart/kline.rs:779-805](../../src/chart/kline.rs#L779) `insert_hist_klines` もモード分岐しており、
ライブ用フェッチが遅延完了するだけでバッファが吹き飛ぶバグを招いた（修正済み: `fix_replay_pending_deadlock.md`）。

### P4. mid-replay CRUD が ceremony 化している

[src/main.rs:1159-1234](../../src/main.rs#L1159) `SyncReplayBuffers` は
「mid-replay で set-ticker / split / set-timeframe した後に手動で呼ぶ差分同期」であり、
呼び忘れると buffer が更新されない。
refresh_streams の末尾で chain される運用だが、ペイン API 追加の度にリマインダーが必要な設計は保守性が低い。

### P5. D1 と M1 で throttling ロジックが非対称

[src/replay.rs:457-462](../../src/replay.rs#L457) の `COARSE_CUTOFF_MS = 3,600,000ms` / `COARSE_BAR_MS = 1,000ms` は
D1 で 1 バー/秒に throttle するための heuristic だが、統一 Tick ループの内部に埋め込まれている。
「表示上のペーシング」と「時刻進行のロジック」が絡み合い、テストで境界値を大量に確認する必要がある。

### P6. Trade バッファ per-stream cursor

[src/replay.rs:95-278](../../src/replay.rs#L95) `TradeBuffer` は `cursor` 位置で drain 済み範囲を管理する。
[src/replay.rs:95-120](../../src/replay.rs#L95) `advance_cursor_to` が後方バックフィル用に存在し、
クエリインタフェース（時刻範囲指定）ではなく状態機械（cursor）として設計されているため、
ランダムアクセス（Step Backward 時の復元）が難しい。

---

## 新設計: Virtual Clock + Event Store + Dispatcher

### コア概念

リプレイを **「過去の時刻を再現する単方向 wall-clock」** と
**「(stream, time) で引ける read-only 履歴ストア」** の 2 層に分解する。
両者の間を繋ぐ **Dispatcher** が各フレームで
「前回フレームから今回フレームまでに『過ぎ去った』イベント」を抽出し、
**Live と同じ経路でチャートに注入する**。

```
┌─────────────────┐    elapsed_ms × speed    ┌──────────────────┐
│  VirtualClock   │ ───────────────────────▶ │   Dispatcher     │
│  (state = now)  │                          │                  │
└─────────────────┘                          │  query store     │
                                             │  events in       │
┌─────────────────┐                          │  (prev_now, now] │
│   EventStore    │ ◀──── stream,range ────  │                  │
│ (historical DB) │ ────── Vec<Event> ─────▶ │  emit via        │
└─────────────────┘                          │  same channel    │
                                             │  as WebSocket    │
                                             └─────────┬────────┘
                                                       ▼
                                             ┌──────────────────┐
                                             │  KlineChart      │
                                             │  (replay-naive)  │
                                             └──────────────────┘
```

### 設計原則

1. **単一時刻**: `VirtualClock::now_ms() -> u64` が唯一の現在時刻。`virtual_elapsed_ms` / `current_time` の両方を廃止。
2. **イベント駆動**: Dispatcher は各フレームで「`(prev_now, now]` に含まれるイベント全て」をチャートに送るだけ。`FireStatus` は存在しない。
3. **チャート非対称の廃止**: KlineChart は `replay_kline_buffer` を持たず、WebSocket と同じ API (`ingest_trades` / `update_latest_kline`) でデータを受け取る。
4. **事前ロード**: replay range 全体の履歴データは Play ボタン押下時に EventStore に bulk load する。ロード完了前は Clock を進めない（= Play が「準備中」状態になる）。
5. **CRUD は Store への subscribe**: mid-replay で新ペイン追加時は新 stream 分のデータを Store にロードするだけ。Dispatcher は次フレームで自動的に新 stream を query する。

### 型構造

#### `VirtualClock`

```rust
/// リプレイ再生中の単方向仮想時刻。
/// wall elapsed × speed を積算するだけ。pause 時は `anchor_wall` を update せず holds。
pub struct VirtualClock {
    /// 仮想時刻 (Unix ms)。Play 開始時 = replay.start_ms、Pause 時は固定。
    now_ms: u64,
    /// 最後に now_ms を更新した実時刻。Pause 中は None。
    anchor_wall: Option<Instant>,
    /// 再生速度 (1.0 = 等倍)。
    speed: f32,
    /// 再生状態。
    status: ClockStatus,
}

enum ClockStatus {
    Paused,
    Playing,
    /// EventStore が range loading 中。now_ms を進めない。
    Waiting,
}

impl VirtualClock {
    /// 各フレームで呼ぶ。wall elapsed を仮想時刻に変換して now_ms を進め、
    /// (prev_now, current_now) の範囲を返す。
    pub fn advance(&mut self, wall_now: Instant) -> Range<u64> { ... }

    pub fn play(&mut self, wall_now: Instant) { ... }
    pub fn pause(&mut self) { ... }
    pub fn seek(&mut self, target_ms: u64) { ... } // step forward/back で使用
    pub fn set_speed(&mut self, speed: f32) { ... }

    /// Waiting 状態に落とす（Store 未 load が検出されたとき）。
    /// anchor_wall は None にリセット、Playing へ戻るときに再設定。
    /// **冪等**: 既に Waiting なら何もしない。毎フレームの dispatch_tick から
    /// 安全に呼び出せる。
    pub fn set_waiting(&mut self) { ... }

    /// Waiting → Playing へ復帰する。EventStore::ingest_loaded 完了時に呼ぶ。
    /// 新しい wall_now を anchor にするため、待機中の実時間経過ぶんは
    /// 仮想時間に反映されない（= データ待ちで空転する秒を飛ばす）。
    pub fn resume_from_waiting(&mut self, wall_now: Instant) { ... }
}
```

**ポイント**: `virtual_elapsed_ms` の累積は `advance` 内部に閉じ、外部には `Range<u64>` のみ出す。
状態機械は `ClockStatus` の 3 値のみで、「バー境界」「COARSE cutoff」といった概念を持たない。

#### `EventStore`

```rust
/// (stream, time) で引ける read-only 履歴データストア。
/// Range 単位で bulk load される。
pub struct EventStore {
    klines: HashMap<StreamKind, SortedVec<Kline>>,
    trades: HashMap<StreamKind, SortedVec<Trade>>,
    /// 各 stream で既に load 済みの時刻範囲。
    loaded_ranges: HashMap<StreamKind, Vec<Range<u64>>>,
}

impl EventStore {
    /// 指定範囲の trades を返す（binary search）。cursor なし。
    pub fn trades_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Trade];
    pub fn klines_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Kline];

    /// range が loaded かどうかを返す。未 load ならクロックを Waiting にする。
    pub fn is_loaded(&self, stream: &StreamKind, range: Range<u64>) -> bool;

    /// CRUD で新 stream が追加されたときに呼ぶ。
    /// Task::perform で fetch → Self::ingest_loaded を発火。
    pub fn request_load(&mut self, stream: StreamKind, range: Range<u64>) -> Task<Message>;

    pub fn ingest_loaded(&mut self, stream: StreamKind, range: Range<u64>, data: LoadedData);

    /// stream がどのペインからも参照されなくなったときに呼ぶ。
    pub fn drop_stream(&mut self, stream: &StreamKind);
}
```

**ポイント**: cursor を廃止。クエリ API で時刻範囲を受け取り、binary search で slice を返す。
Step Backward は Store 側では自然に扱えるが、チャート側の再構築が必要（下記「Step Backward フロー」参照）。

#### `dispatch_tick` (free function)

Dispatcher はフィールドを持たないため struct ではなく module-level free function にする。

```rust
/// 各 Tick で呼ばれ、clock を進めて emit すべきイベントを集めて返す。
/// VirtualClock と EventStore の橋渡しをするステートレスなロジック。
pub fn dispatch_tick(
    clock: &mut VirtualClock,
    store: &EventStore,
    active_streams: &HashSet<StreamKind>,
    wall_now: Instant,
) -> DispatchResult;

pub struct DispatchResult {
    pub current_time: u64,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    /// true なら replay 終端に到達、clock は Paused へ。
    pub reached_end: bool,
}
```

**ポイント**: FireStatus が消滅。`dispatch_tick` は clock が進めば進んだぶんのイベントを返すだけで、
「バックフィル中」「pending」といった概念を持たない（待機は Clock 側の `Waiting` で表現）。

#### チャート側の変更

```rust
// KlineChart::replay_kline_buffer, enable_replay_mode, replay_advance_klines, is_replay_mode
// → 全て削除

// update_latest_kline / insert_hist_klines は Live / Replay で完全に同一経路
pub fn ingest_historical_klines(&mut self, klines: &[Kline]) { ... }
pub fn ingest_trades(&mut self, trades: &[Trade]) { ... } // Live と同じ

// Step Backward / Seek 時のチャート全リセット。
// 呼び出し後に Dashboard が [replay.start_ms, seek_target] の履歴を再 ingest する。
pub fn reset_for_seek(&mut self) { ... }
```

**ポイント**: チャートは自分がリプレイ中かを一切知らない。WebSocket と Dispatcher が emit するイベントが同じ型なので、ingest 経路も同一。

#### Step Backward / Seek フロー

`Clock::seek(t_past)` で巻き戻す場合、チャートは既に `t_prev > t_past` までの kline/trade を描画済みであり、
単に Clock の `now_ms` を戻すだけでは画面状態と仮想時刻が不整合になる。
このため Dashboard は seek リクエスト時に全 active pane のチャートを**一度リセットしてから**
`[replay.start_ms, t_past]` の履歴を Store から引き直して再 ingest する。

```text
Message::Replay(StepBackward) / SeekTo(t_past)
  ↓
for each active pane:
    chart.reset_for_seek()                                 // 描画済み状態をクリア
    for each stream in pane.streams:
        let klines = store.klines_in(stream, start..t_past).to_vec();
        let trades = store.trades_in(stream, start..t_past).to_vec();
        chart.ingest_historical_klines(&klines);           // チャートを t_past 時点に再構築
        chart.ingest_trades(&trades);
  ↓
clock.seek(t_past)                                         // Clock を巻き戻し
clock.play(wall_now)                                       // 必要なら再開
```

**コスト見積もり**:
- `klines_in` / `trades_in` は binary search で slice を返すだけなので `O(log N)`。
- `ingest_historical_klines` / `ingest_trades` は既存の live 経路と同じコストで、通常 replay range（Binance 数時間）では 10ms オーダーと見込まれる。
- 頻繁な step backward（数秒に 1 回）でも UI 操作として許容範囲。
- 非常に長い replay range で問題が出た場合は、Store 側に「seek target 近辺のみ再 ingest」の window 最適化を追加する余地がある（R3 の範囲外）。

**旧設計との比較**:
- 旧設計では `TradeBuffer::advance_cursor_to` が cursor を遡上させるだけで、チャート側の巻き戻しは別経路だった。
- 新設計では「チャート状態 = Store からの全再生」という single source of truth が成立するため、
  巻き戻し処理が「reset → 再 ingest」の 2 ステップで閉じる。cursor 復元ロジックや「未処理 trade の破棄」といった特殊ケースが消える。

---

## モジュール構成

```
src/
├── replay/
│   ├── mod.rs              # 公開 API (ReplayState, ReplayMessage)
│   ├── clock.rs            # VirtualClock + ClockStatus
│   ├── store.rs            # EventStore + SortedVec
│   ├── dispatcher.rs       # dispatch_tick (free function)
│   ├── loader.rs           # 履歴データ fetch (現 fetch_trades_batched をここに移動)
│   └── ui.rs               # Replay header widget (現 replay.rs の UI 部分)
└── chart/kline.rs          # replay_* 系フィールド/メソッド全削除
```

現在の [src/replay.rs](../../src/replay.rs) 単一ファイル (≈2000 行) を機能単位で分割する。

## 移行計画（Phase 分割）

現状コードを段階的に差し替える。Phase 間で `cargo check` / 既存テストが壊れないことを必須とする。
**feature flag は使わない**（`#[cfg]` ノイズが最終コードに残るため）。
R4 で旧経路を一括で新経路に差し替え、不具合時は `git revert` で戻す。

| Phase | 内容 | 検証 |
|:-:|---|---|
| **R1** ✅ | 新モジュール `src/replay/` スケルトン + `VirtualClock` + `EventStore` + `dispatch_tick` を 1 PR で追加。既存コードに一切触れない新規ファイルのみ。各コンポーネントの単体テストを同時に追加。 | `cargo test replay::{clock,store,dispatcher}` |
| **R2** ✅ | Binance 向け履歴 loader (`replay/loader.rs`) 実装。現 `fetch_trades_batched` / kline fetch ロジックを EventStore に bulk load する形に書き直す。既存コードは未変更のため Live も Replay も現状動作のまま。 | loader 単体テスト: bulk load が完了すると EventStore::is_loaded が true を返す |
| **R3** ✅ | `main.rs::Tick` で旧 `process_tick` 経路を新 `dispatch_tick` 経路に差し替え。KlineChart から `replay_kline_buffer` / `enable_replay_mode` / `replay_advance` / `is_replay_mode` を削除し、`update_latest_kline` / `insert_hist_klines` を Live 経路と統一。旧 `src/replay.rs` の `PlaybackState` / `FireStatus` / `process_tick` / `TradeBuffer` / `ReplayKlineBuffer` を削除。 | 既存 E2E: §6.2 #1 / #3 / #4（Binance M1 等の基本リプレイ 21 件）、`cargo test` |
| **R4** | mid-replay CRUD: `SyncReplayBuffers` メッセージを削除し、set-ticker / split / set-timeframe / close 各ハンドラで `EventStore::request_load` / `drop_stream` を直接呼ぶ形に。`Message::ReplayLoadCompleted` を追加し Waiting 復帰フローを配線。 | E2E: §6.2 #2, #5, #6, #7, #8（mid-replay CRUD 系） |
| **R5** | Tachibana D1 経路: `VirtualClock::enable_bar_step_mode` を実装。Dashboard で active pane の最大 timeframe を判定して `>= D1` なら呼び出す。 | E2E: §6.2 #2 (Tachibana D1) |
| **R6** | heatmap-only リプレイ: FireStatus / linear advance fallback が廃止されたため、新経路で自動動作することを確認。`pane_crud_api.md` Phase T 残課題 G9 に対応。 | E2E: heatmap-only mid-replay |
| **R7** | cleanup: 残った旧コード削除、import 整理、ドキュメント更新 (`replay_header.md` の記述を新設計に合わせる)。全 236 E2E + ユニットテストが PASS することを確認。 | `cargo test` + 全 E2E green |

### ロールバック戦略

各 Phase を独立した PR にし、不具合検出時は `git revert` で直前 Phase に戻す。
R3 が最大の差分（旧経路削除）のため、R3 の PR だけは特に入念なレビューと手動検証を行う。

### 旧設計との並行動作を行わない理由

feature flag `new_replay` で旧経路を残す選択肢もあるが、次の理由で採用しない：

1. **`#[cfg]` ノイズ**: 旧 `src/replay.rs` と新 `src/replay/` が両方ビルドされる期間、`main.rs::Tick` / Dashboard / KlineChart に `#[cfg(feature = "new_replay")]` 分岐が入る
2. **二重メンテ**: Phase R3-R7 中に既存バグが報告された場合、新旧両方に修正が必要になる
3. **git は十分なロールバック手段**: R3 の PR を revert すれば 1 コマンドで旧経路に戻せる

---

## Phase 別の詳細

### R1: VirtualClock

#### データ構造

```rust
pub struct VirtualClock {
    now_ms: u64,
    anchor_wall: Option<Instant>,
    speed: f32,
    status: ClockStatus,
    range: Range<u64>, // replay start..end
}
```

#### advance の擬似コード

```
fn advance(&mut self, wall_now: Instant) -> Range<u64> {
    if self.status != Playing { return self.now_ms..self.now_ms; }
    let anchor = self.anchor_wall.expect("Playing without anchor");
    let wall_elapsed = wall_now - anchor;
    let virtual_delta = (wall_elapsed.as_secs_f64() * self.speed as f64 * 1000.0) as u64;
    let prev = self.now_ms;
    let next = prev.saturating_add(virtual_delta).min(self.range.end);
    self.now_ms = next;
    self.anchor_wall = Some(wall_now);
    if next >= self.range.end { self.status = Paused; }
    prev..next
}
```

#### 廃止される概念

- `virtual_elapsed_ms` (累積値) — `advance` の呼び出し間で 0 にリセットされる
- `last_tick` — `anchor_wall` に統合
- `comparison_threshold` / COARSE_CUTOFF_MS — 不要

#### ✅ R1 実装メモ (2026-04-13)

- `src/replay.rs` → `src/replay/mod.rs` に移動し、先頭に `pub mod clock; pub mod store; pub mod dispatcher;` を追加。既存コードへの変更はこれのみ。
- `replay/clock.rs`: `VirtualClock` + `ClockStatus` 実装。`bar_step_mode` も R5 用に先行実装（フィールド追加のみ、R5 で配線する）。
- `replay/store.rs`: `EventStore` + `SortedVec<T>` 実装。`partition_point` で binary search range 切り出し。`dedup_by_key` で重複排除。
- `replay/dispatcher.rs`: `dispatch_tick` free function + `DispatchResult` 実装。
- テスト: 合計 32 件追加（clock: 19件、store: 7件、dispatcher: 5件）。全 190 件 green。
- 注意点: `exchange::Volume::default()` は存在しない。`Volume::empty_total()` を使う。
- 注意点: `TickerInfo::new(ticker, min_ticksize_f32, min_qty_f32, contract_size_option_f32)` のシグネチャ（型変換は内部で行う）。

### R2: EventStore

#### 保存形式

`SortedVec<T>` は内部的に `Vec<T>` で、挿入時に `sort_by_key` で時刻順を維持、
クエリは `binary_search_by_key` で range 切り出し。メモリ効率と実装単純性のトレードオフで `Vec` を選択。

#### ✅ R2 実装メモ (2026-04-13)

- `replay/loader.rs` 作成。`load_klines(stream, range) -> Result<KlineLoadResult, String>` async fn を実装。
- klines は `Task::perform(load_klines(...), ...)` で一括 fetch する設計。
- trades は既存 `build_trades_backfill_task` の sip パターンを踏襲（R4 で EventStore 統合）。
- 単体テスト 5 件: `ingest_loaded → is_loaded / trades_in / klines_in` の振る舞いを検証。全 195 件 green。
- 注意点: `range_slice` は `partition_point(t < range.end)` なので **range.end 自体は含まない**（half-open range）。テスト設計時に注意。

#### 複数 range の管理

`loaded_ranges: Vec<Range<u64>>` は overlap や gap を検出して merge するのが望ましいが、
初期実装は「range ごとに独立フェッチ、重複を許容、クエリ時に union して slice」で始める。
後で最適化する。

#### CRUD 時の追加 load

`active_streams` は **Dashboard が所有する** `HashSet<StreamKind>` であり、
ペインの追加・削除・ticker 変更に応じて Dashboard が更新する。
Dispatcher は受け取るだけで自身は保持しない。

```
// Message::PaneSetTicker(pane_id, new_ticker)
// → 旧: SyncReplayBuffers チェーン
// → 新:
//    1. dashboard.active_streams.insert(new_stream)    // Dashboard が管理
//    2. dashboard.active_streams.remove(old_stream)    // 旧 stream が orphan なら削除
//    3. Task::batch [
//         EventStore::request_load(new_stream, replay.range),
//         store.drop_stream(old_stream) if orphan,
//       ]
//    4. 次フレームの Message::Tick で dispatch_tick(active_streams, ...)
//       が呼ばれ、新 stream の events が自動的に emit される
```

#### ✅ R3 実装メモ (2026-04-13)

- `src/chart/kline.rs`: `replay_kline_buffer` フィールド削除、`enable_replay_mode` / `disable_replay_mode` / `is_replay_mode` / `replay_buffer_ready` / `replay_buffer_cursor` / `replay_buffer_len` / `replay_next_kline_time` / `replay_prev_kline_time` / `replay_advance` メソッド削除。`insert_hist_klines` の replay 分岐を削除（Live 経路のみに統一）。`set_basis` の replay バッファクリア処理を削除。`ingest_historical_klines` / `reset_for_seek` を新規追加。旧 replay テスト群を削除し `ingest_historical_klines`・`reset_for_seek` の新規テストを追加。
- `src/screen/dashboard/pane.rs`: `enable_replay_mode_if_needed` / `rebuild_content_for_step_backward` / `replay_kline_chart_ready` / `replay_buffer_cursor` / `replay_buffer_len` / `replay_next_kline_time` / `replay_prev_kline_time` / `replay_advance_klines` を削除。`rebuild_content_for_replay` を追加（内部は `rebuild_content(true)`）。`ingest_replay_klines` / `reset_for_seek` を新規追加。`insert_hist_klines` の `is_replay_mode()` ガードを削除。
- `src/screen/dashboard.rs`: `replay_advance_klines` / `replay_next_kline_time` / `replay_prev_kline_time` / `fire_status` / `collect_new_replay_klines` / `rebuild_for_step_backward` を削除。`ingest_replay_klines(stream, klines, main_window)` を新規追加（stream で対応ペインを検索して注入）。
- `src/replay/mod.rs`: `ReplayState` を `clock: Option<VirtualClock>` / `event_store: EventStore` / `active_streams: HashSet<StreamKind>` ベースに再定義。`PlaybackState` / `PlaybackStatus` / `FireStatus` / `process_tick` / `TradeBuffer` / `TradeStreamDiff` / `last_tick` / `virtual_elapsed_ms` をすべて削除。`on_klines_loaded` / `on_trades_loaded` / `try_resume_from_waiting` / `start` / `resume_from_waiting` / `is_playing` / `is_paused` / `is_loading` / `current_time` / `cycle_speed` / `speed_label` を新規追加。
- `src/main.rs`: `Tick` ハンドラを `dispatch_tick` 経路に差し替え。`Play` ハンドラを `replay.start()` + `Task::perform(load_klines)` に差し替え。`KlinesLoadCompleted` ハンドラを追加。`Resume` / `Pause` / `CycleSpeed` / `StepForward` / `StepBackward` を新 API に更新。旧互換メッセージ (`DataLoaded`, `TradesBatchReceived`, `TradesFetchCompleted`, `SyncReplayBuffers`) を no-op スタブ化。View コードの `playback.status == PlaybackStatus::*` 参照を `replay.is_playing()` / `is_paused()` / `is_loading()` に置き換え。
- 設計偏差: `TickAggr::tick_interval` は存在せず `interval` が正フィールド名。`klines_in` は半開区間 `Range<u64>` のみ受け付け、`0..=t` の代わりに `0..t+1` を使用。旧互換 `build_kline_backfill_task` / `build_trades_backfill_task` は R4 まで残置（dead_code 警告あり）。

### R3: Dispatcher

#### tick の擬似コード

設計原則 #4「Play 押下時に replay range 全体を bulk load する」により、
Dispatcher は常に **full replay range** が loaded されているかだけを確認すれば良い。
`estimated_delta` 等の予測は不要。

```
fn dispatch_tick(clock, store, active_streams, wall_now) -> DispatchResult {
    // 1. 全 active_streams の full replay range が loaded か確認
    for stream in active_streams {
        if !store.is_loaded(stream, clock.full_range()) {
            clock.set_waiting();
            return DispatchResult::empty(clock.now_ms);
        }
    }

    // 2. clock を進める
    let range = clock.advance(wall_now);
    if range.is_empty() {
        return DispatchResult::empty(clock.now_ms);
    }

    // 3. イベント抽出
    let mut trade_events = vec![];
    let mut kline_events = vec![];
    for stream in active_streams {
        trade_events.push((*stream, store.trades_in(stream, range.clone()).to_vec()));
        kline_events.push((*stream, store.klines_in(stream, range.clone()).to_vec()));
    }

    DispatchResult {
        current_time: clock.now_ms,
        trade_events,
        kline_events,
        reached_end: clock.status == ClockStatus::Paused,
    }
}
```

#### Waiting → Playing 復帰フロー

`set_waiting` に落ちた後の再開は **Dashboard::update (Message::ReplayLoadCompleted) で明示的にトリガ**する。
次フレームの自動検出にしない理由：
- `VirtualClock::anchor_wall` のリセットを確実に行うため
- load 完了タイミングと次フレームの `wall_now` を区別するため

```
Message::ReplayLoadCompleted(stream, range, data)
  → store.ingest_loaded(stream, range, data)
  → if clock.status == Waiting && all active_streams loaded:
        clock.resume_from_waiting(Instant::now())
  → 次の Message::Tick で dispatch_tick が通常経路を走る
```

これにより load 完了から再生再開までのラグが 1 フレーム以内に抑えられ、
かつ待機中に仮想時間が進まない（anchor_wall を load 完了時刻にリセットするため）ことが保証される。

#### FireStatus 廃止の根拠

旧設計で `Pending` が必要だったのは「バックフィル中の chart に next_time を問い合わせられない」ため。
新設計では:

- **チャートに next_time は一切問い合わせない**。Store に問い合わせる。
- **Store がまだデータを持っていない場合は Clock が Waiting に落ちる**。自動的にタイム進行が止まる。
- **新 stream の追加は Store::request_load で完結**。バックフィル完了時に上記の復帰フローで自動再開。

### R5: D1 自動再生スロットリング (新設計)

COARSE_CUTOFF_MS ベースの heuristic を廃止し、**Clock に明示的な `bar_step_mode` を導入**する。

```rust
impl VirtualClock {
    /// D1 など大粒度 timeframe で使用する。
    /// 1 バー進めたら次のバー時刻まで wall 1 秒待機。
    pub fn enable_bar_step_mode(&mut self, bar_interval_ms: u64, wall_delay_ms: u64) { ... }
}
```

#### 切替判断

Dashboard が active pane の最大 timeframe を計算し、`>= D1` なら `enable_bar_step_mode(86_400_000, 1_000)` を呼ぶ。
この判断はリプレイ Play 時に 1 回だけ行う。timeframe 変更時に再計算。Phase R5 で実装する。

#### advance の擬似コード（bar_step_mode 有効時）

`bar_step_mode` は **wall_delay_ms ごとに確実に 1 バーずつ進める** 仕様とする。
wall 時間が `wall_delay_ms × N` 経過していれば **N バー catch-up する**（等速ペーシング）。
この catch-up により、フレーム落ちやユーザーがウィンドウ切替等で戻って来た際にも
「1 秒あたり 1 バー」の期待を保てる。

```
if bar_step_mode {
    let prev = self.now_ms;
    let wall_elapsed_ms = (wall_now - anchor_wall).as_millis() as u64;
    let bars_to_advance = wall_elapsed_ms / wall_delay_ms;
    if bars_to_advance > 0 {
        let current_bar = now_ms / bar_interval_ms;
        let next_bar_time = (current_bar + bars_to_advance) * bar_interval_ms;
        self.now_ms = next_bar_time.min(self.range.end);
        // 消費した wall 時間ぶんだけ anchor を進める（余剰は次 frame に繰り越し）
        self.anchor_wall = Some(anchor_wall + Duration::from_millis(bars_to_advance * wall_delay_ms));
        if self.now_ms >= self.range.end { self.status = Paused; }
    }
    return prev..self.now_ms;
}
```

**ポイント**:
- **「1 バーだけ」ではなく「経過時間ぶんの catch-up」**。タブ切替で 10 秒止まった後に戻ってくると 10 バー一気に進む。
- `anchor_wall` は消費した bar 分だけ進めることで、余剰の wall 時間（`wall_elapsed_ms % wall_delay_ms`）を次フレームに繰り越す。これにより端数誤差が累積しない。
- UX 上「タブ切替後に瞬時に 10 バー進む」が許容できない場合は catch-up 上限を設ける（例：`bars_to_advance.min(3)`）。初期実装では上限なしで始め、必要なら R5 のレビューで追加。

**利点**: D1 throttling が統一 Tick ループから独立した設定になり、テストが単純化される。

---

## 設計の tradeoff

### Pros

1. **時間管理が単一**: `VirtualClock::now_ms` のみ。デバッグ時の認知負荷が大幅に下がる。
2. **FireStatus 廃止**: Dashboard → 各ペイン → Clock の依存グラフが消え、コンポーネント単方向に整う。
3. **チャートが非対称性を持たない**: `fix_replay_pending_deadlock.md` で発生したようなモード取り違いバグが構造的に不可能になる。
4. **mid-replay CRUD がイベント駆動**: `SyncReplayBuffers` チェーン忘れバグが構造的に不可能になる。
5. **Step Backward が 2 ステップで閉じる**: `chart.reset_for_seek()` → `store.klines_in(start..t)` を再 ingest するだけ。cursor 復元 / 「未処理 trade 破棄」のような特殊ケースが消え、「画面状態 = Store からの全再生」という single source of truth が成立する。
6. **テストの単純化**: Clock / Store / Dispatcher が互いに独立した純粋関数層になり、境界値テストが激減する。

### Cons / リスク

1. **移行コストが大きい**: Phase R1-R7 で推定 1-2 週間。既存 E2E 236 件の再検証が必要。
2. **EventStore のメモリ使用量**: replay range 全体を bulk load するため、長時間 replay でメモリが増える。
   - 対策: 初期実装は Binance 数時間の range を想定 → 問題が出たら lazy window 方式に移行。
3. **bar_step_mode の導入タイミング**: D1 判断ロジックを Dashboard 側に持たせるか Clock 側に持たせるかで責務境界が悩ましい。R5 でプロトタイプし決定。
4. **`dispatch_tick` が全 active_streams をループするコスト**: 各フレームで stream 数 × binary_search。現実的には stream 数 ≤ 10 程度なので問題ないはずだが、R2 で計測する。
5. **R3 の一括差し替えリスク**: feature flag を使わないため、R3 の PR で旧経路が即削除される。手動検証 + 主要 E2E の事前 run を必須とする。

---

## 残課題との関係

### `fix_replay_pending_deadlock.md`

新設計では `replay_kline_buffer` 自体が存在しないため、ライブ用フェッチが遅延完了しても
**上書きされるべき state が存在しない**。問題の class そのものが消える。

### `pane_crud_api.md` Phase T 残課題

- **G1 (Playing 中 set-timeframe のタイムアウト)**: Dispatcher が新 stream を自動検出するため、タイムアウトが発生する構造的理由がなくなる。
- **G2 (M30/H1 での buffer_ready トグル数最適化)**: buffer_ready 概念が廃止される。
- **G3 (set-ticker で ticker_info 未ロード時の待機)**: Clock の Waiting status で自然に待機できる。sleep 5s 固定が不要に。
- **G5 (TAS close 直後の orphan 掃除)**: EventStore::drop_stream が Dispatcher ループから即反映される。
- **G9 (heatmap-only mid-replay)**: linear advance fallback 経路が不要 (FireStatus 廃止) なので構造的に動作する。

### 立花 D1 経路

新設計 R5 の `bar_step_mode` は現行 COARSE 経路より意図が明示的で、テスト容易性が向上する。

---

## 次のステップ

1. 本プランをレビュー → 合意取得
2. Phase R1 着手: `src/replay/` スケルトン + `VirtualClock` + `EventStore` + `dispatch_tick` を 1 PR で追加（既存コード未変更）
3. R2 以降は各 Phase を独立 PR にし、不具合時は `git revert` でロールバック
4. R3 の PR は特に入念なレビュー（旧経路を一括削除するため）

## 参考

- [replay_header.md](../replay_header.md) — リプレイ UI ヘッダーの UX 仕様
- [pane_crud_api.md](pane_crud_api.md) — Pane CRUD API / mid-replay E2E 検証
- [fix_replay_pending_deadlock.md](fix_replay_pending_deadlock.md) — replay_kline_buffer 上書きバグ修正
- [src/replay.rs](../../src/replay.rs) — 現行リプレイ実装
- [src/chart/kline.rs:163-446](../../src/chart/kline.rs#L163) — 現行 chart 側リプレイ結合
- [src/screen/dashboard.rs:1134-1168](../../src/screen/dashboard.rs#L1134) — 現行 fire_status 集約
