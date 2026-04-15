# リプレイ機能 仕様書

**最終更新**: 2026-04-14
**対象バージョン**: `sasa/step` ブランチ (Phase 1〜8 + Tachibana Phase 1〜3 + R3 アーキテクチャ刷新 + Fixture 直接起動 + Auto-play タイムアウト廃止 完了)
**関連ドキュメント**:
- [docs/plan/replay_redesign.md](plan/replay_redesign.md) — R1〜R3 リファクタ計画と実装ログ
- [docs/plan/replay_bar_step_loop.md](plan/replay_bar_step_loop.md) — StepClock / EventStore / dispatch_tick 設計
- [docs/plan/replay_fixture_direct_boot.md](plan/replay_fixture_direct_boot.md) — Fixture 直接起動 (auto-play) 設計
- [docs/plan/replay_auto_play_no_timeout.md](plan/replay_auto_play_no_timeout.md) — Auto-play タイムアウト廃止（イベント駆動化）
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
| EventStore ベース再生 | kline・trades を `EventStore` に一括ロードし、バーステップ単位で dispatch |
| mid-replay ペイン操作 | リプレイ中のペイン追加・削除・timeframe / ticker 変更 |
| HTTP 制御 API | `127.0.0.1:9876` でリプレイ・ペイン操作を外部から駆動 |
| E2E テスト支援 | `POST /api/app/save` で状態をディスク保存 |
| **起動時自動再生** | `saved-state.json` に replay 構成が含まれる場合、全ペイン Ready になった瞬間に自動 Play |

### 1.2 非ゴール

- 板情報（Depth）の再生 — 取引所 API が過去スナップショットを提供しない
- Comparison ペインのリプレイ — 複数銘柄同期は将来課題
- リプレイ中の Layout 切替 — `active_dashboard()` が変わる動作は未定義
- インジケータ再計算 / 仮想売買 — 別タスク

---

## 2. 用語

| 用語 | 定義 |
|---|---|
| **Live モード** | WebSocket から直接ストリーム受信する通常状態 |
| **Replay モード** | 過去データを仮想時刻で再生する状態 |
| **仮想時刻 (`current_time`)** | Replay 中に進行する Unix ms タイムスタンプ（バー境界値） |
| **プリフェッチ** | Play 押下後、再生開始までに行う過去データの一括取得 |
| **バックフィル** | mid-replay でペインを追加した際の遅延フェッチ |
| **Tick** | `iced::window::frames()` が発火するフレームイベント (~60fps) |
| **StepClock** | 離散バーステップクロック。`now_ms` は常にバー境界値。`tick(wall_now)` で 1 ステップ進める |
| **ClockStatus** | クロックの状態。`Playing` / `Paused` / `Waiting`（データ待ち）の 3 状態 |
| **EventStore** | stream ごとに kline + trades を保持するストア。`is_loaded` / `klines_in` / `trades_in` で問い合わせる |
| **dispatch_tick** | 仮想時刻スライスを EventStore から抽出するステートレスな関数 |
| **DispatchResult** | `dispatch_tick` の戻り値。`kline_events` / `trade_events` / `reached_end` を含む |
| **Waiting 状態** | データ未ロードのためクロックが自動停止している状態 |
| **pending_auto_play** | 起動時 fixture 復元後、全ペイン Ready になった瞬間に Play を自動発火するための transient フラグ |

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

不正な場合は `ParseRangeError::{InvalidStartFormat, InvalidEndFormat, StartAfterEnd}` を返し、Toast 通知で中断する（[src/replay/mod.rs](../src/replay/mod.rs) `parse_replay_range`）。

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
    /// バーステップクロック。Play 開始後 Some になる。
    pub clock: Option<StepClock>,
    pub event_store: EventStore,
    pub active_streams: HashSet<StreamKind>,
    /// 起動時 fixture 復元の結果として次の「全ペイン Ready」で Play を自動発火する。
    /// 一度発火したら false に戻す。永続化しない。
    pub pending_auto_play: bool,
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
- `toggle_mode()` で Replay → Live に戻す際は `clock = None`, `event_store = EventStore::new()`, `active_streams = HashSet::new()`, `range_input = default`, `pending_auto_play = false` にリセット
- `pending_auto_play` は **永続化しない**（再起動のたびに起動時ロジックで再評価する）

### 4.2 StepClock ([src/replay/clock.rs](../src/replay/clock.rs))

```rust
pub struct StepClock {
    now_ms: u64,                    // 現在の仮想時刻（常にバー境界値）
    next_step_at: Option<Instant>,  // 次のステップを発火する wall 時刻
    step_size_ms: u64,              // 1 ステップで進む仮想時刻幅（min active timeframe ms）
    base_step_delay_ms: u64,        // 1x speed での wall delay (= BASE_STEP_DELAY_MS = 1000ms)
    speed: f32,                     // 1.0 | 2.0 | 5.0 | 10.0
    status: ClockStatus,
    range: Range<u64>,              // リプレイ範囲 (start_ms, end_ms)
}

pub enum ClockStatus {
    Paused,    // 停止中
    Playing,   // 再生中
    Waiting,   // EventStore がデータロード中
}
```

**StepClock の動作**（連続時刻クロックではなく **離散バーステップ** モデル）:

- `new(start_ms, end_ms, step_size_ms)`: 初期状態は `Paused`、`now_ms = start_ms`
- `play(wall_now)`: `Playing` に遷移し、`next_step_at = wall_now + step_delay_ms()` を設定
- `pause()`: `Paused` に遷移、`next_step_at = None`
- `set_waiting()`: `Waiting` に遷移（データロード待ち）
- `resume_from_waiting(wall_now)`: `Waiting → Playing` に復帰
- `seek(target_ms)`: `now_ms` を指定値（step_size 境界にスナップ）に設定し `Paused` で停止
- `tick(wall_now) -> Range<u64>`: `wall_now >= next_step_at` のとき `now_ms` を 1 ステップ進め、emit 範囲 `[prev_ms, new_ms)` を返す。まだ発火タイミングでなければ空 Range を返す
- `step_delay_ms()`: `base_step_delay_ms / speed` (ms)。例: 2x なら 500ms/bar

**現在時刻の参照**: `clock.now_ms()` は常にバー境界値を返す（連続補間なし）。

### 4.3 EventStore ([src/replay/store.rs](../src/replay/store.rs))

```rust
pub struct EventStore {
    // stream ごとに SortedVec<Trade> + SortedVec<Kline> を保持（内部実装）
}

impl EventStore {
    pub fn new() -> Self;
    pub fn ingest_loaded(&mut self, stream: StreamKind, range: Range<u64>, data: LoadedData);
    pub fn is_loaded(&self, stream: &StreamKind, range: Range<u64>) -> bool;
    pub fn trades_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Trade];
    pub fn klines_in(&self, stream: &StreamKind, range: Range<u64>) -> &[Kline];
}

pub struct LoadedData {
    pub klines: Vec<Kline>,
    pub trades: Vec<Trade>,
}
```

- `klines_in` / `trades_in` は **半開区間** `[start, end)` を返す（`time >= start && time < end`）
- `is_loaded` は指定 stream + range に対してデータが `ingest_loaded` 済みかを返す
- ロード前に `klines_in` を呼んでも空スライスが返るだけでパニックしない

### 4.4 DispatchResult ([src/replay/dispatcher.rs](../src/replay/dispatcher.rs))

```rust
pub struct DispatchResult {
    pub current_time: u64,
    pub kline_events: Vec<(StreamKind, Vec<Kline>)>,
    pub trade_events: Vec<(StreamKind, Vec<Trade>)>,
    pub reached_end: bool,
}
```

Tick ごとに `dispatch_tick` が生成する。`current_time` は今フレームの仮想時刻（`clock.now_ms()`）。

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
| `KlinesLoadCompleted(StreamKind, Range<u64>, Vec<Kline>)` | kline 一括ロード完了 |
| `DataLoadFailed(String)` | プリフェッチ失敗 |
| `SyncReplayBuffers` | mid-replay stream 同期（バックフィル完了後のバッファ再構成） |
| `ReloadKlineStream { old_stream: Option<StreamKind>, new_stream: StreamKind }` | mid-replay で timeframe 変更時に旧 stream を除去し新 stream を再ロードする |

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
  ├─ 3. ReplayState::start(start_ms, end_ms, step_size_ms) で
  │     - clock = Some(StepClock { status: Waiting, now_ms: start_ms, ... })
  │     - event_store = EventStore::new()
  │     - active_streams = 現在の全 StreamKind
  ├─ 4. 全 active_streams に対して Task::perform(loader::load_klines(stream, range), ...)
  │     → Message::Replay(KlinesLoadCompleted(stream, range, klines))
  └─ 5. subscription() の次評価で exchange_streams が外れ WS 切断

[KlinesLoadCompleted(stream, range, klines) 受信]
  ├─ klines が空の場合は "未ロード" として無視（early return）
  ├─ ReplayState::on_klines_loaded(stream, range, klines, wall_now)
  │     event_store.ingest_loaded(stream, range, LoadedData { klines, .. })
  │     全 active_streams が is_loaded → clock.resume_from_waiting(wall_now) で再生開始
  │     未完の stream あり → clock は Waiting のまま
  └─ pre_start_history(klines, start_ms) で start 時刻前のバー（k.time < start_ms）のみ抽出し
       dashboard.ingest_replay_klines(&stream, &history_klines, main_window) で注入
       （start 時刻以降のバーは dispatch_tick が逐次注入するため、ここでは注入しない）
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
         clock は自動的に Paused（dispatch_tick 内で処理済み）
```

`dispatch_tick` のアルゴリズムは §7.1 を参照。

### 6.3 起動時 Auto-play（Fixture 直接起動）

`saved-state.json` に `replay_config.mode = "replay"` と有効な `range_start` / `range_end` が含まれる場合、起動時に **自動 Play** が発火する。ワークアラウンドの「Live 起動 → toggle → play」という 3 ステップが不要になる。

```
[アプリ起動 (src/main.rs ReplayState 初期化)]
  ├─ replay_config.mode == "replay" && parse_replay_range() が Ok
  │     → pending_auto_play = true
  └─ それ以外 → pending_auto_play = false（通常起動）

[SessionRestoreResult(None) — Tachibana ペインあり]
  ├─ has_tachibana_stream_pane() == true && pending_auto_play
  │     → on_session_unavailable() で pending_auto_play = false
  │        log::info!("[auto-play] session unavailable — auto-play deferred")
  │        Toast::info("Replay auto-play was deferred: please log in to resume")
  └─ Tachibana ペインなし（Binance 等）→ pending_auto_play はそのまま維持

[Sidebar::TickersTable::UpdateMetadata 受信]
  └─ pending_auto_play が true
       → Dashboard::refresh_waiting_panes() で全 Waiting ペインの mark_resolution_due() を呼び出し
          log::info!("[auto-play] metadata updated — refreshed waiting panes")
          （次の ResolveStreams で即座に Ready 昇格を試みる）

[Dashboard::Event::ResolveStreams 処理 (Ok(resolved) 分岐)]
  ├─ (1) dashboard.resolve_streams() で当該ペインを Ready に昇格（同期）
  └─ (2) pending_auto_play が true かつ is_replay() かつ
         all_panes_have_ready_streams() == true
           → pending_auto_play = false
              Task::done(Message::Replay(ReplayMessage::Play)) を batch dispatch

[ReplayMessage::Play 受信]
  └─ on_manual_play_requested() で pending_auto_play = false
     （ユーザーが手動で Play を押した場合でも auto-play と二重発火しないようにする）
```

**ゲート判定の詳細**: `all_panes_have_ready_streams(window_id)` は `iter_all_panes()` を使い、全ペインの `streams` フィールドを検査する。`Waiting { streams: [] }` は stream 未設定（空ペイン）として Ready 扱いにする（[src/screen/dashboard.rs](../src/screen/dashboard.rs)）。

**タイムアウトなし**: auto-play はイベント駆動で発火する。タイマーは存在しない。Tachibana ペインが含まれる場合は session restore 失敗時に明示的に flag を落とし、ユーザーへ info トーストで通知する。

**pending_auto_play は永続化しない**: `Play` 実行後に `POST /api/app/save` で保存した state の `replay_config` に mode/range が残るため、次回起動でも同じ自動再生が発動する。これはユーザが「リプレイ状態を保存した＝次回も同じ区間を再生したい」という意図と一致する。

### 6.4 mid-replay バックフィル

```
[ペイン操作: Sidebar / Dashboard / Pane API]
  ├─ streams を mutate
  └─ active_streams に新 stream を追加 + Task::perform(load_klines(...))

[KlinesLoadCompleted(new_stream, ...) 受信]
  ├─ event_store.ingest_loaded(new_stream, ...)
  ├─ dashboard.ingest_replay_klines(&new_stream, &klines, main_window)
  └─ clock が Waiting && 全 streams ロード完了 → clock.resume_from_waiting(wall_now)
     既存 streams は再生を止めない（EventStore は既ロード済み）
```

### 6.5 StepForward / StepBackward

```
[StepForward — Playing 中]
  ├─ clock.pause()
  ├─ clock.seek(range.end)                               ← current_time を End まで一気に移動
  ├─ dashboard.reset_charts_for_seek(main_window)         ← チャートをクリア
  └─ inject_klines_up_to(range.end)                      ← End 時点までのデータを再注入
     range.end は変更しない（ユーザー設定の終了時刻を保持）
     kline 再フェッチは行わない — EventStore から再構成

[StepForward — Paused 中]
  ├─ new_time = current_time + min_timeframe_ms(active_streams)
  ├─ new_time > range.end の場合は no-op（範囲終端を超える）
  ├─ clock.seek(new_time)
  └─ inject_klines_up_to(new_time): 0..new_time+1 の klines を全 active_streams から注入

[StepBackward — Playing 中]
  ├─ clock.pause()
  ├─ clock.seek(range.start)                              ← current_time を start に戻す
  ├─ dashboard.reset_charts_for_seek(main_window)         ← チャートをクリア
  └─ inject_klines_up_to(range.start)                    ← start 時点のデータを再注入
     range.end は変更しない（ユーザー設定の終了時刻を保持）
     kline 再フェッチは行わない — EventStore から再構成

[StepBackward — Paused / Waiting 中]
  ├─ 全 active_streams の klines_in(stream, 0..current_time) で
  │   current_time より小さい最大の kline.time を探す
  ├─ new_time = compute_step_backward_target(prev_time, current_time, start_ms)
  │   （start_ms 未満には戻れないようクランプ）
  ├─ clock.seek(new_time) + clock.pause()
  ├─ dashboard.reset_charts_for_seek(main_window) でチャートをクリア
  └─ inject_klines_up_to(new_time): 0..new_time+1 の klines を全 active_streams から注入
     kline 再フェッチは行わない — EventStore から再構成
```

### 6.6 ユーザー操作による初期状態リセット

以下の操作を受けたとき、clock が Some（リプレイ進行中）ならば **初期状態（range.start）に戻して停止** する。

```
操作: CycleSpeed / StartTimeChanged / EndTimeChanged

共通フロー（clock が Some の場合）:
  ├─ clock.pause()
  ├─ clock.seek(range.start)
  ├─ dashboard.reset_charts_for_seek(main_window)
  └─ inject_klines_up_to(range.start)
     kline 再フェッチは行わない — EventStore から再構成

CycleSpeed の場合は上記に加えて速度の循環変更も行う。
StartTimeChanged / EndTimeChanged は range_input の更新も行う（clock は再作成しない）。
clock が None の場合（Play 前）は入力更新または速度変更のみ、リセットは行わない。
```

### 6.7 Live 復帰

```
[ReplayMessage::ToggleMode (Replay → Live)]
  ├─ clock = None, event_store = EventStore::new(), active_streams = {}
  ├─ range_input = default
  ├─ pending_auto_play = false
  ├─ Dashboard::rebuild_for_live() で content リビルド
  └─ subscription() 次評価で exchange_streams 復帰 → WS 自動再購読
```

---

## 7. 再生エンジン

### 7.1 dispatch_tick ([src/replay/dispatcher.rs](../src/replay/dispatcher.rs))

```rust
pub fn dispatch_tick(
    clock: &mut StepClock,
    store: &EventStore,
    active_streams: &HashSet<StreamKind>,
    wall_now: Instant,
) -> DispatchResult;
```

**アルゴリズム**:

1. 全 `active_streams` について `store.is_loaded(stream, full_range)` を確認
   - 未ロードの stream があれば `clock.set_waiting()` して空 `DispatchResult` を返す
2. `clock.status == Waiting` → 空を返す（全 streams ロード完了後は `resume_from_waiting` を待つ）
3. `clock.tick(wall_now)` で 1 ステップ進める → emit 範囲 `[prev_ms, new_ms)` を取得
   - 発火タイミング未到達（空 Range）→ 空 `DispatchResult` を返す
4. 各 `stream` in `active_streams`:
   - `store.klines_in(stream, range)` → `kline_events` に追加
   - `store.trades_in(stream, range)` → `trade_events` に追加
5. `clock.status == Paused`（終端到達時に `tick()` が自動 Pause する）→ `reached_end = true`
6. `DispatchResult { current_time: clock.now_ms(), kline_events, trade_events, reached_end }` を返す

**キャッチアップ**: wall 時間が遅れていた場合（例: スリープ後）、`tick()` は `next_step_at` から経過した分だけ複数ステップを一括処理し、全時間範囲のイベントをまとめて返す。

**状態変化の範囲**: `dispatch_tick` が書き換えるのは `StepClock` の `now_ms` / `next_step_at` / `status` のみ。`EventStore` への書き込みは行わない。

### 7.2 Pause / Resume / CycleSpeed

- `Pause`: `clock.pause()` — `status = Paused`, `next_step_at = None`
- `Resume`: `clock.play(wall_now)` — `status = Playing`, `next_step_at` 再設定
- `CycleSpeed`: `SPEEDS = [1.0, 2.0, 5.0, 10.0]` を `(i+1) % 4` で循環、`clock.set_speed(next)` を呼ぶ。
  clock が Some の場合は §6.6 の「初期状態リセット」フローを実行する（pause + seek to start）。
- `speed_label()`: `if speed == floor(speed) { "Nx" } else { "N.Nx" }` を返す

### 7.3 Waiting 状態の自動解除

`ClockStatus::Waiting` はデータロード待ち中を示す。`dispatch_tick` が毎フレーム `is_loaded` チェックを行い、全 active_streams がロード済みになっていても `Waiting` のままではフレーム抽出しない。ロード完了の通知は `KlinesLoadCompleted` ハンドラ内の `on_klines_loaded()` が担い、全 streams 完了時点で `clock.resume_from_waiting(wall_now)` を呼んで `Playing` に復帰する。

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
2. `clock` が Playing なら `Waiting` に遷移（`dispatch_tick` が自動検出）
3. `Task::perform(load_klines(new_stream, range), KlinesLoadCompleted)` を発火
4. `KlinesLoadCompleted` 受信 → `event_store.ingest_loaded` → 全 streams ロード完了で `clock.resume_from_waiting()`

既存の streams / EventStore エントリは変更されず、再生は継続される。

### 8.3 reset_for_seek / ingest_historical_klines

`KlineChart` の API:

```rust
// チャートデータを全クリアしてシークの準備をする（StepBackward, ticker 変更等）
pub fn reset_for_seek(&mut self);

// EventStore から取り出した klines をチャートに挿入する
pub fn ingest_historical_klines(&mut self, klines: &[Kline]);
```

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

#### 認証確認 (`/api/auth/*`)

| メソッド | パス | リクエストボディ | レスポンス | 対応コマンド |
|---|---|---|---|---|
| GET | `/api/auth/tachibana/status` | — | `{"session":"present"\|"none"}` | `TachibanaSessionStatus` |

#### アプリ制御 (`/api/app/*`)

| メソッド | パス | 用途 |
|---|---|---|
| POST | `/api/app/save` | アプリ状態をディスクに保存 (E2E テスト用)|
| POST | `/api/app/screenshot` | デスクトップ全体を `C:/tmp/screenshot.png` に保存（レスポンス: `{"ok":true}` or `{"ok":false,"error":"..."}`）|

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

#### 通知 (`/api/notification/*`)

| メソッド | パス | リクエストボディ | 用途 |
|---|---|---|---|
| GET | `/api/notification/list` | — | 現在の Toast 通知一覧を返す（E2E テスト検証用）|

### 11.3 レスポンススキーマ

#### `ReplayStatus` ([src/replay/mod.rs](../src/replay/mod.rs))

```rust
pub struct ReplayStatus {
    pub mode: String,                    // "Live" | "Replay"

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,          // "Playing" | "Paused" | "Loading"

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

`status` は `ClockStatus` を文字列化したもの。`Waiting` は `"Loading"` として返す（`to_status()` で変換）。

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
| `BASE_STEP_DELAY_MS` | `1_000` | [src/replay/clock.rs](../src/replay/clock.rs) | 1x speed での wall delay (ms/bar) |
| `PRE_START_HISTORY_BARS` | `300` | [src/replay/mod.rs](../src/replay/mod.rs) | リプレイ開始前のコンテキスト用 kline 本数 |
| API port (default) | `9876` | [src/replay_api.rs](../src/replay_api.rs) | `FLOWSURFACE_API_PORT` で上書き |
| HTTP buffer size | `8192` | [src/replay_api.rs](../src/replay_api.rs) | リクエスト読み取りバッファ |
| mpsc channel bound | `32` | [src/replay_api.rs](../src/replay_api.rs) | API → iced キュー |

### 12.2 時間範囲

- **Kline フェッチ範囲**: `(start_ms - PRE_START_HISTORY_BARS * step_size_ms, end_ms)` = `(start_ms - 300 * tf_ms, end_ms)` — リプレイ開始前のコンテキストも 300 本含める
- **EventStore クエリ範囲**: 全て **半開区間** `[start, end)` — `time >= start && time < end`
- **StepClock の emit 範囲**: `[prev_now_ms, new_now_ms)` — 1 ステップで `step_size_ms` 進む

### 12.3 設計上の不変条件

| # | 不変条件 | 破壊したときの症状 |
|:-:|---|---|
| 1 | `dispatch_tick` は `EventStore` に書き込まない。読み取り専用 | 同一スライスが二重 dispatch される、またはデータが消える |
| 2 | `klines_in` / `trades_in` は半開区間 `[start, end)` で返す | 境界 kline が重複挿入されるか、スキップされる |
| 3 | `KlinesLoadCompleted` 受信時は必ず `event_store.ingest_loaded` の後に `ingest_replay_klines` を呼ぶ | チャートに kline が届かない（EventStore にはあるが UI に反映されない）|
| 4 | `reset_for_seek()` は StepBackward と ticker/timeframe 変更時に必ず呼ぶ | 古い kline が残ったまま新データが重なる |
| 5 | `pending_auto_play` は永続化しない。`toggle_mode()` の Replay→Live 経路でも必ずリセットする | Live に切り替えた後でも auto-play が残留し、次回 ResolveStreams で誤発火する |
| 6 | `resolve_streams()` で dashboard の借用を解放した後に `all_panes_have_ready_streams()` を呼ぶ | Rust borrow checker エラー（&mut self と &self の競合）|

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
| インジケータ再計算 | まずは Kline + Trades に集中 |
| リプレイデータのローカルキャッシュ | 毎回 API 取得 |
| 日時ピッカー UI | 現状はテキスト入力 |
| Tachibana の M1 / 時間足 | API 非対応 |
| Trades の EventStore 統合（R4） | 現在 Trades は旧 fetch_batched 経路のまま（将来対応）|
| auto-play の無効化設定 | 「Replay 構成で保存＝次回も再生する」が現仕様。設定化は将来課題 |

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
| [src/replay/mod.rs](../src/replay/mod.rs) | `ReplayState` / `ReplayMessage` / `ReplayMode` / `ReplayStatus` / `parse_replay_range` / `format_current_time` / `SPEEDS` / `pending_auto_play` 管理 |
| [src/replay/clock.rs](../src/replay/clock.rs) | `StepClock` / `ClockStatus` / `play` / `pause` / `seek` / `tick` / `set_waiting` / `resume_from_waiting` / `BASE_STEP_DELAY_MS` |
| [src/replay/store.rs](../src/replay/store.rs) | `EventStore` / `LoadedData` / `ingest_loaded` / `is_loaded` / `klines_in` / `trades_in` |
| [src/replay/controller.rs](../src/replay/controller.rs) | `ReplayController` / `TickOutcome` / `handle_message` / `tick` — `ReplayMessage` ハンドラと Tick ループを main.rs から分離 |
| [src/replay/dispatcher.rs](../src/replay/dispatcher.rs) | `dispatch_tick` / `DispatchResult` |
| [src/replay/loader.rs](../src/replay/loader.rs) | `load_klines` / `KlineLoadResult` / `fetch_all_klines` |
| [src/replay_api.rs](../src/replay_api.rs) | HTTP サーバー (`tokio::net::TcpListener` + 手動パース) / `ApiCommand` / `PaneCommand` / `ReplySender` / ルーティング |
| [src/main.rs](../src/main.rs) | `Flowsurface` への `replay` フィールド、起動時 `pending_auto_play` 初期化、`Message::Replay` / `Message::ReplayApi` ハンドラ、auto-play ゲート（`ResolveStreams` + `Tick`）、`view_replay_header()`、`subscription()`、`handle_pane_api()` |
| [src/screen/dashboard.rs](../src/screen/dashboard.rs) | `prepare_replay()` / `rebuild_for_live()` / `collect_trade_streams()` / `ingest_replay_klines()` / `ingest_trades()` / `all_panes_have_ready_streams()` |
| [src/screen/dashboard/pane.rs](../src/screen/dashboard/pane.rs) | `rebuild_content_for_replay()` / `rebuild_content_for_live()` / `ingest_replay_klines()` / `reset_for_seek()` / `insert_hist_klines()` / Heatmap の Depth unavailable オーバーレイ |
| [src/chart/kline.rs](../src/chart/kline.rs) | `ingest_historical_klines()` / `reset_for_seek()` |
| [src/connector/fetcher.rs](../src/connector/fetcher.rs) | Tachibana D1 range フィルタ分岐 |

### 14.2 テスト

- `src/replay/mod.rs`: `parse_replay_range` / `to_status` / `pending_auto_play` / `toggle_mode` リセット等
- `src/replay/store.rs`: `ingest_loaded` / `is_loaded` / `klines_in` / `trades_in` 動作検証
- `src/replay/clock.rs`: `ClockStatus` 遷移 / `tick` ステップ計算 / `seek` スナップ
- `src/replay/dispatcher.rs`: `dispatch_tick` のスライス抽出 / Waiting 検出 / 終端判定
- `src/replay/loader.rs`: `EventStore` 直接操作で loader 動作を検証
- `src/chart/kline.rs`: `ingest_historical_klines` / `reset_for_seek`
- `src/screen/dashboard.rs`: `all_panes_have_ready_streams` の true/false 条件
- `src/replay_api.rs`: ルーター/パーサーテスト
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
| **R3: アーキテクチャ刷新** | `PlaybackState` / `FireStatus` / `process_tick` / `ReplayKlineBuffer` / `TradeBuffer` を全廃し、`StepClock` + `EventStore` + `dispatch_tick` に置き換え | [docs/plan/replay_redesign.md](plan/replay_redesign.md) |
| **Fixture 直接起動** | `pending_auto_play` / `all_panes_have_ready_streams` を追加し、`saved-state.json` の replay 構成で自動 Play | [docs/plan/replay_fixture_direct_boot.md](plan/replay_fixture_direct_boot.md) |

### 15.2 R3 刷新の設計判断

#### 15.2.1 旧アーキテクチャの問題点

Phase 8 までの実装（`process_tick` + `COARSE_CUTOFF_MS` 境界 + `FireStatus` + buffer ベース）は動作したが、以下の構造的問題を抱えていた:

1. **`FireStatus` の 3 状態管理**: `None(buffer末尾)` と `None(バックフィル中)` を区別するために enum が必要で、全 chart を走査するたびに `min` 計算が複雑化した
2. **`TradeBuffer` の cursor ベース管理**: `advance_cursor_to` / `drain_until` の cursor 不変条件が壊れると trades が重複 or 欠損し、デバッグが困難だった
3. **`SyncReplayBuffers` の 2 系統 chain**: streams 変更を確実に追従させるため、`Message::Dashboard` 末尾と `Message::Sidebar::TickerSelected` の両方に chain が必要で、追加を忘れると silent バグになった
4. **`is_replay_mode()` ガード**: 遅延完了したライブフェッチが `replay_kline_buffer` を上書きするのを防ぐガードが必要で、デッドロックのトリップワイヤーだった

#### 15.2.2 R3 の設計方針

- **EventStore**: データを「ロード済み範囲」として蓄積し、クエリは純粋な範囲検索。cursor 管理ゼロ
- **StepClock**: 離散バーステップモデル。`tick(wall_now)` を呼ぶと発火タイミングかどうかを判定し、発火した場合のみ emit 範囲を返す。連続時刻補間なし
- **dispatch_tick**: ステートレスなロジック。Tick ごとに「今の step 範囲」のスライスを EventStore から取得するだけ
- **Waiting 状態**: データ未ロード時は `dispatch_tick` がクロックを自動 Waiting に遷移し、ロード完了で `resume_from_waiting` により自動再開

#### 15.2.3 廃止されたコンポーネント

| 廃止コンポーネント | 代替 |
|---|---|
| `PlaybackState` | `StepClock` + `EventStore` + `active_streams` |
| `FireStatus` enum | `dispatch_tick` の戻り値 `reached_end: bool` |
| `process_tick` | `dispatch_tick` (ステートレス) |
| `TradeBuffer` | `EventStore::trades_in` (half-open range query) |
| `ReplayKlineBuffer` | `KlineChart::ingest_historical_klines` |
| `enable_replay_mode` / `disable_replay_mode` / `is_replay_mode` | 廃止（モード管理不要）|
| `replay_advance` | `ingest_historical_klines` で EventStore のスライスを直接渡す |
| `rebuild_for_step_backward` | `reset_for_seek` + `ingest_historical_klines` に統一 |
| `VirtualClock` | `StepClock`（離散ステップに特化）|

### 15.3 Fixture 直接起動の設計判断

以前の E2E テストは「Live fixture で起動 → 15s 待機 → `POST /api/replay/toggle` → `POST /api/replay/play`」という 4 ステップを強制されていた。これを `saved-state.json` に replay 構成を含めた fixture を置くだけで自動再生できるように改修した。

**方針選定**: `ReplayState` に `pending_auto_play` フラグを transient フィールドとして追加し、全ペインが `Ready` になった瞬間に `ReplayMessage::Play` を dispatch する。既存の `prepare_replay()` / `start()` / kline load パスを一切変更しないため、UI でのPlay操作と完全等価な経路を通る。詳細は [docs/plan/replay_fixture_direct_boot.md](plan/replay_fixture_direct_boot.md) を参照。
