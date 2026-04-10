# リプレイヘッダー機能 — 実装プラン

**作成日**: 2026-04-10
**対象**: メインウィンドウにヘッダーバーを追加し、ライブ/リプレイモードの切替と再生制御を実現する

---

## 1. 概要

メインウィンドウ上部にヘッダーバーを新設し、以下の要素を配置する:

```
┌───────────────────────────────────────────────────────────────────────────┐
│ 🕐 2026-04-10 14:32:05  │ [LIVE / REPLAY] │ 2026-04-01 09:00 ~ 2026-04-01 15:00 │ ▶⏸ ⏭ │
│     現在時刻              トグルボタン        開始日時 ～ 終了日時                    再生制御 │
└───────────────────────────────────────────────────────────────────────────┘
※ Phase 4 で ⏮（巻き戻し）と速度表示（1x）を追加
```

| 要素 | ライブモード | リプレイモード |
|------|:----------:|:----------:|
| 現在時刻 | リアルタイム表示 | 再生中の仮想時刻を表示 |
| トグルボタン | **LIVE** がアクティブ | **REPLAY** がアクティブ |
| 開始日時 ～ 終了日時 | 無効（グレーアウト） | 有効（入力可能） |
| 再生制御ボタン | 無効（グレーアウト） | 有効 |

---

## 2. 現状のアーキテクチャ（関連箇所）

### 2.1 メインビュー構造 (`src/main.rs:703-711`)

```rust
let base = column![
    header_title,                           // macOS のみ "FLOWSURFACE" テキスト
    match sidebar_pos {                     // サイドバー + ダッシュボード
        Left  => row![sidebar_view, dashboard_view],
        Right => row![dashboard_view, sidebar_view],
    }.spacing(4).padding(8),
];
```

→ `header_title` と `row![sidebar, dashboard]` の間に新しいヘッダーバーを挿入する。

### 2.2 リアルタイムデータフロー

```
WebSocket → exchange::Event → main.rs::update()
  ├── Event::TradesReceived  → dashboard.ingest_trades()
  ├── Event::KlineReceived   → dashboard.update_latest_klines()
  └── Event::DepthReceived   → dashboard.ingest_depth()
```

### 2.3 フレームティック

```
iced::window::frames() → Message::Tick(Instant) → dashboard.tick()
```

毎フレーム呼ばれるため、リプレイ時の仮想時刻の進行やデータ注入のフックとして利用可能。

### 2.4 既存の過去データ取得 API

| API | シグネチャ | 対応取引所 |
|-----|----------|----------|
| `fetch_klines()` | `(TickerInfo, Timeframe, Option<(u64,u64)>) → Vec<Kline>` | Binance, Bybit, Hyperliquid, OKX, MEXC |
| `fetch_trades_batched()` | `(TickerInfo, from_time, to_time, data_path) → impl Straw<(), Vec<Trade>, AdapterError>` | Binance, Bybit |
| `fetch_open_interest()` | `(TickerInfo, Timeframe, Option<(u64,u64)>) → Vec<OpenInterest>` | Binance, Bybit, OKX |

→ これらの API でリプレイ用の過去データを取得可能。

> **制約**: `fetch_klines()` は Binance で 1 リクエストあたり最大 1000 本。1 分足 × 6 時間 = 360 本なので範囲上限 6 時間なら 1 リクエストで収まる。
> `fetch_trades_batched()` は `Straw` ストリームを返し、`Vec<Trade>` をバッチごとに yield する（一括ではない）。

### 2.5 時刻体系

- アプリ全体で **Unix ミリ秒 (`u64`)** を使用
- `Trade.time`, `Kline.time` はすべて Unix ms
- 表示変換: `data::UserTimezone` でタイムゾーン変換（UIレイヤーのみ）

---

## 3. 設計

### 3.1 状態管理: `ReplayState`

```rust
/// リプレイモードの状態を管理する。
/// Flowsurface 構造体に `replay: ReplayState` として追加。
pub struct ReplayState {
    /// ライブ / リプレイの切替
    pub mode: ReplayMode,
    /// リプレイ範囲の設定（UI入力）
    pub range_input: ReplayRangeInput,
    /// リプレイ実行中の状態（再生開始後に Some になる）
    pub playback: Option<PlaybackState>,
}

pub enum ReplayMode {
    Live,
    Replay,
}

pub struct ReplayRangeInput {
    pub start: String,   // "2026-04-01 09:00" 形式のテキスト入力
    pub end: String,     // "2026-04-01 15:00" 形式のテキスト入力
}

pub struct PlaybackState {
    /// リプレイ範囲（パース済み、Unix ms）
    pub start_time: u64,
    pub end_time: u64,
    /// 現在の仮想時刻（Unix ms）
    pub current_time: u64,
    /// 再生状態
    pub status: PlaybackStatus,
    /// 再生速度倍率（1x, 2x, 5x, 10x, ...）
    pub speed: f64,
    /// プリフェッチ済み Trades バッファ（ストリームごと）
    /// ※ Kline は再生開始時に insert_hist_klines() で一括挿入するためバッファ不要
    /// NOTE: StreamKind が Hash+Eq を derive していない場合は Vec<(StreamKind, TradeBuffer)> に変更
    pub trade_buffers: HashMap<StreamKind, TradeBuffer>,
}

pub enum PlaybackStatus {
    Playing,
    Paused,
}

pub struct TradeBuffer {
    pub trades: Vec<Trade>,
    /// 次に注入するインデックス
    pub cursor: usize,
}
```

### 3.2 メッセージ

```rust
// Message enum に追加
enum Message {
    // ... 既存 ...
    Replay(ReplayMessage),
}

enum ReplayMessage {
    /// ライブ/リプレイ切替
    ToggleMode,
    /// 開始日時の入力変更
    StartTimeChanged(String),
    /// 終了日時の入力変更
    EndTimeChanged(String),
    /// 再生ボタン押下
    Play,
    /// 停止ボタン押下
    Pause,
    /// 進むボタン（1分早送り）
    StepForward,
    /// Trades バッチ受信（Straw ストリームから逐次到着）
    TradesBatchReceived(StreamKind, Vec<Trade>),
    /// 全データのプリフェッチ完了
    DataLoaded,
    /// データプリフェッチ失敗
    DataLoadFailed(String),
}

// Phase 4 で追加:
//   SpeedChanged(f64)    — 再生速度変更
//   StepBackward          — チャートリセット＋再注入が必要
```

### 3.3 ヘッダーバー UI (`src/main.rs` の view)

挿入位置: `header_title` の直後、`row![sidebar, dashboard]` の直前。

```
column![
    header_title,
    replay_header_bar,    // ← NEW
    row![sidebar_view, dashboard_view].spacing(4).padding(8),
]
```

ヘッダーバーのレイアウト:

```rust
fn view_replay_header(&self) -> Element<'_, Message> {
    let time_display = text(format_current_time(&self.replay, self.timezone))
        .font(style::AZERET_MONO)
        .size(12);

    let mode_toggle = button(match self.replay.mode {
            ReplayMode::Live   => text("LIVE"),
            ReplayMode::Replay => text("REPLAY"),
        })
        .on_press(Message::Replay(ReplayMessage::ToggleMode));

    let is_replay = matches!(self.replay.mode, ReplayMode::Replay);

    // on_input() を呼ばなければ read-only になる
    let mut start_input = text_input("Start", &self.replay.range_input.start);
    let mut end_input = text_input("End", &self.replay.range_input.end);
    if is_replay {
        start_input = start_input.on_input(|s| Message::Replay(ReplayMessage::StartTimeChanged(s)));
        end_input = end_input.on_input(|s| Message::Replay(ReplayMessage::EndTimeChanged(s)));
    }

    // Phase 1: ▶/⏸ と ⏭ のみ。⏮ は Phase 4 で追加
    let controls = row![
        play_pause_btn,      // ▶ / ⏸
        step_forward_btn,    // ⏭
    ].spacing(4);

    row![time_display, mode_toggle, start_input, text("~"), end_input, controls]
        .spacing(12)
        .padding(padding::all(4))
        .align_y(Alignment::Center)
        .into()
}
```

### 3.4 リプレイデータフロー

```
[ユーザー操作]
    │
    ▼
ToggleMode → Replay に切替
    │
    ▼
Play 押下
    │
    ├── 1. range_input をパース → start_time / end_time (Unix ms)
    │      ※パース失敗時は input の border を赤くして中断
    │      ※範囲が 6 時間を超える場合はエラー表示して中断
    ├── 2. アクティブな全ペインの StreamKind を列挙
    ├── 3. ペインの content を再構築してチャートデータをクリア
    │      （settings / streams は保持、content のみ KlineChart::new() 等で再生成）
    ├── 4. status を Loading に設定（ヘッダーに "Loading..." 表示）
    ├── 5. 既存の fetcher::request_fetch() を利用して過去データを取得:
    │       ├── FetchRange::Kline(start, end) → Kline は一括 insert_hist_klines()
    │       └── FetchRange::Trades(start, end) → fetch_trades_batched() の Straw ストリームを購読
    │           各バッチ → TradesBatchReceived → TradeBuffer に追記
    │           全バッチ完了 → DataLoaded → PlaybackState 初期化
    │      ※fetch_trades_batched() は Binance / Bybit のみ対応。
    │       他の取引所では Kline のみの再生になる。
    ├── 6. WebSocket 購読は subscription() の除外で自動停止
    │      （mode が Replay になった時点で次の subscription 評価で exchange_streams を返さなくなる）
    └── 7. リプレイ中はペインの追加/削除/ティッカー変更を無効化

[DataLoaded 受信後]
    │
    ▼
PlaybackState を初期化 (current_time = start_time, status = Playing)

[毎フレーム: Message::Tick]
    │
    ▼
リプレイモード & Playing の場合:
    ├── current_time += elapsed_ms * speed
    ├── current_time <= t の Trades を trade_buffers から取り出す
    ├── dashboard.ingest_trades(stream, buffer, update_t, main_window) に注入
    │   ※ update_t = バッチ内最終トレードの time
    │   （Kline は再生開始時に一括挿入済みのため、フレーム毎の注入は Trades のみ）
    └── current_time >= end_time なら Paused に遷移

[ライブに戻す]
    │
    ▼
ToggleMode → Live に切替
    ├── PlaybackState を破棄
    ├── ペインの content を再構築（リプレイデータをクリア）
    ├── ペイン操作の無効化を解除
    └── WebSocket 購読を再開（subscription() が自動で再購読）
```

### 3.5 WebSocket 制御

リプレイモード中はリアルタイムデータの注入を無効化する必要がある。

**方式**: `subscription()` でリプレイ中は `exchange_streams` を購読リストから除外する。

```rust
fn subscription(&self) -> Subscription<Message> {
    let window_events = window::events().map(Message::WindowEvent);
    let sidebar = self.sidebar.subscription().map(Message::Sidebar);

    if self.login_window.is_some() {
        return Subscription::batch(vec![window_events, sidebar]);
    }

    let tick = iced::window::frames().map(Message::Tick);
    let hotkeys = keyboard::listen().filter_map(/* ... */);

    // リプレイモード中は WebSocket ストリームを購読しない
    if matches!(self.replay.mode, ReplayMode::Replay) {
        return Subscription::batch(vec![window_events, sidebar, tick, hotkeys]);
    }

    let exchange_streams = self.active_dashboard()
        .market_subscriptions()
        .map(Message::MarketWsEvent);

    Subscription::batch(vec![exchange_streams, sidebar, window_events, tick, hotkeys])
}
```

→ Iced の `Subscription` は宣言的に動作するため、`exchange_streams` を返さなくなった時点で WebSocket 接続は自動的にドロップされる。ライブに戻した際も自動再接続される。

---

## 4. 実装ステップ

### Phase 1: ヘッダーバー UI（見た目のみ）

リプレイ機能のロジックは含めず、UIの枠組みだけを構築する。

| Step | 内容 | ファイル |
|------|------|---------|
| 1-1 | `ReplayState`, `ReplayMode`, `ReplayRangeInput` 型定義 | `src/replay.rs` (新規) |
| 1-2 | `Flowsurface` に `replay: ReplayState` フィールド追加 | `src/main.rs` |
| 1-3 | `Message::Replay(ReplayMessage)` バリアント追加（Phase 1 で使うバリアントのみ） | `src/main.rs` |
| 1-4 | `view_replay_header()` メソッド実装（現在時刻 + トグル + 日時入力 + ▶/⏸ + ⏭） | `src/main.rs` |
| 1-5 | `view()` の `column!` にヘッダーバーを挿入 | `src/main.rs` |
| 1-6 | トグルボタンでモード切替（UI状態のみ。入力欄・ボタンの有効/無効切替） | `src/main.rs` |

> ⏮（StepBackward）ボタンは Phase 1 では配置しない。Phase 4 で実装と同時に追加する。

**検証**: ビルドしてメインウィンドウ上部にヘッダーバーが表示されること。トグルで入力欄・ボタンの有効/無効が切り替わること。

### Phase 2: リプレイデータのプリフェッチ

| Step | 内容 | ファイル |
|------|------|---------|
| 2-1 | `PlaybackState`, `PlaybackStatus`, `TradeBuffer` 型定義 | `src/replay.rs` |
| 2-2 | 再生ボタン押下時に日時文字列をパース → `(u64, u64)`。`chrono::NaiveDateTime::parse_from_str` で変換。パース失敗時は input border を赤くする。範囲 > 6 時間ならエラー | `src/replay.rs` |
| 2-3 | ペインの content を再構築してチャートデータをクリア（settings/streams は保持） | `src/main.rs` |
| 2-4 | Kline 取得: 既存の `fetcher::request_fetch()` → `insert_hist_klines()` で一括挿入 | `src/main.rs` |
| 2-5 | Trades 取得: `fetch_trades_batched()` の Straw ストリームを購読 → `TradesBatchReceived` で TradeBuffer に逐次追記 → 全バッチ完了で `DataLoaded` | `src/main.rs`, `src/connector/fetcher.rs` |
| 2-6 | ローディング状態の表示（ヘッダーに "Loading replay data..." + 進捗） | `src/main.rs` |
| 2-7 | リプレイ中はペインの追加/削除/ティッカー変更を無効化 | `src/main.rs`, `src/screen/dashboard.rs` |

**検証**: 再生ボタン押下でAPIコールが発行され、Kline がチャートに一括挿入、Trades がバッファに格納されること。全データ到着後に PlaybackState が初期化されること。

### Phase 3: リプレイ再生エンジン

| Step | 内容 | ファイル |
|------|------|---------|
| 3-1 | `Message::Tick` ハンドラでリプレイモード分岐を追加 | `src/main.rs` |
| 3-2 | フレームごとに `current_time` を進め、該当範囲の Trades を `ingest_trades(stream, buffer, update_t, main_window)` に注入（Kline は Phase 2 で一括挿入済み） | `src/main.rs`, `src/replay.rs` |
| 3-3 | `subscription()` でリプレイ中は `exchange_streams` を除外 | `src/main.rs` |
| 3-4 | 一時停止/再開の制御 | `src/main.rs` |
| 3-5 | StepForward で `current_time` を1分先にジャンプ（該当区間の Trades を一括注入） | `src/main.rs`, `src/replay.rs` |
| 3-6 | `current_time >= end_time` で自動停止 | `src/replay.rs` |
| 3-7 | Heatmap / Ladder ペインに「Replay: Depth unavailable」オーバーレイを表示 | `src/screen/dashboard/pane.rs` |
| 3-8 | TimeAndSales ペインはリプレイ Trades を `ingest_trades()` 経由で自動的に受信（追加実装不要だが動作確認） | — |
| 3-9 | Comparison ペインはリプレイ対象外とし、スコープ外に明記 | — |

**検証**: 再生ボタンでチャートが過去データに基づいて動的に更新されること。一時停止・再開・早送りが機能すること。TimeAndSales にリプレイ中の約定が表示されること。

> **NOTE**: StepBackward（巻き戻し）はチャートリセット＋ `start_time` からの再注入が必要なため Phase 4 に延期。

### Phase 4: ライブ復帰・巻き戻し・クリーンアップ

| Step | 内容 | ファイル |
|------|------|---------|
| 4-1 | LIVE に戻す時: PlaybackState を破棄、ペインの content を再構築、ペイン操作の無効化を解除 | `src/main.rs` |
| 4-2 | 再生速度の変更 UI: 速度テキスト（「1x」）をクリックで 1x → 2x → 5x → 10x サイクル切替 + `SpeedChanged` メッセージ追加 | `src/main.rs`, `src/replay.rs` |
| 4-3 | ⏮ ボタンを UI に追加 + StepBackward の実装: チャートリセット → Kline 再挿入 → `start_time` から `new_current_time` まで Trades を一括再注入 | `src/main.rs`, `src/replay.rs` |

**検証**:
- リプレイ→ライブ切替でリアルタイムデータが再び表示されること（WebSocket は subscription() の自動再購読で復帰）
- 巻き戻しでチャートが正しく再描画されること
- 速度切替で再生速度が変わること

---

## 5. 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `src/replay.rs` | **新規**: ReplayState, PlaybackState, TradeBuffer, ReplayMessage, 再生エンジン |
| `src/main.rs` | Flowsurface に replay フィールド追加、Message::Replay 追加、view_replay_header(), update() でのリプレイ制御, subscription() の分岐 |
| `src/screen/dashboard.rs` | リプレイ用のペイン content 再構築関数、リプレイ中のペイン操作無効化 |
| `src/screen/dashboard/pane.rs` | Heatmap の「Depth unavailable」オーバーレイ表示 |
| `src/connector/fetcher.rs` | (軽微) リプレイ用のフェッチリクエスト分岐（既存 `request_fetch()` を再利用） |

---

## 6. 設計判断とトレードオフ

### 6.1 データ注入方式: Kline 一括挿入 + Trades フレーム注入

**採用**: Kline は `insert_hist_klines()` で再生開始時に一括挿入、Trades は `ingest_trades()` でフレーム毎に注入

- メリット: Kline の再生ロジックが不要（バッファ・カーソル管理なし）。チャート描画コードの変更が不要
- デメリット: リプレイ開始時にペインの content を再構築（`KlineChart::new()` 等）してクリアする必要がある
- チャートリセット方式: ペインの `settings` / `streams` は保持したまま `content` のみ再生成する

### 6.2 WebSocket 停止方式: subscription 除外 vs フラグ制御

**採用**: `subscription()` から `exchange_streams` を除外

- Iced の宣言的購読モデルに沿っている
- WebSocket 接続は自動的にドロップ/再接続される
- `update()` 側のデータ受信ハンドラに分岐を入れる必要がない

### 6.3 データプリフェッチ: 既存 fetcher 再利用

**採用**: 再生開始前に全範囲をプリフェッチ。既存の `fetcher::request_fetch()` を再利用する

- メリット: 再生中のネットワーク遅延がない。フェッチロジックのコード重複を避けられる
- デメリット: 長時間範囲では大量メモリを消費する可能性
- 緩和策: **範囲上限 6 時間**を設定。6 時間なら 1 分足 360 本で fetch_klines の 1000 本制限内に収まる
- `fetch_trades_batched()` は Straw ストリームなので、全バッチ到着を待ってから再生を開始する

### 6.4 Depth データの扱い

**スコープ外とする**。

- 過去の板情報（Depth）は取引所 API で取得できない
- リプレイ中の Heatmap ペインは Trades のフットプリントのみ表示（ヒートマップ部分は空）
- **Phase 3 で Heatmap / Ladder ペインに「Replay: Depth unavailable」オーバーレイを表示**し、ユーザーの混乱を防ぐ

### 6.5 リプレイ中のペイン操作

**リプレイ中はペインの追加/削除/ティッカー変更を無効化する**。

- 新しいペインのデータは未取得であり、追加フェッチのハンドリングが複雑になる
- UI 側でペイン操作ボタンをグレーアウトすることで実装コストを最小化

---

## 7. スコープ外（後続タスク）

| 項目 | 理由 |
|------|------|
| Depth（板情報）のリプレイ | 過去の板スナップショットは取引所 API で取得不可 |
| リプレイ範囲の永続化 | UI 状態のみ。設定保存は UX 検証後 |
| リプレイ中のインジケータ再計算 | まずは Kline + Trades の再生に集中 |
| Tachibana（立花証券）のリプレイ対応 | `fetch_klines` / `fetch_trades` が未実装 |
| リプレイデータのローカルキャッシュ | 毎回 API 取得。頻繁に使うなら後でキャッシュ検討 |
| 日時ピッカーウィジェット | Phase 1 はテキスト入力。カレンダーUI は別タスク |
| リプレイ中のペイン追加/削除 | 未取得データのハンドリングが複雑。リプレイ中は操作無効化で対応 |
| Comparison ペインのリプレイ | 複数銘柄の同期フェッチが必要。まずは単一銘柄ペインに集中 |

---

## 8. リスクと未確定事項

1. **チャートのリセット/再初期化**
   - リプレイ開始時・ライブ復帰時にペインの content を再構築する
   - 方式: `settings` / `streams` を保持したまま `content` のみ `KlineChart::new()` 等で再生成
   - ライブ復帰時は WebSocket 再購読による自動バックフィルが必要

2. **大量データのメモリ消費**
   - → **範囲上限 6 時間で緩和**。6 時間分の Trades は活発な銘柄でも数十 MB 程度に収まる

3. **Trades フェッチの取引所制限**
   - `fetch_trades_batched()` は現在 Binance / Bybit のみ対応
   - 他の取引所ではリプレイ時に Kline のみの再生になる

4. **フレームレートと再生精度**
   - `Tick` は `iced::window::frames()` で発火（≒60fps）
   - 1フレーム ≒ 16ms 間隔。1x 速度なら十分な精度
   - 高速再生（10x 以上）ではデータの粒度が粗くなる可能性

5. **~~ペインの動的変更~~** → **解決済み**
   - リプレイ中はペインの追加/削除/ティッカー変更を無効化する（設計判断 6.5）

6. **StepBackward（巻き戻し）の複雑性**
   - `current_time` を戻すだけではチャートに既に注入済みのデータは消えない
   - チャートリセット → Kline 再挿入 → `start_time` から `new_current_time` まで Trades 一括再注入が必要
   - Phase 4 に延期して複雑度を管理する

7. **StreamKind の Hash 実装**
   - `trade_buffers: HashMap<StreamKind, TradeBuffer>` を使うには `StreamKind` が `Hash + Eq` を実装している必要がある
   - 未実装なら `Vec<(StreamKind, TradeBuffer)>` で代替する