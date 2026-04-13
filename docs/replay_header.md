# リプレイ機能 仕様書

**最終更新**: 2026-04-13
**対象バージョン**: `sasa/step` ブランチ (Phase 1〜8 + Tachibana Phase 1〜3 + R3 アーキテクチャ刷新 完了)
**関連ドキュメント**:
- [docs/plan/replay_redesign.md](plan/replay_redesign.md) — R1〜R3 リファクタ計画と実装ログ
- [docs/plan/archive/tachibana_replay.md](plan/archive/tachibana_replay.md) — 立花証券 D1 対応の実装経緯（完了、アーカイブ）
- [docs/plan/archive/replay_unified_step.md](plan/archive/replay_unified_step.md) — 統一 Tick ハンドラ設計メモ（完了、アーカイブ）
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
| EventStore ベース再生 | kline・trades を `EventStore` に一括ロードし、仮想時刻スライスで dispatch |
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
| **VirtualClock** | 仮想時刻の単一ソース。`anchor_wall` と `speed` から現在の仮想時刻を計算する |
| **ClockStatus** | クロックの状態。`Playing` / `Paused` / `Waiting`（データ待ち）の 3 状態 |
| **EventStore** | stream ごとに kline + trades を保持するストア。`is_loaded` / `klines_in` / `trades_in` で問い合わせる |
| **dispatch_tick** | 仮想時刻スライスを EventStore から抽出する純粋関数。副作用なし |
| **DispatchResult** | `dispatch_tick` の戻り値。`kline_events` / `trade_events` / `reached_end` を含む |
| **Waiting 状態** | データ未ロードのためクロックが自動停止している状態（旧 Loading に相当） |

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
| ⏮ / ⏭ (Step) | 無効 | `clock.is_some()` && `!Waiting` で有効 |
| ▶⏸ (Play/Pause/Resume) | 無効 | 状態依存で Play/Pause/Resume |
| 速度ボタン | 無効 | 有効、`1x → 2x → 5x → 10x → 1x` 循環 |
| `Loading...` | 非表示 | `is_loading()` == true で表示 |

日時テキストは `iced::text_input` で `on_input` を渡さないことで read-only を実現する。

### 3.2 入力フォーマット

```
YYYY-MM-DD HH:MM    (UTC として解釈)
```

不正な場合は `ParseRangeError::{InvalidStartFormat, InvalidEndFormat, StartAfterEnd}` を返し、Toast 通知で中断する（`src/replay/mod.rs` [`parse_replay_range`](../src/replay/mod.rs)）。

### 3.3 キーバインド

| キー | 動作 |
|---|---|
| `F5` | `ReplayMessage::ToggleMode` をディスパッチ |
| `Escape` | `Message::GoBack`（リプレイとは独立、レイヤー閉じ）|

キーバインドは `subscription()` 内の `keyboard::listen().filter_map()` で処理する。

### 3.4 リプレイ中の UI 制約

- ペインの **位置移動 (drag / resize)** は無効化
- ペインの **追加 / 削除 / timeframe 変更 / ticker 変更** は許容（§8 参照）
- `Heatmap` / `ShaderHeatmap` / `Ladder` ペインには `"Replay: Depth unavailable"` オーバーレイを表示

---

## 4. 状態モデル

### 4.1 ReplayState ([src/replay/mod.rs](../src/replay/mod.rs))

```rust
pub struct ReplayState {
    pub mode: ReplayMode,
    pub range_input: ReplayRangeInput,
    pub clock: Option<VirtualClock>,
    pub event_store: EventStore,
    pub active_streams: HashSet<StreamKind>,
}

pub enum ReplayMode { Live, Replay }

pub struct ReplayRangeInput {
    pub start: String,
    pub end: String,
}
```

- `clock` は `Play` 押下で `Some` になり、`Live` 復帰で `None` に戻る
- `event_store` は stream ごとにロード済みの kline / trades を保持する（play 開始時に初期化）
- `active_streams` は再生中の全 `StreamKind` の集合
- `toggle_mode()` で Replay → Live に戻す際は `clock = None`, `event_store = EventStore::new()`, `active_streams = HashSet::new()`, `range_input = default` にリセット

### 4.2 VirtualClock ([src/replay/clock.rs](../src/replay/clock.rs))

```rust
pub struct VirtualClock {
    pub now_ms: u64,                    // 現在の仮想時刻
    pub anchor_wall: Option<Instant>,   // Playing 開始時の壁時計
    pub speed: f32,                     // 1.0 | 2.0 | 5.0 | 10.0
    pub status: ClockStatus,
    pub range: Range<u64>,              // (start_ms, end_ms)
}

pub enum ClockStatus {
    Playing,   // 仮想時刻が進行中
    Paused,    // ユーザーが一時停止
    Waiting,   // データ未ロードのため自動停止
}
```

- `now_ms`: `anchor_wall` と `speed` から計算する。`Playing` 以外は固定値
- `play(wall_now)`: `anchor_wall = Some(wall_now)`, `status = Playing`
- `pause()`: `now_ms` を現在値に固定し `status = Paused`, `anchor_wall = None`
- `seek(target_ms)`: `now_ms = target_ms`, クロックを `Paused` で静止させる
- `current_time(wall_now)`: `Playing` なら `now_ms + (wall_now - anchor_wall) * speed` を計算。他の状態は `now_ms`

### 4.3 EventStore ([src/replay/store.rs](../src/replay/store.rs))

```rust
pub struct EventStore {
    // stream ごとに SortedVec<Trade> + SortedVec<Kline> を保持（内部実装）
}

impl EventStore {
    pub fn new() -> Self;
    pub fn ingest_loaded(&mut self, stream: StreamKind, range: Range<u64>, data: LoadedData);
    pub fn is_loaded(&self, stream: &StreamKind, range: Range<u64>) -> bool;
    pub fn trades_in(&self, stream: &StreamKind, range: Range<u64>) -> Vec<&Trade>;
    pub fn klines_in(&self, stream: &StreamKind, range: Range<u64>) -> Vec<&Kline>;
}

pub struct LoadedData {
    pub klines: Vec<Kline>,
    pub trades: Vec<Trade>,
}
```

- `klines_in` / `trades_in` は **半開区間** `[start, end)` を返す（`time >= start && time < end`）
- `is_loaded` は指定 stream + range に対してデータが `ingest_loaded` 済みかを返す
- ロード前に `klines_in` を呼んでも空 Vec が返るだけでパニックしない

### 4.4 DispatchResult ([src/replay/dispatcher.rs](../src/replay/dispatcher.rs))

```rust
pub struct DispatchResult {
    pub current_time: u64,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub reached_end: bool,
}
```

Tick ごとに `dispatch_tick` が生成する。`current_time` は今フレームの仮想時刻スナップショット。

---

## 5. メッセージとイベント

### 5.1 ReplayMessage ([src/replay/mod.rs](../src/replay/mod.rs))

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
| `KlinesLoadCompleted(StreamKind, Range<u64>, Vec<Kline>)` | kline 一括ロード完了（R3）|
| `TradesLoadCompleted(StreamKind, Range<u64>, Vec<Trade>)` | trades 一括ロード完了（将来）|
| `DataLoaded` | *(stub, 旧互換)* 現在は no-op |
| `DataLoadFailed(String)` | プリフェッチ失敗 |
| `TradesBatchReceived(StreamKind, Vec<Trade>)` | *(stub, 旧互換)* 現在は no-op |
| `TradesFetchCompleted(StreamKind)` | *(stub, 旧互換)* 現在は no-op |
| `SyncReplayBuffers` | *(stub, 旧互換)* 現在は no-op |

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
  │     - kline ペインは KlineChart::new() + reset_for_seek()
  │     - Heatmap 等はクリア
  ├─ 3. ReplayState::start(start_ms, end_ms, wall_now) で
  │     - clock = Some(VirtualClock { status: Waiting, ... })
  │     - event_store = EventStore::new()
  │     - active_streams = 現在の全 StreamKind
  ├─ 4. 全 active_streams に対して Task::perform(loader::load_klines(stream, range), ...)
  │     → Message::Replay(KlinesLoadCompleted(stream, range, klines))
  └─ 5. subscription() の次評価で exchange_streams が外れ WS 切断

[KlinesLoadCompleted(stream, range, klines) 受信]
  ├─ event_store.ingest_loaded(stream, range, LoadedData { klines, .. })
  ├─ dashboard.ingest_replay_klines(&stream, &klines, main_window)
  │     → 各 pane の KlineChart::ingest_historical_klines() を呼び出し
  └─ ReplayState::on_klines_loaded(stream, range, klines, wall_now)
        全 active_streams が is_loaded → clock.play(wall_now) で再生開始
        未完の stream あり → clock は Waiting のまま
```

### 6.2 Tick ループ

```
[Message::Tick(now)]
  └─ replay.clock が Some && is_replay() の場合:
       dispatch = dispatch_tick(clock, &event_store, &active_streams, now)
       
       for (stream, klines) in &dispatch.kline_events:
         dashboard.ingest_replay_klines(&stream, &klines, main_window)
       
       for (stream, trades) in &dispatch.trade_events:
         dashboard.ingest_trades(stream, trades)
       
       if dispatch.reached_end:
         clock.pause()
```

`dispatch_tick` のアルゴリズムは §7.1 を参照。

### 6.3 mid-replay バックフィル

```
[ペイン操作: Sidebar / Dashboard / Pane API]
  ├─ streams を mutate
  └─ active_streams に新 stream を追加 + Task::perform(load_klines(...))

[KlinesLoadCompleted(new_stream, ...) 受信]
  ├─ event_store.ingest_loaded(new_stream, ...)
  ├─ dashboard.ingest_replay_klines(&new_stream, &klines, main_window)
  └─ clock が Waiting && 全 streams ロード完了 → clock.play(wall_now)
     既存 streams は再生を止めない（EventStore は既ロード済み）
```

### 6.4 StepForward / StepBackward

```
[StepForward]
  ├─ 全 active_streams の klines_in(stream, current_time..end_time) で
  │   current_time より大きい最小の kline.time を探す
  └─ clock.seek(next_time) で Paused 停止

[StepBackward]
  ├─ 全 active_streams の klines_in(stream, 0..current_time) で
  │   current_time より小さい最大の kline.time を探す
  ├─ dashboard 全 pane に reset_for_seek() を呼んでチャートをクリア
  ├─ event_store.klines_in(stream, start..prev_time) を
  │   dashboard.ingest_replay_klines() で再挿入
  └─ clock.seek(prev_time) で Paused 停止
     (kline 再フェッチは行わない — EventStore から再構成)
```

### 6.5 Live 復帰

```
[ReplayMessage::ToggleMode (Replay → Live)]
  ├─ clock = None, event_store = EventStore::new(), active_streams = {}
  ├─ range_input = default
  ├─ Dashboard::rebuild_for_live() で content リビルド
  └─ subscription() 次評価で exchange_streams 復帰 → WS 自動再購読
```

---

## 7. 再生エンジン

### 7.1 dispatch_tick ([src/replay/dispatcher.rs](../src/replay/dispatcher.rs))

```rust
pub fn dispatch_tick(
    clock: &mut VirtualClock,
    store: &EventStore,
    active_streams: &HashSet<StreamKind>,
    wall_now: Instant,
) -> DispatchResult;
```

**アルゴリズム**:

1. `clock.current_time(wall_now)` で仮想時刻スナップショット `t` を取得
2. `clock.status != Playing` → 空の `DispatchResult` を返す（Paused / Waiting は進行しない）
3. `t >= clock.range.end` → `reached_end = true`, `clock.pause()`
4. 各 `stream` in `active_streams`:
   - `store.klines_in(stream, prev_t..t)` → `kline_events` に追加
   - `store.trades_in(stream, prev_t..t)` → `trade_events` に追加
5. `kline_events` / `trade_events` を含む `DispatchResult` を返す

**状態変化なし**: `dispatch_tick` は純粋関数に近い設計で、`clock.pause()` / `clock.now_ms` 更新のみが副作用。`EventStore` への書き込みは行わない。

### 7.2 Pause / Resume / CycleSpeed

- `Pause`: `clock.pause()` — `now_ms` を現在値に固定、`anchor_wall = None`
- `Resume`: `clock.play(wall_now)` — `anchor_wall = Some(wall_now)` で再開
- `CycleSpeed`: `SPEEDS = [1.0, 2.0, 5.0, 10.0]` を `(i+1) % 4` で循環
- `speed_label()`: `if speed == floor(speed) { "Nx" } else { "N.Nx" }` を返す

### 7.3 Waiting 状態の自動解除

`ClockStatus::Waiting` はデータロード待ち中を示す。`KlinesLoadCompleted` 受信時に `event_store.is_loaded()` で全 streams のロード完了を確認し、全て完了していれば `clock.play(wall_now)` で自動再開する。

---

## 8. mid-replay ペイン操作

### 8.1 許容される操作

- SplitPane / ClosePane
- Timeframe 変更
- Ticker 変更
- Sidebar からの TickerSelected
- HTTP Pane API 経由の全操作（§11.2）

### 8.2 新ペインのバックフィル

1. ペイン追加 → 新 `StreamKind` を `active_streams` に追加
2. `clock` が Playing なら `Waiting` に遷移（または Paused のまま）
3. `Task::perform(load_klines(new_stream, range), KlinesLoadCompleted)` を発火
4. `KlinesLoadCompleted` 受信 → `event_store.ingest_loaded` → 全 streams ロード完了で `clock.play()`

既存の streams / EventStore エントリは変更されず、再生は継続される。

### 8.3 reset_for_seek / ingest_historical_klines

`KlineChart` の新 API:

```rust
// チャートデータを全クリアしてシークの準備をする（StepBackward, ticker 変更等）
pub fn reset_for_seek(&mut self);

// EventStore から取り出した klines をチャートに挿入する
pub fn ingest_historical_klines(&mut self, klines: &[Kline]);
```

旧 `enable_replay_mode` / `replay_advance` / `replay_kline_buffer` は廃止された。

---

## 9. WebSocket 制御

`subscription()` の構造:

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
2. **離散ステップ**: StepForward / StepBackward が EventStore から次/前 kline timestamp を検索し、休場日（土日祝）を自動スキップ
3. **Play 時の挙動**: `dispatch_tick` が各 Tick で kline スライスを返すため、D1 リプレイも同一経路で動作

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
| GET | `/api/pane/list` | — | 全ペイン + EventStore ロード状態を返す |
| POST | `/api/pane/split` | `{"pane_id": "<uuid>", "axis": "Vertical"\|"Horizontal"}` | ペイン分割 |
| POST | `/api/pane/close` | `{"pane_id": "<uuid>"}` | ペイン削除 |
| POST | `/api/pane/set-ticker` | `{"pane_id": "<uuid>", "ticker": "BinanceLinear:BTCUSDT"}` | ticker 差し替え |
| POST | `/api/pane/set-timeframe` | `{"pane_id": "<uuid>", "timeframe": "M1"\|...\|"D1"}` | timeframe 変更 |

#### Sidebar 経由 (`/api/sidebar/*`)

| メソッド | パス | リクエストボディ | 用途 |
|---|---|---|---|
| POST | `/api/sidebar/select-ticker` | `{"pane_id": "<uuid>", "ticker": "...", "kind": "..." or null}` | Sidebar::TickerSelected 経路の再現（E2E テスト用）|

### 11.3 レスポンススキーマ

#### `ReplayStatus` ([src/replay/mod.rs](../src/replay/mod.rs))

```rust
pub struct ReplayStatus {
    pub mode: String,                    // "Live" | "Replay"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,          // "Waiting" | "Playing" | "Paused"

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

`status` は `ClockStatus` を文字列化したもの。`Waiting` は旧 `Loading` に相当する。

**Live モード レスポンス例**（clock 未開始時は省略フィールドが消える）:

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
      "link_group": "A" | null
    }
  ]
}
```

旧 `replay_buffer_ready` / `replay_buffer_cursor` / `replay_buffer_len` / `pending_trade_streams` / `trade_buffer_streams` フィールドは R3 で廃止された。

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

# ペイン一覧
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
| `SPEEDS` | `[1.0, 2.0, 5.0, 10.0]` | [src/replay/mod.rs](../src/replay/mod.rs) | 再生速度テーブル |
| Kline backfill bars | `450` | [src/main.rs](../src/main.rs) | リプレイ開始前に追加フェッチする本数 |
| API port (default) | `9876` | [src/replay_api.rs](../src/replay_api.rs) | `FLOWSURFACE_API_PORT` で上書き |
| HTTP buffer size | `8192` | [src/replay_api.rs](../src/replay_api.rs) | リクエスト読み取りバッファ |
| mpsc channel bound | `32` | [src/replay_api.rs](../src/replay_api.rs) | API → iced キュー |

### 12.2 時間範囲

- **Kline フェッチ範囲**: `(start_ms - 450 * tf.to_milliseconds(), end_ms)` — リプレイ開始前のコンテキストも 450 本含める
- **Trades フェッチ範囲**: `(start_ms, end_ms)` — Binance のみ `fetch_trades_batched` を Straw ストリームで消費（将来実装）
- **EventStore クエリ範囲**: 全て **半開区間** `[start, end)` — `time >= start && time < end`

### 12.3 設計上の不変条件

| # | 不変条件 | 破壊したときの症状 |
|:-:|---|---|
| 1 | `dispatch_tick` は `EventStore` に書き込まない。読み取り専用 | 同一スライスが二重 dispatch される、またはデータが消える |
| 2 | `klines_in` / `trades_in` は半開区間 `[start, end)` で返す | 境界 kline が重複挿入されるか、スキップされる |
| 3 | `KlinesLoadCompleted` 受信時は必ず `event_store.ingest_loaded` の後に `ingest_replay_klines` を呼ぶ | チャートに kline が届かない（EventStore にはあるが UI に反映されない）|
| 4 | `reset_for_seek()` は StepBackward と ticker/timeframe 変更時に必ず呼ぶ | 古い kline が残ったまま新データが重なる |

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
| Trades の EventStore 統合（R4） | 現在 Trades は旧 fetch_batched 経路のまま（将来対応）|

### 13.2 既知の制限

| # | 制限 | 影響 |
|---|---|---|
| 1 | Trades の EventStore 未統合 | `trade_events` は現時点では空。kline のみ dispatch される |
| 2 | fetch タスクの abort 経路不在 | バックフィル中にペインを削除しても fetch タスクは自然完了まで稼働 |
| 3 | Binance 以外の trades 未対応 | Bybit / Hyperliquid / OKX / MEXC / Tachibana では Kline のみの再生 |

---

## 14. 実装ファイルマップ

### 14.1 主要ファイル

| ファイル | 責務 |
|---|---|
| [src/replay/mod.rs](../src/replay/mod.rs) | `ReplayState` / `ReplayMessage` / `ReplayMode` / `ReplayStatus` / `parse_replay_range` / `format_current_time` / `SPEEDS` |
| [src/replay/clock.rs](../src/replay/clock.rs) | `VirtualClock` / `ClockStatus` / `play` / `pause` / `seek` / `current_time` |
| [src/replay/store.rs](../src/replay/store.rs) | `EventStore` / `LoadedData` / `ingest_loaded` / `is_loaded` / `klines_in` / `trades_in` |
| [src/replay/dispatcher.rs](../src/replay/dispatcher.rs) | `dispatch_tick` / `DispatchResult` |
| [src/replay/loader.rs](../src/replay/loader.rs) | `load_klines` / `KlineLoadResult` / `fetch_all_klines` |
| [src/replay_api.rs](../src/replay_api.rs) | HTTP サーバー (`tokio::net::TcpListener` + 手動パース) / `ApiCommand` / `PaneCommand` / `ReplySender` / ルーティング |
| [src/main.rs](../src/main.rs) | `Flowsurface` への `replay` フィールド、`Message::Replay` / `Message::ReplayApi` ハンドラ、`view_replay_header()`、`subscription()`、`handle_pane_api()` 各メソッド |
| [src/screen/dashboard.rs](../src/screen/dashboard.rs) | `prepare_replay()` / `rebuild_for_live()` / `collect_trade_streams()` / `ingest_replay_klines()` / `ingest_trades()` |
| [src/screen/dashboard/pane.rs](../src/screen/dashboard/pane.rs) | `rebuild_content_for_replay()` / `rebuild_content_for_live()` / `ingest_replay_klines()` / `reset_for_seek()` / `insert_hist_klines()` / Heatmap の Depth unavailable オーバーレイ |
| [src/chart/kline.rs](../src/chart/kline.rs) | `ingest_historical_klines()` / `reset_for_seek()` |
| [src/connector/fetcher.rs](../src/connector/fetcher.rs) | Tachibana D1 range フィルタ分岐 |

### 14.2 テスト

- `src/replay/store.rs` のユニットテスト: `ingest_loaded` / `is_loaded` / `klines_in` / `trades_in` 動作検証
- `src/replay/clock.rs` のユニットテスト: `ClockStatus` 遷移 / `current_time` 計算
- `src/replay/dispatcher.rs` のユニットテスト: `dispatch_tick` のスライス抽出 / 終端判定
- `src/replay/loader.rs` のユニットテスト: `EventStore` 直接操作で loader 動作を検証
- `src/chart/kline.rs` のユニットテスト: `ingest_historical_klines` / `reset_for_seek`
- `src/replay_api.rs` のルーター/パーサーテスト
- E2E テスト: `tests/` ディレクトリ配下、HTTP API 経由でシナリオ検証

`cargo test --bin flowsurface` で全件実行する。

---

## 15. 付録: 実装履歴と設計判断

### 15.1 実装フェーズ

| Phase | 内容 | 参照 |
|---|---|---|
| Phase 1 | ヘッダーバー UI（見た目のみ）| 本書 §3 |
| Phase 2 | リプレイデータのプリフェッチ | 本書 §6.1 |
| Phase 3 | リプレイ再生エンジン (Tick + 段階挿入) | 本書 §7 |
| Phase 4 | ライブ復帰・巻き戻し・速度切替 | 本書 §7.2 |
| Phase 5 | ローカル HTTP 制御 API | 本書 §11 |
| Phase 6 | 統一 Tick ハンドラ（D1 分岐撤廃） | [docs/plan/archive/replay_unified_step.md](plan/archive/replay_unified_step.md) |
| Phase 7 | mid-replay ペイン操作許容 | 本書 §8 |
| Phase 8 | レビュー駆動の設計不整合修正 | 本書 §12.3 (旧) |
| Tachibana Phase 1〜3 | 立花証券 D1 対応 | [docs/tachibana_spec.md §8](tachibana_spec.md) / [docs/plan/archive/tachibana_replay.md](plan/archive/tachibana_replay.md) |
| **R3: アーキテクチャ刷新** | `PlaybackState` / `FireStatus` / `process_tick` / `ReplayKlineBuffer` / `TradeBuffer` を全廃し、`VirtualClock` + `EventStore` + `dispatch_tick` に置き換え | [docs/plan/replay_redesign.md](plan/replay_redesign.md) |

### 15.2 R3 刷新の設計判断

#### 15.2.1 旧アーキテクチャの問題点

Phase 8 までの実装（`process_tick` + `COARSE_CUTOFF_MS` 境界 + `FireStatus` + buffer ベース）は動作したが、以下の構造的問題を抱えていた:

1. **`FireStatus` の 3 状態管理**: `None(buffer末尾)` と `None(バックフィル中)` を区別するために enum が必要で、全 chart を走査するたびに `min` 計算が複雑化した
2. **`TradeBuffer` の cursor ベース管理**: `advance_cursor_to` / `drain_until` の cursor 不変条件が壊れると trades が重複 or 欠損し、デバッグが困難だった
3. **`SyncReplayBuffers` の 2 系統 chain**: streams 変更を確実に追従させるため、`Message::Dashboard` 末尾と `Message::Sidebar::TickerSelected` の両方に chain が必要で、追加を忘れると silent バグになった
4. **`is_replay_mode()` ガード**: 遅延完了したライブフェッチが `replay_kline_buffer` を上書きするのを防ぐガードが必要で、デッドロックのトリップワイヤーだった

#### 15.2.2 R3 の設計方針

- **EventStore**: データを「ロード済み範囲」として蓄積し、クエリは純粋な範囲検索。cursor 管理ゼロ
- **VirtualClock**: 仮想時刻を壁時計との差分で計算。`anchor_wall` + `speed` で状態が決まり、`ClockStatus` で明示的に状態を表現
- **dispatch_tick**: 副作用なしの純粋関数に近い設計。Tick ごとに「今の仮想時刻」と「前回の仮想時刻」の差分スライスを EventStore から取得するだけ
- **Waiting 状態**: データ未ロード時はクロックが自動停止し、ロード完了で自動再開。旧 `Loading` 状態に相当するが、外部から `SyncReplayBuffers` を発火させる必要がない

#### 15.2.3 廃止されたコンポーネント

| 廃止コンポーネント | 代替 |
|---|---|
| `PlaybackState` | `VirtualClock` + `EventStore` + `active_streams` |
| `FireStatus` enum | `dispatch_tick` の戻り値 `reached_end: bool` |
| `process_tick` | `dispatch_tick` (stateless) |
| `TradeBuffer` | `EventStore::trades_in` (half-open range query) |
| `ReplayKlineBuffer` | `KlineChart::ingest_historical_klines` |
| `enable_replay_mode` / `disable_replay_mode` / `is_replay_mode` | 廃止（モード管理不要）|
| `replay_advance` | `ingest_historical_klines` で EventStore のスライスを直接渡す |
| `SyncReplayBuffers` | stub (no-op)、mid-replay バックフィルは `KlinesLoadCompleted` 経路に統一 |
| `fire_status` (Dashboard method) | 廃止 |
| `collect_new_replay_klines` | 廃止 |
| `rebuild_for_step_backward` | `reset_for_seek` + `ingest_historical_klines` に統一 |
| `COARSE_CUTOFF_MS` / `COARSE_BAR_MS` | 廃止（dispatch_tick は実時間連動のみ）|

### 15.3 残課題

| 項目 | 影響 | 将来の解決策 |
|---|---|---|
| Trades の EventStore 統合（R4） | `trade_events` が空のため trades の再生が無効 | `load_trades` → `TradesLoadCompleted` → `EventStore::ingest_loaded` 経路を実装 |
| fetch タスクの abort 経路不在 | バックフィル中にペインを削除しても fetch が自然完了まで稼働 | `AbortHandle` を `active_streams` と紐づけて管理 |

---

**変更時の注意**: §12.3 の不変条件（特に半開区間の一貫性）を破る変更を行う場合は、`EventStore` のテストを全件確認すること。R3 の設計代替案を再検討する場合は §15.2 の「廃止理由」と照らし合わせ、同じ議論を再発明しないこと。
