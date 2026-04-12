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
Step Backward も `Clock::seek(t)` → 次フレームで `trades_in(s, t..new_now)` で自然に処理される。

#### `Dispatcher`

```rust
/// VirtualClock と EventStore を橋渡しするステートレスなロジック層。
/// 各 Tick で呼ばれ、emit すべきイベントを集めて返す。
pub struct Dispatcher;

impl Dispatcher {
    /// フレーム駆動エントリポイント。
    /// - clock を advance してイベント範囲を取得
    /// - 各 active stream について store.*_in() でイベント抽出
    /// - (stream, events) 列を返す（呼び出し側が chart に ingest）
    pub fn tick(
        clock: &mut VirtualClock,
        store: &EventStore,
        active_streams: &HashSet<StreamKind>,
        wall_now: Instant,
    ) -> DispatchResult;
}

pub struct DispatchResult {
    pub current_time: u64,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    /// true なら replay 終端に到達、clock は Paused へ。
    pub reached_end: bool,
}
```

**ポイント**: FireStatus が消滅。Dispatcher は clock が進めば進んだぶんのイベントを返すだけで、
「バックフィル中」「pending」といった概念を持たない（待機は Clock 側の `Waiting` で表現）。

#### チャート側の変更

```rust
// KlineChart::replay_kline_buffer, enable_replay_mode, replay_advance_klines, is_replay_mode
// → 全て削除

// update_latest_kline / insert_hist_klines は Live / Replay で完全に同一経路
pub fn ingest_historical_klines(&mut self, klines: &[Kline]) { ... }
pub fn ingest_trades(&mut self, trades: &[Trade]) { ... } // Live と同じ
```

**ポイント**: チャートは自分がリプレイ中かを一切知らない。WebSocket と Dispatcher が emit するイベントが同じ型なので、ingest 経路も同一。

---

## モジュール構成

```
src/
├── replay/
│   ├── mod.rs              # 公開 API (ReplayState, ReplayMessage)
│   ├── clock.rs            # VirtualClock + ClockStatus
│   ├── store.rs            # EventStore + SortedVec
│   ├── dispatcher.rs       # Dispatcher::tick
│   ├── loader.rs           # 履歴データ fetch (現 fetch_trades_batched をここに移動)
│   └── ui.rs               # Replay header widget (現 replay.rs の UI 部分)
└── chart/kline.rs          # replay_* 系フィールド/メソッド全削除
```

現在の [src/replay.rs](../../src/replay.rs) 単一ファイル (≈2000 行) を機能単位で分割する。

## 移行計画（Phase 分割）

| Phase | 内容 | 検証 |
|:-:|---|---|
| **R0** | 新モジュール `src/replay/` スケルトン追加。既存 `src/replay.rs` はそのまま残す。 | `cargo check` |
| **R1** | `VirtualClock` 実装 + 単体テスト (play/pause/seek/speed)。 | `cargo test replay::clock` |
| **R2** | `EventStore` 実装 + 単体テスト (trades_in / klines_in / is_loaded)。 | `cargo test replay::store` |
| **R3** | `Dispatcher::tick` 実装 + 単体テスト (range 抽出と DispatchResult)。 | `cargo test replay::dispatcher` |
| **R4** | 新 Clock/Store/Dispatcher を **feature flag `new_replay`** で配線。`main.rs::Tick` 分岐で新経路を選択可能に。既存経路はそのまま残す。 | 手動切替で両経路を並行動作 |
| **R5** | Binance 向け履歴 loader (`replay/loader.rs`) 移植。EventStore に bulk load できるか確認。 | Replay Play で M1 が描画される |
| **R6** | KlineChart から `replay_kline_buffer` / `enable_replay_mode` / `replay_advance` を削除。`update_latest_kline` / `insert_hist_klines` が Live 経路と統一されることを確認。 | E2E: §6.2 #1 (M1 etc.) |
| **R7** | mid-replay CRUD: set-ticker / split / set-timeframe / close が新経路で動作することを確認。`SyncReplayBuffers` メッセージを削除。 | E2E: §6.2 #2, #5, #6, #7, #8 |
| **R8** | Tachibana D1 経路で throttling を新設計に移植。COARSE_CUTOFF_MS を廃止し、**Clock に `bar_step_mode: bool`** を導入して D1 のときだけ「1 バー描画後に wall 1 秒待機」へ切替。 | E2E: §6.2 #2 (Tachibana D1) |
| **R9** | heatmap-only fallback 経路が新設計では自動的に動作することを確認 (FireStatus 廃止により linear advance fallback も不要)。 | E2E: heatmap-only mid-replay |
| **R10** | 旧 `src/replay.rs` の全コード削除。feature flag `new_replay` を削除して新経路を既定に。全 236 E2E + ユニットテストが PASS することを確認。 | `cargo test` + 全 E2E green |

### ロールバック戦略

R4〜R9 の全期間で feature flag `new_replay` により旧実装へ即座に戻せる。
R10 のみ不可逆だが、git revert で巻き戻し可能。

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

### R2: EventStore

#### 保存形式

`SortedVec<T>` は内部的に `Vec<T>` で、挿入時に `sort_by_key` で時刻順を維持、
クエリは `binary_search_by_key` で range 切り出し。メモリ効率と実装単純性のトレードオフで `Vec` を選択。

#### 複数 range の管理

`loaded_ranges: Vec<Range<u64>>` は overlap や gap を検出して merge するのが望ましいが、
初期実装は「range ごとに独立フェッチ、重複を許容、クエリ時に union して slice」で始める。
後で最適化する。

#### CRUD 時の追加 load

```
// Message::PaneSetTicker(pane_id, new_ticker)
// → 旧: SyncReplayBuffers チェーン
// → 新: Task::batch [
//        EventStore::request_load(new_stream, replay.range),
//        Dispatcher は次フレームで自動的に new_stream を active_streams に含む
//      ]
```

### R3: Dispatcher

#### tick の擬似コード

```
fn tick(clock, store, active_streams, wall_now) -> DispatchResult {
    // 1. 全 active_streams の range が loaded か確認
    for stream in active_streams {
        if !store.is_loaded(stream, clock.now_ms..clock.now_ms + estimated_delta) {
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

#### FireStatus 廃止の根拠

旧設計で `Pending` が必要だったのは「バックフィル中の chart に next_time を問い合わせられない」ため。
新設計では:

- **チャートに next_time は一切問い合わせない**。Store に問い合わせる。
- **Store がまだデータを持っていない場合は Clock が Waiting に落ちる**。自動的にタイム進行が止まる。
- **新 stream の追加は Store::request_load で完結**。バックフィル完了時に自動再開。

### R8: D1 自動再生スロットリング (新設計)

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
この判断はリプレイ Play 時に 1 回だけ行う。timeframe 変更時に再計算。

#### advance の擬似コード（bar_step_mode 有効時）

```
if bar_step_mode {
    let next_bar = ((now_ms / bar_interval_ms) + 1) * bar_interval_ms;
    if wall_now - anchor_wall >= wall_delay_ms {
        self.now_ms = next_bar;
        self.anchor_wall = Some(wall_now);
    }
    return prev..self.now_ms;
}
```

**利点**: D1 throttling が統一 Tick ループから独立した設定になり、テストが単純化される。

---

## 設計の tradeoff

### Pros

1. **時間管理が単一**: `VirtualClock::now_ms` のみ。デバッグ時の認知負荷が大幅に下がる。
2. **FireStatus 廃止**: Dashboard → 各ペイン → Clock の依存グラフが消え、コンポーネント単方向に整う。
3. **チャートが非対称性を持たない**: `fix_replay_pending_deadlock.md` で発生したようなモード取り違いバグが構造的に不可能になる。
4. **mid-replay CRUD がイベント駆動**: `SyncReplayBuffers` チェーン忘れバグが構造的に不可能になる。
5. **Step Backward が自然**: `Clock::seek(t)` → Dispatcher が次フレームで `t..new_now` を返すだけ。cursor 復元ロジック不要。
6. **テストの単純化**: Clock / Store / Dispatcher が互いに独立した純粋関数層になり、境界値テストが激減する。

### Cons / リスク

1. **移行コストが大きい**: Phase R0-R10 で推定 1-2 週間。既存 E2E 236 件の再検証が必要。
2. **EventStore のメモリ使用量**: replay range 全体を bulk load するため、長時間 replay でメモリが増える。
   - 対策: 初期実装は Binance 数時間の range を想定 → 問題が出たら lazy window 方式に移行。
3. **bar_step_mode の導入タイミング**: D1 判断ロジックを Dashboard 側に持たせるか Clock 側に持たせるかで責務境界が悩ましい。R8 でプロトタイプし決定。
4. **Dispatcher が全 active_streams をループするコスト**: 各フレームで stream 数 × binary_search。現実的には stream 数 ≤ 10 程度なので問題ないはずだが、R5 で計測する。
5. **feature flag 期間の二重メンテ**: R4-R10 中は旧経路と新経路が共存するため、バグ修正が両方に必要になる可能性がある。R4-R10 を連続実行して期間を最小化する。

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

新設計 R8 の `bar_step_mode` は現行 COARSE 経路より意図が明示的で、テスト容易性が向上する。

---

## 次のステップ

1. 本プランをレビュー → 合意取得
2. Phase R0 着手: `src/replay/` スケルトン追加 + 既存 `src/replay.rs` をそのまま保持
3. 各 Phase ごとに短い PR を作成し、feature flag `new_replay` 経由で段階的に切替
4. R10 で旧コード削除

## 参考

- [replay_header.md](../replay_header.md) — リプレイ UI ヘッダーの UX 仕様
- [pane_crud_api.md](pane_crud_api.md) — Pane CRUD API / mid-replay E2E 検証
- [fix_replay_pending_deadlock.md](fix_replay_pending_deadlock.md) — replay_kline_buffer 上書きバグ修正
- [src/replay.rs](../../src/replay.rs) — 現行リプレイ実装
- [src/chart/kline.rs:163-446](../../src/chart/kline.rs#L163) — 現行 chart 側リプレイ結合
- [src/screen/dashboard.rs:1134-1168](../../src/screen/dashboard.rs#L1134) — 現行 fire_status 集約
