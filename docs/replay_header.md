# リプレイ機能 仕様書

**最終更新**: 2026-04-12
**対象バージョン**: `sasa/develop` ブランチ (Phase 1〜8 + Tachibana Phase 1〜3 完了)
**関連ドキュメント**:
- [docs/plan/archive/tachibana_replay.md](plan/archive/tachibana_replay.md) — 立花証券 D1 対応の実装経緯（完了、アーカイブ）
- [docs/plan/archive/replay_unified_step.md](plan/archive/replay_unified_step.md) — 統一 Tick ハンドラ設計メモ（完了、アーカイブ）
- [docs/plan/archive/refactor_tachibana_replay.md](plan/archive/refactor_tachibana_replay.md) — 未着手のリファクタ計画（archived）
- [docs/tachibana_spec.md §8](tachibana_spec.md#8-リプレイ対応の設計判断) — 立花証券リプレイ設計の「なぜ」

本書は flowsurface のリプレイ機能を、実装・API 利用・運用に十分な粒度で説明するリファレンス仕様書である。実装履歴は本文では触れず、必要に応じて §15 の付録を参照する。

---

## 目次

1. [概要](#1-概要)
2. [用語](#2-用語)
3. [UI 仕様](#3-ui-仕様)
4. [状態モデル](#4-状態モデル)
5. [メッセージとイベント](#5-メッセージとイベント)
6. [データフロー](#6-データフロー)
7. [再生エンジン](#7-再生エンジン)
8. [mid-replay ペイン操作](#8-mid-replay-ペイン操作)
9. [WebSocket 制御](#9-websocket-制御)
10. [取引所別対応状況](#10-取引所別対応状況)
11. [HTTP 制御 API](#11-http-制御-api)
12. [定数と設計不変条件](#12-定数と設計不変条件)
13. [スコープ外・既知の制限](#13-スコープ外既知の制限)
14. [実装ファイルマップ](#14-実装ファイルマップ)
15. [付録: 実装履歴と設計判断](#15-付録-実装履歴と設計判断)

---

## 1. 概要

flowsurface のリプレイ機能は、取引所 API から取得した過去の Kline / Trades データを時系列順に再生し、ライブチャートと同等のビュー更新を行うインフラである。ユーザーは以下のゲームループの **Step 1「観察」** を、過去の任意区間で繰り返し体験できる:

> 観察 → 仮説 → エントリー → 結果 → 改善

仮想売買・PnL・スコアリングといった後続ステップは本機能のスコープ外で、本機能はそれらの土台となる「決定論的なデータ再生基盤」を提供する。

### 1.1 主な機能

| 機能 | 内容 |
|---|---|
| モード切替 | LIVE / REPLAY をヘッダーバー or F5 or HTTP API でトグル |
| 範囲指定 | `YYYY-MM-DD HH:MM` 形式で開始・終了（UTC 解釈）|
| 再生制御 | Play / Pause / Resume / StepForward / StepBackward / CycleSpeed |
| 再生速度 | 1x / 2x / 5x / 10x の循環切替 |
| 段階更新 | ReplayKlineBuffer による kline の段階挿入（再生開始時に一括表示しない）|
| mid-replay ペイン操作 | リプレイ中のペイン追加・削除・timeframe / ticker 変更 |
| HTTP 制御 API | `127.0.0.1:9876` でリプレイ・ペイン操作を外部から駆動 |
| E2E テスト支援 | `POST /api/app/save` で状態をディスク保存 |

### 1.2 非ゴール

- 板情報（Depth）の再生 — 取引所 API が過去スナップショットを提供しない
- リプレイ範囲の永続化 — UI 状態のみ保持
- Comparison ペインのリプレイ — 複数銘柄同期は将来課題
- リプレイ中の Layout 切替 — `active_dashboard()` が変わる動作は未定義
- インジケータ再計算 / 仮想売買 — 別タスク

---

## 2. 用語

| 用語 | 定義 |
|---|---|
| **Live モード** | WebSocket から直接ストリーム受信する通常状態 |
| **Replay モード** | 過去データを仮想時刻で再生する状態 |
| **仮想時刻 (`current_time`)** | Replay 中に進行する Unix ms タイムスタンプ |
| **プリフェッチ** | Play 押下後、再生開始までに行う過去データの一括取得 |
| **バックフィル** | mid-replay でペインを追加した際の遅延フェッチ |
| **Tick** | `iced::window::frames()` が発火するフレームイベント (~60fps) |
| **統一 Tick** | timeframe に依存しない単一の `process_tick` 経路 |
| **FireStatus** | 次バー発火の状態（Ready/Pending/Terminal）|
| **ReplayKlineBuffer** | kline を段階挿入するための per-chart バッファ |
| **TradeBuffer** | trades の per-stream バッファ（cursor ベース）|
| **pending stream** | バックフィル中のため drain をスキップする trade stream 集合 |
| **orphan stream** | 削除済みだが `trade_buffers` に残存する stream（§12.3 不変条件 #2 で遮断）|

時刻はすべて **Unix ミリ秒 (`u64`)** を基準とし、表示時のみ `data::UserTimezone` で変換する。

---

## 3. UI 仕様

### 3.1 ヘッダーバー

メインウィンドウ最上部（macOS では `FLOWSURFACE` テキスト直下）に配置。

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ 🕐 2026-04-10 14:32:05  [LIVE/REPLAY]  [開始 ~ 終了]  ⏮ ▶⏸ ⏭ 1x  Loading... │
│   現在時刻              モードトグル     範囲入力      再生制御     ローディング │
└─────────────────────────────────────────────────────────────────────────────────┘
```

| 要素 | Live | Replay |
|---|:-:|:-:|
| 現在時刻 | リアルタイム (UTC or user TZ) | 仮想時刻 `current_time` |
| モードトグル | `LIVE` アクティブ | `REPLAY` アクティブ |
| 開始・終了日時 | read-only | 編集可 |
| ⏮ / ⏭ (Step) | 無効 | `playback.is_some()` && `!Loading` で有効 |
| ▶⏸ (Play/Pause/Resume) | 無効 | 状態依存で Play/Pause/Resume |
| 速度ボタン | 無効 | 有効、`1x → 2x → 5x → 10x → 1x` 循環 |
| `Loading...` | 非表示 | `status == Loading` で表示 |

速度ボタンには tooltip が付き、「M30 以下: 実時間連動 × speed / H1 以上: 1 バー/秒 × speed」と表示する。文言は §12.1 の `COARSE_CUTOFF_MS = 3_600_000ms` と **必ず同期**させる（§12.3 不変条件 #4）。

日時テキストは `iced::text_input` で `on_input` を渡さないことで read-only を実現する。

### 3.2 入力フォーマット

```
YYYY-MM-DD HH:MM    (UTC として解釈)
```

不正な場合は `ParseRangeError::{InvalidStartFormat, InvalidEndFormat, StartAfterEnd}` を返し、Toast 通知で中断する（`src/replay.rs` [`parse_replay_range`](../src/replay.rs)）。

### 3.3 キーバインド

| キー | 動作 |
|---|---|
| `F5` | `ReplayMessage::ToggleMode` をディスパッチ |
| `Escape` | `Message::GoBack`（リプレイとは独立、レイヤー閉じ）|

キーバインドは `subscription()` 内の `keyboard::listen().filter_map()` で処理する（[src/main.rs:1534-1577](../src/main.rs#L1534-L1577)）。

### 3.4 リプレイ中の UI 制約

- ペインの **位置移動 (drag / resize)** は無効化
- ペインの **追加 / 削除 / timeframe 変更 / ticker 変更** は許容（§8 参照）
- `Heatmap` / `ShaderHeatmap` / `Ladder` ペインには `"Replay: Depth unavailable"` オーバーレイを表示

---

## 4. 状態モデル

### 4.1 ReplayState ([src/replay.rs:41-54](../src/replay.rs#L41-L54))

```rust
pub struct ReplayState {
    pub mode: ReplayMode,
    pub range_input: ReplayRangeInput,
    pub playback: Option<PlaybackState>,
    pub last_tick: Option<std::time::Instant>,
    pub virtual_elapsed_ms: f64,
}

pub enum ReplayMode { Live, Replay }

pub struct ReplayRangeInput {
    pub start: String,
    pub end: String,
}
```

- `playback` は `Play` 押下で `Some` になり、`Live` 復帰で `None` に戻る
- `virtual_elapsed_ms` は Tick 毎に `elapsed_ms * speed` を累積する仮想時間カウンタ。次バー発火しきい値に到達するとジャンプ分だけ減算される
- `toggle_mode()` で Replay → Live に戻す際は `playback = None`, `range_input = default` にリセット

### 4.2 PlaybackState ([src/replay.rs:68-86](../src/replay.rs#L68-L86))

```rust
pub struct PlaybackState {
    pub start_time: u64,                             // Unix ms, パース済み
    pub end_time: u64,
    pub current_time: u64,                           // 仮想時刻
    pub status: PlaybackStatus,                      // Loading | Playing | Paused
    pub speed: f64,                                  // 1.0 | 2.0 | 5.0 | 10.0
    pub trade_buffers: HashMap<StreamKind, TradeBuffer>,
    pub resume_status: PlaybackStatus,               // DataLoaded 後の復帰先
    pub pending_trade_streams: HashSet<StreamKind>,  // バックフィル中 stream
}

pub enum PlaybackStatus { Loading, Playing, Paused }
```

- `status` 遷移: `Loading → (Playing | Paused) → ...`
  - `DataLoaded` 受信時に `resume_status` へ遷移。通常は `Playing`、StepBackward 経由では `Paused`
- `pending_trade_streams` に含まれる stream は `drain_all_trade_buffers` がスキップする（§7.2）

### 4.3 TradeBuffer ([src/replay.rs:95-278](../src/replay.rs#L95-L278))

```rust
pub struct TradeBuffer {
    pub trades: Vec<Trade>,
    pub cursor: usize,
}

impl TradeBuffer {
    pub fn drain_until(&mut self, current_time: u64) -> &[Trade];
    pub fn advance_cursor_to(&mut self, target_time: u64) -> usize;
    pub fn is_exhausted(&self) -> bool;
}
```

- `drain_until`: `cursor` から `time <= current_time` の範囲をスライスで返し、`cursor` を進める
- `advance_cursor_to`: `cursor` を単調増加で早送りする（mid-replay バックフィル完了時に「過去分 trades を捨てる」ため）
- 戻り値の `usize` はスキップ数

### 4.4 ReplayKlineBuffer ([src/chart/kline.rs](../src/chart/kline.rs))

Kline チャート側に持つバッファ。`enable_replay_mode()` で `Some(empty)` に初期化され、`insert_hist_klines()` がリプレイモード検出時にチャート本体ではなくバッファへ蓄積する。`replay_advance(current_time)` で `current_time` 以下の kline を段階的にチャートへ挿入する。

状態:

| 状態 | 意味 |
|---|---|
| `None` | リプレイ未対応 / ライブモード |
| `Some(empty)` | バックフィル待ち or 全 kline 挿入済み |
| `Some(with klines)` | 段階挿入中 |

`replay_buffer_ready()` は `Some` かつ `klines.len() > 0` のときに `true` を返す。§7.1 `FireStatus` の算出で使われる。

### 4.5 FireStatus ([src/replay.rs:368-378](../src/replay.rs#L368-L378))

```rust
pub enum FireStatus {
    Ready(u64),   // 全 ready chart の next_time_after の min が確定
    Pending,      // ready chart 無し、バックフィル中 chart あり → 待機
    Terminal,     // 全 chart 終端、バックフィル中も無し → Paused へ
}
```

`Dashboard::fire_status(current_time, main_window)` が全 kline chart を走査して返す。`None` の場合は kline chart が 1 つも無い（heatmap-only）ため、linear advance フォールバックに回る（§13 既知の制限 #1）。

---

## 5. メッセージとイベント

### 5.1 ReplayMessage ([src/replay.rs:101-133](../src/replay.rs#L101-L133))

| バリアント | 説明 |
|---|---|
| `ToggleMode` | Live / Replay 切替 |
| `StartTimeChanged(String)` | 開始日時入力変更 |
| `EndTimeChanged(String)` | 終了日時入力変更 |
| `Play` | 範囲パース → プリフェッチ開始 |
| `Resume` | 一時停止から再開 |
| `Pause` | 一時停止 |
| `StepForward` | 次バー境界へ離散ジャンプ |
| `StepBackward` | 前バー境界へ離散ジャンプ |
| `CycleSpeed` | 速度循環 |
| `TradesBatchReceived(StreamKind, Vec<Trade>)` | Straw ストリームのバッチ到着 |
| `TradesFetchCompleted(StreamKind)` | 全バッチ到着 |
| `DataLoaded` | 全プリフェッチ完了 |
| `DataLoadFailed(String)` | プリフェッチ失敗 |
| `SyncReplayBuffers` | mid-replay stream 構成変更バックフィル発火（§8）|

### 5.2 Message::Replay / Message::ReplayApi

`src/main.rs` の `Message` enum に以下が存在:

```rust
enum Message {
    // ...
    Replay(ReplayMessage),
    ReplayApi((replay_api::ApiCommand, replay_api::ReplySender)),
    // ...
}
```

- `Message::Replay` は UI / HTTP から発火する内部メッセージ
- `Message::ReplayApi` は HTTP サーバー subscription が発火する「コマンド + 応答チャネル」のタプル

---

## 6. データフロー

### 6.1 Play 押下から再生開始まで

```
[ReplayMessage::Play]
  ├─ 1. parse_replay_range(start, end) → (start_ms, end_ms)
  │     失敗時は Notification でエラー
  ├─ 2. Dashboard::prepare_replay() で全ペイン content をリビルド
  │     - settings / streams は保持
  │     - kline ペインは KlineChart::new() + enable_replay_mode()
  │     - Heatmap 等はクリア
  ├─ 3. PlaybackState::new() で status = Loading に初期化
  ├─ 4. build_kline_backfill_task() / build_trades_backfill_task() で
  │     Task::batch() を構築
  │     - Kline: fetch 範囲 = (start_ms - 450*tf, end_ms)
  │       → insert_hist_klines() が ReplayKlineBuffer に格納
  │     - Trades: Task::sip(fetch_trades_batched) →
  │       TradesBatchReceived ストリーム → PlaybackState::ingest_trades_batch
  ├─ 5. .chain(Task::done(Message::Replay(DataLoaded))) で完了通知
  └─ 6. subscription() の次評価で exchange_streams が外れ WS 切断

[DataLoaded 受信]
  └─ PlaybackState::status = resume_status (通常 Playing)
     current_time = start_time
```

### 6.2 Tick ループ（統一 Tick）

```
[Message::Tick(instant)]
  ├─ elapsed_ms = instant - last_tick
  ├─ fire_status = Dashboard::fire_status(current_time, main_window)
  │     None     → linear advance fallback (heatmap-only)
  │     Some(fs) → process_tick(pb, &mut virtual_elapsed_ms, elapsed_ms, fs)
  ├─ TickResult {
  │     current_time: Option<u64>,   // Some なら kline 段階挿入
  │     trades_collected: Option<Vec<(StreamKind, Vec<Trade>, u64)>>,
  │   }
  ├─ current_time が Some なら
  │     Dashboard::replay_advance_klines(new_current_time) で全 chart に段階挿入
  └─ trades_collected が Some なら
        Dashboard::ingest_trades(stream, trades, update_t) で分配
```

`process_tick` のアルゴリズムは §7.1 を参照。

### 6.3 mid-replay バックフィル

```
[ペイン操作: Sidebar / Dashboard / Pane API]
  ├─ streams を mutate
  └─ return path で .chain(Task::done(Message::Replay(SyncReplayBuffers)))

[SyncReplayBuffers 受信]
  ├─ 1. collect_new_replay_klines() で replay_kline_buffer==None の kline pane 検出
  │     → enable_replay_mode() で Some(empty) に切替
  ├─ 2. PlaybackState::diff_trade_streams(current_streams) → TradeStreamDiff
  │     { new_streams, orphan_streams }
  ├─ 3. trade_buffers 更新
  │     - new: 空 buffer 追加 + pending_trade_streams に追加
  │     - orphan: trade_buffers / pending から削除
  ├─ 4. バックフィル Task::batch() 構築
  │     - build_kline_backfill_task(pane_id, stream, start, end, layout_id)
  │     - build_trades_backfill_task(stream, start, end)
  └─ 5. Task を発火 → 既存 chart の再生は止めない
        （バックフィル中 chart は replay_buffer_ready()==false で fire_status から除外）

[TradesFetchCompleted(stream) 受信]
  ├─ advance_cursor_to(pb.current_time) で過去分を読み捨て
  └─ pending_trade_streams から stream を削除 → 通常 drain 経路に合流
```

### 6.4 Live 復帰

```
[ReplayMessage::ToggleMode (Replay → Live)]
  ├─ playback = None, range_input = default
  ├─ Dashboard::rebuild_for_live() で content リビルド + disable_replay_mode
  └─ subscription() 次評価で exchange_streams 復帰 → WS 自動再購読
```

---

## 7. 再生エンジン

### 7.1 統一 Tick ([src/replay.rs:434-490](../src/replay.rs#L434-L490))

```rust
pub fn process_tick(
    pb: &mut PlaybackState,
    virtual_elapsed_ms: &mut f64,
    elapsed_ms: f64,
    fire_status: FireStatus,
) -> TickResult;
```

**アルゴリズム（案 C, §12.1 `COARSE_CUTOFF_MS` 参照）**:

1. `Terminal` → `status = Paused`, `virtual_elapsed_ms = 0.0`, 戻り値は空
2. `Pending` → 何もしない（drain も行わない）
3. `Ready(next_fire)`:
    a. まず現在時刻で `drain_all_trade_buffers(pb)` (穴 A: 未達 Tick でも trades を流す)
    b. `delta_to_next = next_fire - pb.current_time`
    c. `threshold_ms = if delta_to_next >= COARSE_CUTOFF_MS { COARSE_BAR_MS } else { delta_to_next }`
    d. `virtual_elapsed_ms += elapsed_ms * pb.speed`
    e. `virtual_elapsed_ms + 1e-6 < threshold_ms` → drain だけ返して終了
    f. 以上: `virtual_elapsed_ms -= threshold_ms`, `pb.current_time = next_fire`
    g. ジャンプ後に再度 `drain_all_trade_buffers(pb)` (cursor ベースで冪等)

**速度セマンティクス**:

| 条件 | モード | threshold |
|---|---|---|
| `delta < COARSE_CUTOFF_MS (1h)` | 実時間連動 | `delta × speed` |
| `delta >= COARSE_CUTOFF_MS` | 粗補正 | `COARSE_BAR_MS (1000ms) × speed` |

- M1〜M30 単独: 実時間連動（1x = 実時間、10x = 10倍速）
- H1〜D1 単独: 1 バー/秒 × speed
- M1+D1 混在: min = M1 なので M1 基準、D1 は 1440 Tick ごとに 1 本（実質停止）

この設計は立花証券 D1 リプレイを実用時間内で進行させるための唯一の方法である（§10）。

### 7.2 drain_all_trade_buffers

```rust
fn drain_all_trade_buffers(pb: &mut PlaybackState)
    -> Option<Vec<(StreamKind, Vec<Trade>, u64)>>;
```

- `pending_trade_streams` の stream はスキップ
- `trade_buffers` が全て空なら `None`
- `Some(collected)` は `(stream, trades, update_t)` の Vec、`update_t` は最後の trade の time

### 7.3 StepForward / StepBackward

離散ステップに統一。timeframe に関わらず「次/前のバー境界」にジャンプする。

```rust
// StepForward
Dashboard::replay_next_kline_time(current_time)  // → Option<u64>
// StepBackward
Dashboard::replay_prev_kline_time(current_time)  // → Option<u64>
```

StepBackward は以下のシーケンスを踏む:

1. `current_time = prev_time` (min: `start_time`)
2. 全 `TradeBuffer` の cursor を 0 にリセット
3. `drain_until(prev_time)` で cursor を早送り
4. `Dashboard::rebuild_for_step_backward()` でバッファ保持版リビルド
5. `Dashboard::replay_advance_klines(prev_time)` で kline 再挿入
6. `status = Paused` で停止

kline の再フェッチは行わない（バッファから再構成）。

### 7.4 Pause / Resume / CycleSpeed

- `Pause` / `Resume` は `status` の単純遷移
- `CycleSpeed` は `SPEEDS = [1.0, 2.0, 5.0, 10.0]` を `(i+1) % 4` で循環
- `speed_label()` は `if speed == floor(speed) { "Nx" } else { "N.Nx" }` を返す

---

## 8. mid-replay ペイン操作

### 8.1 許容される操作

- SplitPane / ClosePane
- Timeframe 変更
- Ticker 変更
- Sidebar からの TickerSelected
- HTTP Pane API 経由の全操作（§11.2）

### 8.2 SyncReplayBuffers 発火経路

`SyncReplayBuffers` は 2 系統の chain で発火する:

1. **一次集約**: [src/main.rs](../src/main.rs) の `Message::Dashboard` ハンドラ return path 末尾で `.chain(Task::done(Message::Replay(SyncReplayBuffers)))`
2. **二次個別 chain**: `Message::Sidebar::TickerSelected` の return path に追加

二次個別 chain が必要な理由: `init_focused_pane()` が `Task::none()` を返すケース（heatmap-only）があり、一次集約だけでは chain が発火しないため。

**設計上の不変条件**: streams を mutate するが `Task::none()` を返すハンドラを新設する場合は、呼び出し側で個別 chain が必須（§12.3 不変条件 #3）。

### 8.3 TradeStreamDiff ([src/replay.rs:332-359](../src/replay.rs#L332-L359))

```rust
pub struct TradeStreamDiff {
    pub new_streams: Vec<StreamKind>,     // バックフィル対象
    pub orphan_streams: Vec<StreamKind>,  // 削除対象
}

impl PlaybackState {
    pub fn diff_trade_streams(&self, current: &[StreamKind]) -> TradeStreamDiff;
}
```

順序保持のため `Vec` を使用。`contains` の O(n²) コストは許容（通常 2〜8 stream 程度）。

### 8.4 Orphan trade stream 対策

`TradesBatchReceived` は `PlaybackState::ingest_trades_batch(stream, batch)` を経由する:

```rust
pub fn ingest_trades_batch(&mut self, stream: StreamKind, batch: Vec<Trade>) -> bool {
    match self.trade_buffers.get_mut(&stream) {
        Some(buffer) => { buffer.trades.extend(batch); true }
        None => false,  // orphan: 黙って drop
    }
}
```

**重要**: `or_insert_with` を使わない。これは orphan 削除済み stream に残存 fetch タスクから到着するバッチが buffer を自己復活させ、次の `SyncReplayBuffers` で再度 orphan 検出 → 無限 flap ループになるのを防ぐ（§12.3 不変条件 #2）。

**残課題**: 残存 fetch タスクは自然完了まで稼働する。将来 `trade_fetch_handles: HashMap<StreamKind, AbortHandle>` で abort 経路を追加可能。

### 8.5 バックフィル中の chart 扱い

- kline: `replay_buffer_ready() == false` → `fire_status()` の min 計算から除外
- trade: `pending_trade_streams` に含まれる → `drain_all_trade_buffers` がスキップ

いずれも既存 chart の再生を止めない。fetch 完了で自動的に通常経路へ合流する。

### 8.6 `set_basis()` の不変条件

`KlineChart::set_basis()` (timeframe 変更時呼ばれる) は末尾で:

```rust
if let Some(buffer) = self.replay_kline_buffer.as_mut() {
    buffer.klines.clear();
    buffer.cursor = 0;
}
```

を実行する。**`Some`/`None` の状態は維持し、中身だけ空にする**。これを破ると timeframe 変更後に `collect_new_replay_klines()` がバックフィル対象として検出できず、mid-replay timeframe 変更が壊れる（§12.3 不変条件 #1）。

---

## 9. WebSocket 制御

`subscription()` [src/main.rs:1534-1577](../src/main.rs#L1534-L1577) の構造:

```rust
fn subscription(&self) -> Subscription<Message> {
    let window_events = window::events().map(Message::WindowEvent);
    let sidebar      = self.sidebar.subscription().map(Message::Sidebar);
    let replay_api   = Subscription::run(replay_api::subscription).map(Message::ReplayApi);

    // ログイン画面中でも API は常時 ON
    if self.login_window.is_some() {
        return Subscription::batch(vec![window_events, sidebar, replay_api]);
    }

    let tick    = iced::window::frames().map(Message::Tick);
    let hotkeys = keyboard::listen().filter_map(|e| {
        let KeyPressed { key, .. } = e else { return None };
        match key {
            Key::Named(Escape) => Some(Message::GoBack),
            Key::Named(F5)     => Some(Message::Replay(ReplayMessage::ToggleMode)),
            _ => None,
        }
    });

    if self.replay.is_replay() {
        // Replay 中: exchange_streams を外す
        return Subscription::batch(vec![
            window_events, sidebar, tick, hotkeys, replay_api
        ]);
    }

    let exchange_streams = self.active_dashboard()
        .market_subscriptions()
        .map(Message::MarketWsEvent);

    Subscription::batch(vec![
        exchange_streams, sidebar, window_events, tick, hotkeys, replay_api
    ])
}
```

**ポイント**:
- iced の宣言的 subscription により、`exchange_streams` を返さなくなった時点で WebSocket は自動切断
- Live 復帰時も次の評価で自動再購読
- `replay_api` は全状態で常時購読（ログイン画面中でも API が動く）
- F5 は全 replay 状態でトグル可能（Live ↔ Replay）

---

## 10. 取引所別対応状況

| 取引所 | Kline | Trades | Depth | リプレイ可否 |
|---|:-:|:-:|:-:|:-:|
| Binance (Spot / Linear / Inverse) | ✅ 全 tf | ✅ `fetch_trades_batched` | ❌ | ✅ 完全 |
| Bybit | ✅ 全 tf | ❌ | ❌ | ⚠️ kline のみ |
| Hyperliquid | ✅ 全 tf | ❌ | ❌ | ⚠️ kline のみ |
| OKX | ✅ 全 tf | ❌ | ❌ | ⚠️ kline のみ |
| MEXC | ✅ 全 tf | ❌ | ❌ | ⚠️ kline のみ |
| **Tachibana (立花証券)** | ✅ **D1 のみ** | ❌ | ❌ | ⚠️ D1 kline のみ |

### 10.1 Tachibana D1 の特記事項

立花証券は日足のみ API 提供のため、以下の特別処理が入る:

1. **range フィルタ**: `fetch_tachibana_daily_klines(issue_code, range)` が全履歴取得後に `range` でフィルタする（API 自体は range 引数を受け付けない）
2. **離散ステップ**: StepForward / StepBackward が kline timestamp リストに基づき休場日（土日祝）を自動スキップ
3. **D1 自動再生スロットリング**: `COARSE_CUTOFF_MS` 境界（1h）で粗補正モードが発動し、1 バー/秒 × speed で進行
4. **Play 時の挙動**: 設計上は自動再生可能だが、UX として日足は StepForward / StepBackward での逐次操作を推奨

設計判断の背景（なぜこの実装か）は [docs/tachibana_spec.md §8](tachibana_spec.md#8-リプレイ対応の設計判断) を参照。実装経緯（Phase 1〜3 作業ログ）は [docs/plan/archive/tachibana_replay.md](plan/archive/tachibana_replay.md) に保存。

---

## 11. HTTP 制御 API

ローカル HTTP サーバーが `127.0.0.1:9876` で常時待機し、外部プロセスからリプレイとペイン操作を駆動できる。E2E テスト・自動化ツール・スクリプト制御を想定。

### 11.1 ベース仕様

| 項目 | 値 |
|---|---|
| Bind | `127.0.0.1` （外部アクセス不可）|
| Port | 環境変数 `FLOWSURFACE_API_PORT` or デフォルト `9876` |
| Protocol | HTTP/1.1, `Connection: close`, `Access-Control-Allow-Origin: *` |
| Content-Type | `application/json` (レスポンス・リクエスト) |
| Keep-alive | 非対応（1 リクエスト / 接続）|
| 最大リクエストサイズ | 8192 バイト |

**ステータスコード**:

| コード | 意味 |
|---|---|
| 200 | 成功 |
| 400 | Bad Request — 不正 JSON、必須フィールド欠落、不正 UUID、不正 axis |
| 404 | Not Found — 未定義のパスまたは method |
| 500 | Internal Server Error — アプリチャネル切断、応答タイムアウト |

### 11.2 エンドポイント一覧

#### リプレイ制御 (`/api/replay/*`)

| メソッド | パス | リクエストボディ | レスポンス | 対応コマンド |
|---|---|---|---|---|
| GET | `/api/replay/status` | — | `ReplayStatus` | `GetStatus` |
| POST | `/api/replay/toggle` | — | `ReplayStatus` | `Toggle` |
| POST | `/api/replay/play` | `{"start": "YYYY-MM-DD HH:MM", "end": "YYYY-MM-DD HH:MM"}` | `ReplayStatus` | `Play` |
| POST | `/api/replay/pause` | — | `ReplayStatus` | `Pause` |
| POST | `/api/replay/resume` | — | `ReplayStatus` | `Resume` |
| POST | `/api/replay/step-forward` | — | `ReplayStatus` | `StepForward` |
| POST | `/api/replay/step-backward` | — | `ReplayStatus` | `StepBackward` |
| POST | `/api/replay/speed` | — | `ReplayStatus` | `CycleSpeed` |

#### アプリ制御 (`/api/app/*`)

| メソッド | パス | 用途 |
|---|---|---|
| POST | `/api/app/save` | アプリ状態をディスクに保存 (E2E テスト用)|

#### ペイン CRUD (`/api/pane/*`)

| メソッド | パス | リクエストボディ | 用途 |
|---|---|---|---|
| GET | `/api/pane/list` | — | 全ペイン + リプレイバッファ状態を返す |
| POST | `/api/pane/split` | `{"pane_id": "<uuid>", "axis": "Vertical"\|"Horizontal"}` | ペイン分割 |
| POST | `/api/pane/close` | `{"pane_id": "<uuid>"}` | ペイン削除 |
| POST | `/api/pane/set-ticker` | `{"pane_id": "<uuid>", "ticker": "BinanceLinear:BTCUSDT"}` | ticker 差し替え |
| POST | `/api/pane/set-timeframe` | `{"pane_id": "<uuid>", "timeframe": "M1"\|...\|"D1"}` | timeframe 変更 |

#### Sidebar 経由 (`/api/sidebar/*`)

| メソッド | パス | リクエストボディ | 用途 |
|---|---|---|---|
| POST | `/api/sidebar/select-ticker` | `{"pane_id": "<uuid>", "ticker": "...", "kind": "..." or null}` | Sidebar::TickerSelected 経路の再現（Fix 4 回帰テスト用）|

`kind = None` → `switch_tickers_in_group` 経路、`Some` → `init_focused_pane` 経路。どちらも `SyncReplayBuffers` chain が発火することを検証するための経路。

### 11.3 レスポンススキーマ

#### `ReplayStatus` ([src/replay.rs:21-38](../src/replay.rs#L21-L38))

```rust
pub struct ReplayStatus {
    pub mode: String,                    // "Live" | "Replay"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,          // "Loading" | "Playing" | "Paused"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_time: Option<u64>,       // Unix ms

    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<String>,           // "1x" | "2x" | "5x" | "10x"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<u64>,

    pub range_start: String,             // UI 入力テキスト（常に含まれる）
    pub range_end: String,
}
```

**Live モード レスポンス例**（playback 未開始時は 5 フィールドが省略される）:

```json
{
  "mode": "Live",
  "range_start": "",
  "range_end": ""
}
```

**Replay Playing レスポンス例**:

```json
{
  "mode": "Replay",
  "status": "Playing",
  "current_time": 1743492600000,
  "speed": "2x",
  "start_time": 1743487200000,
  "end_time": 1743508800000,
  "range_start": "2026-04-01 09:00",
  "range_end": "2026-04-01 15:00"
}
```

#### ペインリスト (`GET /api/pane/list`)

```json
{
  "panes": [
    {
      "id": "<uuid>",
      "window_id": "MainWindow",
      "type": "Kline" | "Heatmap" | "ShaderHeatmap" | "TimeAndSales" | "Ladder" | "Starter",
      "ticker": "BinanceLinear:BTCUSDT" | null,
      "timeframe": "M1" | ... | "D1" | null,
      "link_group": "A" | null,
      "replay_buffer_ready": true,
      "replay_buffer_cursor": 42,
      "replay_buffer_len": 450
    }
  ],
  "pending_trade_streams": ["Trades(BinanceLinear:BTCUSDT)"],
  "trade_buffer_streams": ["Trades(BinanceLinear:BTCUSDT)", "Trades(BinanceLinear:ETHUSDT)"]
}
```

`pending_trade_streams` と `trade_buffer_streams` は orphan 除去の検証用フィールド（E2E テストが close 後に該当 stream が消えることを確認する）。

### 11.4 エラーレスポンス

```json
{"error": "Not Found"}
{"error": "Bad Request"}
{"error": "Bad Request: invalid JSON body"}
{"error": "invalid axis: <value> (expected Vertical or Horizontal)"}
{"error": "pane not found: <uuid>"}
{"error": "App channel closed"}
{"error": "No response from app"}
{"error": "failed to serialize pane list"}
```

### 11.5 アーキテクチャ

```
┌────────────────────┐   mpsc (ApiMessage)   ┌───────────────────┐
│  HTTP Server       │  ─────────────────►  │  iced app         │
│  (tokio::spawn)    │                       │  subscription()   │
│  127.0.0.1:9876    │  ◄─────────────────  │  update()         │
│                    │   oneshot<String>     │                   │
└────────────────────┘                       └───────────────────┘
```

1. HTTP リクエスト到着 → `parse_request` → `route(method, path, body)` → `ApiCommand`
2. `mpsc::send((cmd, ReplySender::new(reply_tx)))` で iced 側へ投入
3. iced `Message::ReplayApi((cmd, reply_tx))` ハンドラが:
   - `ApiCommand::Replay(cmd)` → `self.update(Message::Replay(...))` に委譲 + `to_status()` JSON 応答
   - `ApiCommand::Pane(cmd)` → `handle_pane_api(cmd)` → `(json, task)` 応答
4. `reply_tx.send(json)` → HTTP ハンドラが `write_response(200, json)` で返送

**ReplySender の Clone 戦略**: iced の `Message: Clone` 制約のため、`oneshot::Sender` を `Arc<Mutex<Option<Sender>>>` でラップし、`take()` で一度だけ送信する（二重送信防止）。

### 11.6 利用例

```bash
# モード確認
curl http://127.0.0.1:9876/api/replay/status

# Replay に切替
curl -X POST http://127.0.0.1:9876/api/replay/toggle

# 再生開始
curl -X POST http://127.0.0.1:9876/api/replay/play \
  -d '{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}'

# 一時停止
curl -X POST http://127.0.0.1:9876/api/replay/pause

# ペイン一覧（リプレイバッファ状態も含む）
curl http://127.0.0.1:9876/api/pane/list

# ペインを分割
curl -X POST http://127.0.0.1:9876/api/pane/split \
  -d '{"pane_id":"<uuid>","axis":"Vertical"}'

# ティッカーを差し替え（mid-replay 可）
curl -X POST http://127.0.0.1:9876/api/pane/set-ticker \
  -d '{"pane_id":"<uuid>","ticker":"BinanceLinear:ETHUSDT"}'

# 状態をディスクに保存
curl -X POST http://127.0.0.1:9876/api/app/save
```

---

## 12. 定数と設計不変条件

### 12.1 定数一覧

| 定数 | 値 | 定義箇所 | 意味 |
|---|---|---|---|
| `COARSE_CUTOFF_MS` | `3_600_000` (1h) | [src/replay.rs:363](../src/replay.rs#L363) | 粗補正モード境界 |
| `COARSE_BAR_MS` | `1_000` (1 sec) | [src/replay.rs:366](../src/replay.rs#L366) | 粗補正時の 1 バー threshold |
| `SPEEDS` | `[1.0, 2.0, 5.0, 10.0]` | [src/replay.rs:281](../src/replay.rs#L281) | 再生速度テーブル |
| Kline backfill bars | `450` | [src/main.rs](../src/main.rs) | リプレイ開始前に追加フェッチする本数 |
| API port (default) | `9876` | [src/replay_api.rs:85](../src/replay_api.rs#L85) | `FLOWSURFACE_API_PORT` で上書き |
| HTTP buffer size | `8192` | [src/replay_api.rs:113](../src/replay_api.rs#L113) | リクエスト読み取りバッファ |
| mpsc channel bound | `32` | [src/replay_api.rs:75](../src/replay_api.rs#L75) | API → iced キュー |

### 12.2 時間範囲

- **Kline フェッチ範囲**: `(start_ms - 450 * tf.to_milliseconds(), end_ms)` — リプレイ開始前のコンテキストも 450 本含める
- **Trades フェッチ範囲**: `(start_ms, end_ms)` — Binance のみ `fetch_trades_batched` を Straw ストリームで消費

### 12.3 設計上の不変条件

| # | 不変条件 | 破壊したときの症状 | 参照 |
|:-:|---|---|---|
| 1 | `KlineChart::set_basis()` は `replay_kline_buffer` の `Some/None` 状態を維持し、中身だけ空にする | mid-replay timeframe 変更で `collect_new_replay_klines()` がバックフィル発火しない | §8.6 |
| 2 | `trade_buffers` への挿入は `PlaybackState::ingest_trades_batch` 経由のみ。`or_insert_with` を使わない | 削除済み stream が残存 fetch タスクで復活し、無限 flap ループ | §8.4 |
| 3 | `SyncReplayBuffers` は `Message::Dashboard` 末尾 chain + `Message::Sidebar::TickerSelected` 個別 chain の両方が必要 | `Task::none()` を返す経路で mid-replay stream 構成変更が追従しない | §8.2 |
| 4 | `COARSE_CUTOFF_MS` の変更時は speed ボタン tooltip 文言を必ず同期する | UI と実挙動が乖離 | §3.1 / §7.1 |

不変条件 #4 は `coarse_cutoff_boundary_matches_h1_in_ms` テストがトリップワイヤーとして機能する。

### 12.4 時刻の取扱い

- 全レイヤーで **Unix ms (`u64`)** を使用
- 表示変換のみ `data::UserTimezone::format_with_kind(ms: i64, TimeLabelKind)` を使う
- 入力パースは UTC として解釈 (`NaiveDateTime::parse_from_str` + `and_utc().timestamp_millis()`)

---

## 13. スコープ外・既知の制限

### 13.1 スコープ外

| 項目 | 理由 |
|---|---|
| Depth（板情報）のリプレイ | 取引所 API で過去スナップショット取得不可 |
| Comparison ペインのリプレイ | 複数銘柄同期フェッチが必要 |
| Layout 切替中のリプレイ | `active_dashboard()` が変わる動作が未定義 |
| リプレイ範囲の永続化 | UI 状態のみ |
| インジケータ再計算 | まずは Kline + Trades に集中 |
| リプレイデータのローカルキャッシュ | 毎回 API 取得 |
| 日時ピッカー UI | 現状はテキスト入力 |
| Tachibana の M1 / 時間足 | API 非対応 |

### 13.2 既知の制限

| # | 制限 | 影響 |
|---|---|---|
| 1 | linear advance フォールバック経路の `pending_trade_streams` 未対応 | heatmap-only リプレイ中の mid-replay ペイン追加で pending ガードが効かない |
| 2 | fetch タスクの abort 経路不在 | orphan 削除後も残存 fetch が自然完了まで稼働（CPU/ネット負荷）|
| 3 | H1 境界の非連続性 | M30 (6 分/本) → H1 (1 秒/本) で 360× の速度ジャンプ。混在時は tooltip で明示 |
| 4 | M1+D1 混在で D1 は実質停止 | M1 基準で進行するため D1 は 1440 Tick ごとに 1 本 |
| 5 | 高速再生時のデータ粒度 | 10x 以上ではフレーム間に複数バー分の trades が集中する可能性 |
| 6 | Binance 以外の trades 未対応 | Bybit / Hyperliquid / OKX / MEXC / Tachibana では Kline のみの再生 |

---

## 14. 実装ファイルマップ

### 14.1 主要ファイル

| ファイル | 責務 |
|---|---|
| [src/replay.rs](../src/replay.rs) | `ReplayState` / `PlaybackState` / `ReplayMessage` / `ReplayCommand` / `ReplayStatus` / `TradeBuffer` / `FireStatus` / `TickResult` / `TradeStreamDiff` / `process_tick` / `drain_all_trade_buffers` / `parse_replay_range` / `format_current_time` / `COARSE_*` 定数 |
| [src/replay_api.rs](../src/replay_api.rs) | HTTP サーバー (`tokio::net::TcpListener` + 手動パース) / `ApiCommand` / `PaneCommand` / `ReplySender` / ルーティング |
| [src/main.rs](../src/main.rs) | `Flowsurface` への `replay` フィールド、`Message::Replay` / `Message::ReplayApi` ハンドラ、`view_replay_header()`、`subscription()`、`build_kline_backfill_task()` / `build_trades_backfill_task()`、`handle_pane_api()` 各メソッド、`SyncReplayBuffers` chain |
| [src/screen/dashboard.rs](../src/screen/dashboard.rs) | `prepare_replay()` / `rebuild_for_step_backward()` / `rebuild_for_live()` / `collect_trade_streams()` / `collect_new_replay_klines()` / `fire_status()` / `replay_advance_klines()` / `replay_next_kline_time()` / `replay_prev_kline_time()` / `ingest_trades()` の ticker_info マッチング |
| [src/screen/dashboard/pane.rs](../src/screen/dashboard/pane.rs) | `rebuild_content_for_replay()` / `rebuild_content_for_step_backward()` / `rebuild_content_for_live()` / `enable_replay_mode_if_needed()` / `replay_kline_chart_ready()` / Heatmap の Depth unavailable オーバーレイ |
| [src/chart/kline.rs](../src/chart/kline.rs) | `ReplayKlineBuffer` / `enable_replay_mode()` / `replay_advance()` / `replay_buffer_ready()` / `set_basis()` の buffer 再初期化 |
| [src/connector/fetcher.rs](../src/connector/fetcher.rs) | Tachibana D1 range フィルタ分岐 |

### 14.2 テスト

- `src/replay.rs` のユニットテスト: 80 件超 (`#[test]` ~82 個)
- `src/replay_api.rs` のルーター/パーサーテスト
- `src/chart/kline.rs` の ReplayKlineBuffer テスト
- `src/screen/dashboard.rs` / `pane.rs` の fire_status / collect_new_replay_klines テスト
- E2E テスト: `tests/` ディレクトリ配下、HTTP API 経由でシナリオ検証

`cargo test --bin flowsurface` で全件実行する。具体的な件数はコードと共に変動するため本書では固定値を明示しない。

---

## 15. 付録: 実装履歴と設計判断

### 15.1 実装フェーズ

本機能は以下の Phase で段階的に実装された。詳細な作業ログは各計画書を参照:

| Phase | 内容 | 参照 |
|---|---|---|
| Phase 1 | ヘッダーバー UI（見た目のみ）| 本書 §3 |
| Phase 2 | リプレイデータのプリフェッチ | 本書 §6.1 |
| Phase 3 | リプレイ再生エンジン (Tick + 段階挿入) | 本書 §7 |
| Phase 4 | ライブ復帰・巻き戻し・速度切替 | 本書 §7.3 |
| Phase 5 | ローカル HTTP 制御 API | 本書 §11 |
| Phase 6 | 統一 Tick ハンドラ（D1 分岐撤廃） | [docs/plan/archive/replay_unified_step.md](plan/archive/replay_unified_step.md) |
| Phase 7 | mid-replay ペイン操作許容 | 本書 §8 |
| Phase 8 | レビュー駆動の設計不整合修正 | 本書 §12.3 |
| Tachibana Phase 1〜3 | 立花証券 D1 対応 | [docs/tachibana_spec.md §8](tachibana_spec.md) / [docs/plan/archive/tachibana_replay.md](plan/archive/tachibana_replay.md) |

未着手のリファクタ計画は [docs/plan/archive/refactor_tachibana_replay.md](plan/archive/refactor_tachibana_replay.md) を参照（main.rs スリム化、Tachibana static 撤去、命名統一 など）。

### 15.2 統一 Tick ハンドラの設計判断

§7.1 の `process_tick` と `COARSE_CUTOFF_MS` 境界による threshold 切替は、いくつかの代替案との比較を経て選定された。将来の設計変更時にトレードオフを見失わないよう、議論の要点を保存する。

#### 15.2.1 問題: D1 分岐が脆かった

初期実装（Tachibana Phase 3）では、Tick ハンドラを `Dashboard::is_all_d1_klines()` による 2 分岐にしていた:

- **D1-only**: `advance_d1()` で 1 秒/本の離散ジャンプ
- **非 D1**: `advance_time(elapsed_ms)` で連続前進 + `drain_until`

この設計の問題点:

1. **混在構成で D1 が停止**: M1 + D1 混在ペインだと非 D1 経路に落ち、D1 は 24 時間/本のペースになる
2. **ペイン構成依存**: リプレイ中にペインを追加・削除すると `is_all_d1_klines()` の返値が切り替わり、再生速度がペイン操作で変わる脆さ
3. **新規ペインが fire できない**: `advance_time` は buffer の末尾を見ないため、バックフィル待機中か終端かを区別できない

#### 15.2.2 検討した 3 案

| 案 | 概要 | 評価 |
|---|---|---|
| **A: 全 tf で 1 bar/sec 統一** | `SPEEDS` を「bars/sec」として再解釈し、timeframe に関わらず「1 tick = 次バーへ離散ジャンプ」 | M1 リプレイの「実時間感覚」を失う。ライブと同じ UX で観察したい要件に合わない |
| **B: 全 tf で実時間連動** | `threshold = delta_to_next` とし、timeframe に比例して実時間で進む | D1 が 24 時間/本で実用不能（Phase 3 で既に却下済み） |
| **C: 混合（実時間連動 + 粗 tf 補正）** | `delta < COARSE_CUTOFF_MS` → 実時間、`delta >= COARSE_CUTOFF_MS` → 1 bar/sec | **採用**。M1 の UX を守りつつ、D1 を救済できる |

#### 15.2.3 案 C の採用理由

- **M1 単独**: `delta = 60_000ms` → `threshold = 60_000ms` → 1x = 実時間、10x = 10 倍速。既存 UX を維持
- **H1 以上**: `delta >= 3_600_000ms` → `threshold = COARSE_BAR_MS = 1000ms` → 1x = 1 bar/sec、10x = 10 bars/sec。D1 リプレイが実用時間内に収まる
- **M1 + D1 混在**: min = M1 の境界 60_000ms → M1 基準で進行し、D1 は越境 Tick で自然追従（実質 1440 Tick/本だが、M1 観察が主の場合は許容）

#### 15.2.4 既知のトレードオフ

| # | トレードオフ | 受容理由 |
|---|---|---|
| 1 | H1 境界の 360× 不連続性 | M30 (6 分/本) と H1 (1 秒/本) の間で速度がジャンプ。単一 timeframe 運用では無関係。混在は tooltip で明示 |
| 2 | 混在 M1+D1 での D1 実質停止 | M1 観察が主で D1 は補助という想定。D1 主体で観察したい場合は D1 単独ペインに切替 |
| 3 | 高速再生時のデータ粒度劣化 | 10x では 1 フレーム間に複数バー分の trades が集中するが、cursor ベース drain で欠損は発生しない |
| 4 | 境界判定 `>=` の決め打ち | H1 を粗補正側に入れるか実時間側に残すかは議論の余地あり。現状は「H1 で 60 分待ち」を避けるため `>=` 固定。実機検証で要望があれば再検討 |

#### 15.2.5 `FireStatus` 3 状態の必要性

`next_time_after()` が `None` を返すケースは 2 種類あり、`Option<u64>` だけでは区別できない:

- **(A) buffer 末尾**: 全データ投入済み = 終端 → `Paused` へ遷移
- **(B) buffer 未到着**: バックフィル中で klines がまだ空 → 待機

これを `FireStatus::{Ready(u64), Pending, Terminal}` enum で明示的に区別する。`Pending` と `Terminal` を混同して `Paused` に落とすと、mid-replay ペイン追加時に全体が Pause する重大バグになる。

#### 15.2.6 「穴 A」と「穴 B」

統一 Tick の設計で発見された 2 つの落とし穴:

- **穴 A（未達 Tick でも trades を流す）**: `Ready(t)` 経路で `virtual_elapsed_ms < threshold` のときも、まず `drain_all_trade_buffers()` を実行してから return する。これを忘れると M1 単独の毎フレーム trades 流入が止まる
- **穴 B（バックフィル中 stream の除外）**: `drain_all_trade_buffers()` は `pending_trade_streams` に含まれる stream をスキップする。これを忘れると新規ペインのバックフィル途中で過去分 trades が既存 chart に漏れる

どちらも §7 のアルゴリズム本体に織り込み済み（`drain_all_trade_buffers` [src/replay.rs:394-422](../src/replay.rs#L394-L422) 参照）。

### 15.3 Phase 8 レビュー駆動の設計不整合修正

Phase 6/7 完了後のコードレビューで発覚した 4 件のバグを修正した。いずれも TDD（RED → GREEN）で修正し、`§12.3` の設計不変条件として固定化した:

| # | 問題 | 修正 | 不変条件 |
|---|---|---|---|
| 1 | `KlineChart::set_basis()` が `replay_kline_buffer` を再初期化せず、mid-replay の timeframe 変更が壊れる | `set_basis()` 末尾で `replay_kline_buffer` が `Some` なら `klines.clear()` + `cursor = 0`。Some/None 状態は維持 | #1 |
| 2 | `TradesBatchReceived` が `or_insert_with` で orphan trade buffer を自己復活させ、mid-replay 削除後に無限 flap ループ | `PlaybackState::ingest_trades_batch` を新設し、登録済み stream のみ accept。未登録は黙って drop | #2 |
| 3 | Tooltip 文言「H1 以下: 実時間連動 / H4 以上: 1 バー/sec × speed」が `COARSE_CUTOFF_MS = 3_600_000ms`（H1 境界）と不整合 | 「M30 以下: 実時間連動 × speed / H1 以上: 1 バー/秒 × speed」に修正。`coarse_cutoff_boundary_matches_h1_in_ms` テストで固定化 | #4 |
| 4 | `Message::Sidebar::TickerSelected` → `init_focused_pane()` が `Task::none()` を返すケース（heatmap-only）で `SyncReplayBuffers` chain が発火しない | `Message::Sidebar` ハンドラ return path に明示的に `.chain(Task::done(SyncReplayBuffers))` を追加 | #3 |

これらの修正から得られた教訓が §12.3 の 4 件の不変条件である。新機能追加時は該当不変条件に抵触しないか必ず確認すること。

### 15.4 残課題

以下は Phase 8 時点で認識されているが、現状スコープ外として残している:

| 項目 | 影響 | 将来の解決策 |
|---|---|---|
| linear advance フォールバック経路の `pending_trade_streams` 未対応 | heatmap-only リプレイ中の mid-replay ペイン追加で pending ガードが効かない | `fire_status()` が `None` を返さない形に統一すれば解消 |
| fetch タスクの abort 経路不在 | orphan 削除後も残存 fetch が自然完了まで稼働 | `trade_fetch_handles: HashMap<StreamKind, AbortHandle>` を追加 |
| クリッピング警告 6 件 | 既存コード起因、リプレイとは無関係 | 別 PR で clippy 一斉対応 |
| Tachibana D1 の実機 E2E 検証 | Fix 1 (timeframe 変更) / Fix 4 (heatmap ticker 選択) は実動作確認が未完 | 実機検証タスクで追跡 |

---

**変更時の注意**: §12.3 の不変条件を破る変更を行う場合は、必ず該当テスト（`coarse_cutoff_boundary_matches_h1_in_ms` 等）の見直しと本仕様書の同期更新を行うこと。設計代替案を再検討する場合は §15.2 の「却下理由」と照らし合わせ、同じ議論を再発明しないこと。
