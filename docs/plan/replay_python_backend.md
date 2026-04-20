# リプレイ Python バックエンド分離 実装計画

**作成日**: 2026-04-20  
**ブランチ**: sasa/develop  
**担当**: botterYosuke  
**関連**: `docs/plan/archive/⏳kline_cache.md`（本計画で Python 側に吸収・Rust 実装不要）

---

## 目的

リプレイ用 kline 取得・キャッシュ層を Python FastAPI バックエンドに完全移管する。  
`⏳kline_cache.md` で設計した CSV.gz キャッシュ戦略は Python 側でそのまま実装し、  
Rust 側は `src/backend/` モジュールで子プロセスを管理し、HTTP クライアント経由でデータを受け取る薄い層にする。

---

## 責任分担（完成形）

```
Python Backend (port 8765)               Rust iced App
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
データ取得・キャッシュ・配信               UI・状態管理・描画・仮想約定

fetch from exchange APIs                  src/backend/
 ├─ BinanceLinear / BinanceSpot            ├─ process.rs  (子プロセス管理)
 ├─ BybitLinear / BybitSpot                └─ client.rs   (HTTP クライアント trait)
 └─ OkxLinear
                                          replay/
CSV.gz キャッシュ読み書き                   ├─ loader.rs   (Python client 呼び出しのみ)
 └─ %APPDATA%/flowsurface/                 ├─ store.rs    (EventStore: 変更なし)
    market_data/kline_cache/               ├─ clock.rs    (変更なし)
                                           ├─ dispatcher.rs (変更なし)
Tachibana は対象外                         ├─ controller/ (変更なし)
                                           └─ virtual_exchange/ (変更なし)

                                          connector/
                                           ├─ fetcher.rs  (live chart fetch: 変更なし)
                                           └─ auth.rs     (変更なし)
```

---

## Rust ファイル変更一覧

### 新規作成

```
src/backend/
├── mod.rs          pub use process::PythonBackend; pub use client::KlineDataSource;
├── process.rs      Python 子プロセスのライフサイクル管理
├── client.rs       KlineDataSource trait + PythonBackendClient 実装
└── error.rs        BackendError 型
```

### 変更

| ファイル | 変更内容 |
|---|---|
| `src/main.rs` | `Flowsurface::new()` で `PythonBackend` 起動 Task を返す |
| `src/app.rs` or `src/lib.rs` | `BackendState` を Flowsurface 構造体に追加 |
| `src/screen/dashboard.rs` | backend 未準備時に replay UI を disabled 表示 |
| `src/replay/loader.rs` | `KlineDataSource::fetch_klines()` を呼ぶだけに縮小 |
| `src/message.rs` | `BackendMessage` バリアント追加 |

### 削除

| ファイル / 関数 | 削除理由 |
|---|---|
| `replay/loader.rs` の `fetch_all_klines()` 内 非 Tachibana パス | Python 移管 |
| `replay/loader.rs` の `adapter::fetch_klines()` 呼び出し | Python 移管 |
| （`replay/cache.rs` は作成不要） | Python 側に実装済み |

> `connector/fetcher.rs` の `kline_fetch_task()` / `request_fetch()` は **live チャート用** のため変更しない。  
> Tachibana `fetch_tachibana_daily_klines()` も Rust 側に残す。

---

## `src/backend/` 詳細設計

### `error.rs`

```rust
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Python process failed to start: {0}")]
    SpawnFailed(String),
    #[error("Health check timed out after {secs}s")]
    HealthTimeout { secs: u64 },
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Unexpected response: {0}")]
    BadResponse(String),
}
```

### `process.rs`

```rust
pub struct PythonBackend {
    process: tokio::process::Child,
    port: u16,
}

impl PythonBackend {
    /// `python_backend/` ディレクトリで `uv run uvicorn main:app --host 127.0.0.1 --port 8765` を起動。
    /// /api/health が 200 を返すまで最大 15 秒ポーリングする。
    pub async fn spawn() -> Result<Self, BackendError>;

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for PythonBackend {
    fn drop(&mut self) {
        // SIGTERM → 500ms 待ち → SIGKILL
        let _ = self.process.start_kill();
    }
}
```

**ポーリング仕様:**
- 間隔: 500ms
- タイムアウト: 15 秒
- stdout / stderr は `log::debug!` に転送
- 失敗時: `BackendError::HealthTimeout`

### `client.rs`

```rust
/// リプレイ用 kline データの取得インターフェース。
/// テストでは MockKlineDataSource に差し替えられる。
#[async_trait::async_trait]
pub trait KlineDataSource: Send + Sync + 'static {
    async fn fetch_klines(
        &self,
        ticker_info: &TickerInfo,
        timeframe: Timeframe,
        range: Range<u64>,
    ) -> Result<Vec<Kline>, BackendError>;
}

/// 本番用: Python バックエンドへ HTTP GET
pub struct PythonBackendClient {
    base_url: String,
    http: reqwest::Client,
}

impl PythonBackendClient {
    pub fn new(base_url: impl Into<String>) -> Self;
}

#[async_trait::async_trait]
impl KlineDataSource for PythonBackendClient { ... }
```

---

## アプリ状態への統合

### `BackendState` の追加

```rust
// src/message.rs (または backend/mod.rs)
#[derive(Debug, Clone)]
pub enum BackendMessage {
    Started(String),        // base_url
    Failed(String),         // エラー詳細
}

// Flowsurface 構造体に追加
pub struct Flowsurface {
    // ... 既存フィールド ...
    backend_state: BackendState,
    data_source: Option<Arc<dyn KlineDataSource>>,
}

#[derive(Default)]
pub enum BackendState {
    #[default]
    Starting,
    Ready,
    Failed(String),
}
```

### `main.rs` の起動シーケンス

`iced::daemon` は `new()` で `(State, Task<Message>)` を返せる。

```rust
fn new() -> (Flowsurface, Task<Message>) {
    let state = Flowsurface::default(); // backend_state: Starting
    let task = Task::perform(
        async { backend::process::PythonBackend::spawn().await },
        |result| match result {
            Ok(backend) => Message::Backend(BackendMessage::Started(backend.base_url())),
            Err(e)      => Message::Backend(BackendMessage::Failed(e.to_string())),
        },
    );
    (state, task)
}
```

`update()` で `BackendMessage::Started` を受け取ったら `PythonBackendClient` を生成し `data_source` にセット。

### UI フィードバック（`dashboard.rs`）

```
BackendState::Starting  → replay ボタン disabled + "バックエンド起動中..." ツールチップ
BackendState::Ready     → 通常表示
BackendState::Failed(e) → replay ボタン disabled + エラートースト
```

---

## `replay/loader.rs` の完成形

```rust
/// リプレイ用 kline ロード。データ取得は data_source に委譲する。
pub async fn load_klines(
    stream: StreamKind,
    range: Range<u64>,
    data_source: Arc<dyn KlineDataSource>,
) -> Result<KlineLoadResult, String> {
    let (ticker_info, timeframe) = extract_stream_info(&stream)?;

    // Tachibana は引き続き Rust の fetcher を使用
    if ticker_info.ticker.exchange == Exchange::Tachibana {
        return fetch_tachibana(stream, range).await;
    }

    let klines = data_source
        .fetch_klines(&ticker_info, timeframe, range.clone())
        .await
        .map_err(|e| e.to_string())?;

    Ok(KlineLoadResult { stream, range, klines })
}
```

`Task::perform(load_klines(stream, range, Arc::clone(&self.data_source)), ...)` で呼ぶ。  
`data_source` は `Flowsurface` から `ReplayController` に渡す。

---

## Python プロジェクト構成

```
python_backend/
├── pyproject.toml              # uv 管理（fastapi, uvicorn[standard], httpx）
├── main.py                     # FastAPI app + lifespan（起動/終了ログ）
├── data_service/
│   ├── __init__.py
│   ├── models.py               # KlineRow (pydantic), KlineResponse
│   ├── cache.py                # CSV.gz 読み書き（kline_cache.md 仕様準拠）
│   ├── fetcher.py              # cache → miss 時は adapter へ
│   └── adapters/
│       ├── __init__.py
│       ├── base.py             # fetch_klines(ticker, timeframe, from_ms, to_ms) → protocol
│       ├── binance.py          # BinanceLinear / BinanceSpot
│       ├── bybit.py            # BybitLinear / BybitSpot
│       └── okx.py              # OkxLinear
└── tests/
    ├── test_cache.py           # save/load roundtrip, 月またぎ, 現在月スキップ
    └── test_fetcher.py         # cache hit / miss シナリオ（adapter をモック）
```

---

## API 仕様（port 8765）

### `GET /api/health`
```json
{ "status": "ok", "version": "0.1.0" }
```

### `GET /api/data/klines`

**クエリパラメータ:**

| パラメータ | 型 | 例 |
|---|---|---|
| `ticker` | string | `BinanceLinear:BTCUSDT` |
| `timeframe` | string | `M1`, `H1`, `D1` |
| `from` | i64 (ms) | `1743487200000` |
| `to` | i64 (ms) | `1743490800000` |

**レスポンス（200）:**
```json
{
  "klines": [
    {
      "time": 1743487200000,
      "open": 83000.5,
      "high": 83100.0,
      "low": 82900.0,
      "close": 83050.0,
      "volume_total": 1500.5,
      "volume_buy": 800.0,
      "volume_sell": 700.5
    }
  ],
  "source": "cache"
}
```

**エラー（422）:**
```json
{ "detail": "unsupported ticker: Tachibana:7203" }
```

### `GET /api/data/cache-status`
```json
{
  "cache_dir": "C:/Users/.../AppData/Roaming/flowsurface/market_data/kline_cache",
  "total_files": 12,
  "total_size_mb": 45.2
}
```

---

## キャッシュ仕様（`kline_cache.md` 準拠・Python 実装）

### ファイルパス
```
%APPDATA%/flowsurface/market_data/kline_cache/
  {exchange}_{symbol}_{timeframe}_{period}.csv.gz
```
Python では `platformdirs.user_data_dir("flowsurface")` で解決する。

### CSV フォーマット（`kline_cache.md` と同一）
```
Date,Time,Open,High,Low,Close,VolumeTotal,VolumeBuy,VolumeSell
2025-04-15,04:49,83000.5,83100.0,82900.0,83050.0,1500.5,800.0,700.5
```

### period 粒度（`kline_cache.md` と同一）
| timeframe | 粒度 |
|---|---|
| M1〜H12 | 月次（YYYYMM） |
| D1 | 年次（YYYY） |

### キャッシュ戦略（`kline_cache.md` と同一）
- 全期間ファイルが揃っていれば cache hit → Exchange API 不要
- 1 ファイルでも欠落 → range 全体を API フェッチ → 全ファイル保存
- **現在月は保存しない**（不完全データ防止）
- 過去月は immutable（TTL チェック不要）
- Tachibana は対象外

---

## 2系統制御設計（Python コントローラーモード）

GUI操作に加え、**Python スクリプトからリプレイを完全制御**できる2系統を実現する。  
Python ↔ Rust 間の制御通信は **WebSocket**（port 9877）を使用する。

### ポート役割の整理

| ポート | プロトコル | 用途 |
|---|---|---|
| **8765** | HTTP (FastAPI) | Python バックエンド: kline データ配信 |
| **9876** | HTTP (手製 TCP) | E2E テスト用 bash/Python スクリプト向け制御 API（既存・変更なし） |
| **9877** | WebSocket (tokio-tungstenite) | Python コントローラー ↔ Rust リアルタイム制御（新規） |

### 責任分担の再定義

```
Python Controller                      Rust App
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
current_time 所有・再生ループ           表示・仮想約定エンジン
銘柄・期間・速度の状態管理              GUI 描画のみ（受動的）
注文アクション発行                      注文をバーチャル取引所に通す

port 8765 HTTP ←── kline データ ──   (Rust が Python に取得しに行く)
port 9877 WS  ───  制御メッセージ → (Python が Rust に送りつける)
```

### WebSocket メッセージ仕様（Python → Rust, port 9877）

Rust が WebSocket サーバー、Python がクライアントとして接続する。  
メッセージはすべて JSON、`type` フィールドで種別を識別する。

**銘柄・期間登録:**
```json
{"type": "register_symbol", "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1"}
{"type": "set_range", "from_ms": 1743487200000, "to_ms": 1743490800000}
```

**再生制御:**
```json
{"type": "play"}
{"type": "pause"}
{"type": "stop"}
```

**tick（再生中、毎ステップ送信）:**
```json
{"type": "tick", "current_time_ms": 1743487260000}
```

**注文アクション:**
```json
{"type": "order", "side": "buy", "qty": 0.001, "price": 83000.5}
```

**Rust → Python（ACK・状態通知）:**
```json
{"type": "ack", "for": "register_symbol", "ok": true}
{"type": "state", "mode": "passive", "current_time_ms": 1743487260000}
{"type": "error", "message": "unsupported ticker"}
```

### Rust WebSocket サーバー（`src/ws_api.rs` 新規）

```rust
// tokio-tungstenite を使用
pub async fn start_ws_server(sender: mpsc::Sender<ApiMessage>) {
    let listener = TcpListener::bind("127.0.0.1:9877").await.unwrap();
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let sender = sender.clone();
        tokio::spawn(handle_ws_connection(stream, sender));
    }
}

async fn handle_ws_connection(stream: TcpStream, sender: mpsc::Sender<ApiMessage>) {
    let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    let (mut write, mut read) = ws.split();
    while let Some(Ok(msg)) = read.next().await {
        if let Message::Text(text) = msg {
            match parse_ws_message(&text) {
                Ok(cmd) => {
                    let (reply_tx, reply_rx) = oneshot::channel();
                    let _ = sender.send((cmd, ReplySender::new(reply_tx))).await;
                    // ACK を返す
                    if let Ok((status, body)) = reply_rx.await {
                        let _ = write.send(Message::Text(body)).await;
                    }
                }
                Err(e) => {
                    let _ = write.send(Message::Text(
                        format!(r#"{{"type":"error","message":"{e}"}}"#)
                    )).await;
                }
            }
        }
    }
}
```

`parse_ws_message()` は `type` フィールドを見て `ApiCommand` に変換する。  
既存の `ApiCommand` enum と `ReplySender` をそのまま再利用するため、  
iced の `update()` ループへのメッセージ配送は port 9876 と共通パスになる。

### Rust `clock.rs` の変更

現在の `ReplayClock` は wall clock を見て自律的に時刻を進める。  
Python 駆動モードでは **パッシブモード** を追加する。

```rust
pub enum ClockMode {
    /// GUI 操作時: 従来通り自律的に tick()
    Autonomous,
    /// Python コントローラー時: set_time() で外部から時刻を受け取る
    Passive { current_time_ms: u64 },
}
```

WS `tick` メッセージ → `ApiCommand::SetTime(u64)` → `clock.set_time(ms)` で描画更新。  
`play` / `pause` / `stop` は既存の `ApiCommand` にマッピング。

### `ApiCommand` の追加バリアント

```rust
// src/replay/controller/api.rs に追加
pub enum ApiCommand {
    // ... 既存 ...
    RegisterSymbol { ticker: String, timeframe: String },
    SetRange { from_ms: u64, to_ms: u64 },
    SetTime { current_time_ms: u64 },
    // VirtualExchangeCommand::Order は既存で対応可能か確認
}
```

### Python 側の再生ループ（`controller/replay.py`）

```python
import asyncio, json
import websockets

class ReplayController:
    current_time_ms: int      # Python が所有
    from_ms: int
    to_ms: int
    speed: float              # 1.0 = リアルタイム
    step_ms: int              # 例: 60_000 (1分足)

    async def run(self):
        async with websockets.connect("ws://127.0.0.1:9877") as ws:
            # 銘柄・期間を登録
            await ws.send(json.dumps({"type": "register_symbol",
                                      "ticker": self.ticker, "timeframe": self.timeframe}))
            await ws.send(json.dumps({"type": "set_range",
                                      "from_ms": self.from_ms, "to_ms": self.to_ms}))
            await ws.send(json.dumps({"type": "play"}))

            # 再生ループ
            while self.current_time_ms <= self.to_ms:
                await ws.send(json.dumps({"type": "tick",
                                          "current_time_ms": self.current_time_ms}))
                self.current_time_ms += self.step_ms
                await asyncio.sleep(self.step_ms / 1000 / self.speed)

            await ws.send(json.dumps({"type": "stop"}))
```

### Python 側ファイル追加（`python_backend/` 内）

```
python_backend/
├── controller/
│   ├── __init__.py
│   ├── replay.py       # ReplayController クラス（WS 接続・再生ループ）
│   └── models.py       # ReplayConfig (pydantic)
└── examples/
    └── basic_replay.py # 使用例スクリプト
```

### 使用例（ユーザースクリプト）

```python
import asyncio
from controller.replay import ReplayController

async def main():
    ctrl = ReplayController(
        ticker="BinanceLinear:BTCUSDT",
        timeframe="M1",
        from_ms=1743487200000,
        to_ms=1743490800000,
        speed=10.0,
    )
    await ctrl.run()  # WS 接続 → 銘柄登録 → 再生ループ

asyncio.run(main())
```

### 設計上の注意

- **tick 頻度**: step_ms / speed に比例。M1足・speed=10 なら 100ms ごと。HTTP と違いコネクション確立コストがゼロ。
- **GUI との排他**: `ClockMode::Passive` 中は GUI の play/pause ボタンを disabled 表示（Python が制御主体）。GUI がリプレイを開始したら `ClockMode::Autonomous`。
- **既存 port 9876 は変更しない**: E2E テスト用の bash スクリプトは引き続き HTTP で動作する。WS は Python コントローラー専用。
- **接続断時の挙動**: WS が切断されたら `ApiCommand::Stop` を自動発行し `ClockMode::Autonomous` に戻す。
- **`register_symbol` と PaneCommand**: 既存の `PaneCommand::AddPane` に ticker を渡す形に落とし込む。チャートペイン追加の既存パスを再利用。
- **注文 API**: `VirtualExchangeCommand` の既存バリアントを流用できるか `controller/api.rs` で確認してからマッピング設計する。
- **Cargo.toml 追加依存**: `tokio-tungstenite`, `futures-util`（WS split 用）。

---

## 実装マイルストーン

### Phase 1: Python バックエンド基盤
- [ ] `python_backend/pyproject.toml` 作成（fastapi, uvicorn, httpx, platformdirs）
- [ ] `data_service/models.py` — KlineRow, KlineResponse (pydantic)
- [ ] `data_service/cache.py` — CSV.gz 読み書き（kline_cache.md 準拠）
- [ ] `data_service/adapters/binance.py` — BinanceLinear/Spot kline fetch
- [ ] `data_service/fetcher.py` — cache hit/miss ロジック
- [ ] `main.py` — FastAPI app、`/api/health`、`/api/data/klines` 実装

### Phase 2: Rust バックエンドモジュール
- [ ] `src/backend/error.rs` — BackendError
- [ ] `src/backend/process.rs` — PythonBackend::spawn()、health check、Drop
- [ ] `src/backend/client.rs` — KlineDataSource trait + PythonBackendClient
- [ ] `src/backend/mod.rs` — pub use

### Phase 3: Rust アプリ統合
- [ ] `src/message.rs` — BackendMessage 追加
- [ ] Flowsurface 構造体に `backend_state`, `data_source` フィールド追加
- [ ] `main.rs` の `new()` で spawn Task を返す
- [ ] `update()` の BackendMessage ハンドラ
- [ ] `replay/loader.rs` — `data_source` 引数を受け取る形に変更、非 Tachibana の fetch 削除
- [ ] ReplayController に `data_source: Arc<dyn KlineDataSource>` を渡す
- [ ] UI: BackendState::Starting / Failed 時のフィードバック

### Phase 4: テスト
- [ ] `python_backend/tests/test_cache.py` — roundtrip, 月またぎ, 現在月スキップ
- [ ] `python_backend/tests/test_fetcher.py` — cache hit/miss（adapter モック）
- [ ] Rust: `MockKlineDataSource` を `#[cfg(test)]` で実装
- [ ] E2E: 2 回目の replay 開始が 5 秒以内（キャッシュヒット確認）

### Phase 5: Python コントローラー（2系統制御・WebSocket）
- [ ] `Cargo.toml` — `tokio-tungstenite`, `futures-util` 追加
- [ ] `src/ws_api.rs` — WebSocket サーバー（port 9877）、`parse_ws_message()` 実装
- [ ] `src/main.rs` — `ws_api::start_ws_server()` を起動 Task に追加
- [ ] `src/replay/controller/api.rs` — `RegisterSymbol`, `SetRange`, `SetTime` バリアント追加
- [ ] `src/replay/clock.rs` — `ClockMode::Passive` 追加、`set_time(ms)` 実装
- [ ] `src/screen/dashboard.rs` — `ClockMode::Passive` 時に GUI play/pause を disabled 表示
- [ ] WS 切断時に `ApiCommand::Stop` + `ClockMode::Autonomous` へ戻す処理
- [ ] `python_backend/pyproject.toml` — `websockets` 追加
- [ ] `python_backend/controller/replay.py` — `ReplayController`（WS 接続・再生ループ）
- [ ] `python_backend/examples/basic_replay.py` — 使用例

### Phase 6: 追加 adapter
- [ ] `data_service/adapters/bybit.py`
- [ ] `data_service/adapters/okx.py`

---

## 設計上の注意点

1. **`uv run` でランタイム管理**: `python_backend/pyproject.toml` に依存を記述。ユーザーが pip install する必要なし。Rust から `uv run uvicorn main:app --host 127.0.0.1 --port 8765` を起動する。

2. **stdout/stderr をロギング**: `tokio::process::Command` の `stdout(Stdio::piped())` でキャプチャし、`log::debug!("[python]")` に流す。プロセスのクラッシュを Rust 側のログで追える。

3. **`PythonBackend` は `Flowsurface` が所有**: Drop で kill されるので、iced が終了すれば Python も終了する。`Arc<PythonBackend>` にしない（所有権を1箇所に集中）。

4. **`KlineDataSource` は trait object**: `Arc<dyn KlineDataSource>` で `ReplayController` に渡す。テストで `MockKlineDataSource` に差し替え可能。

5. **Tachibana は Rust のまま**: `fetch_tachibana_daily_klines()` は `connector/fetcher.rs` に残す。session 認証が Rust 側にあるため Python 移管コストが高い。Phase 5 以降で検討。

6. **live チャート fetch は変更しない**: `connector/fetcher.rs` の `kline_fetch_task()` / `request_fetch()` はリプレイ用ではなくライブチャートのヒストリカルデータ取得。Python バックエンドとは別系統。

7. **`⏳kline_cache.md` は本計画で吸収**: `src/replay/cache.rs` は作成不要。計画書をアーカイブ完了扱いに更新する。

---

## 進捗

- ✅ 計画書作成・見直し
- [ ] Phase 1: Python バックエンド基盤
- [ ] Phase 2: Rust バックエンドモジュール
- [ ] Phase 3: Rust アプリ統合
- [ ] Phase 4: テスト
- [ ] Phase 5: Python コントローラー（2系統制御）
- [ ] Phase 6: 追加 adapter
