use crate::narrative::model::NarrativeAction;
use crate::replay::ReplayCommand;
use futures::SinkExt;
use futures::channel::mpsc;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// API ハンドラーが実行するコマンド。既存 ReplayCommand と新規 PaneCommand の union。
#[derive(Debug, Clone)]
pub enum ApiCommand {
    Replay(ReplayCommand),
    Pane(PaneCommand),
    /// 認証状態確認コマンド（テスト・デバッグ用、本番ビルドにも含まれる）。
    Auth(AuthCommand),
    /// 仮想約定エンジンコマンド（Phase 2 互換）。
    VirtualExchange(VirtualExchangeCommand),
    /// ナラティブ API コマンド（Phase 4a）。
    Narrative(NarrativeCommand),
    /// Agent 専用 Replay API コマンド（Phase 4b-1）。
    AgentSession(AgentSessionCommand),
    /// 立花証券余力情報を取得する（GET /api/buying-power）。
    FetchBuyingPower,
    /// 立花証券新規注文を発注する（POST /api/tachibana/order）。
    TachibanaNewOrder {
        req: Box<exchange::adapter::tachibana::NewOrderRequest>,
    },
    /// 立花証券注文一覧を取得する（GET /api/tachibana/orders）。
    /// `eig_day`: 執行予定日 YYYYMMDD。空文字=全件。
    FetchTachibanaOrders {
        eig_day: String,
    },
    /// 立花証券約定明細を取得する（GET /api/tachibana/order/{order_num}）。
    FetchTachibanaOrderDetail {
        order_num: String,
        eig_day: String,
    },
    /// 立花証券訂正注文を発注する（POST /api/tachibana/order/correct）。
    TachibanaCorrectOrder {
        req: Box<exchange::adapter::tachibana::CorrectOrderRequest>,
    },
    /// 立花証券取消注文を発注する（POST /api/tachibana/order/cancel）。
    TachibanaOrderCancel {
        req: Box<exchange::adapter::tachibana::CancelOrderRequest>,
    },
    /// 保有現物株数を取得する（GET /api/tachibana/holdings?issue_code=XXXX）。
    FetchTachibanaHoldings {
        issue_code: String,
    },
    /// E2E テスト用コマンド（debug ビルドで有効）。
    #[cfg(debug_assertions)]
    Test(TestCommand),
}

/// 仮想約定エンジン API コマンド。
#[derive(Debug, Clone)]
pub enum VirtualExchangeCommand {
    /// 仮想注文を登録する（POST /api/replay/order）
    PlaceOrder {
        ticker: String,
        side: String, // "buy" | "sell"
        qty: f64,
        order_type: String, // "market" | "limit"
        limit_price: Option<f64>,
    },
    /// ポートフォリオスナップショットを取得する（GET /api/replay/portfolio）
    GetPortfolio,
    /// 観測データを取得する（GET /api/replay/state）— Phase 1 骨格のみ
    GetState,
    /// pending 注文の一覧を取得する（GET /api/replay/orders）
    GetOrders,
}

/// Agent 専用 Replay API コマンド（Phase 4b-1）。
///
/// ADR-0001 / phase4b_agent_replay_api.md §4 に基づく。UI リモコン API
/// （`/api/replay/*`）とは別経路で、型契約と決定論性を担保する。
#[derive(Debug, Clone)]
pub enum AgentSessionCommand {
    /// `POST /api/agent/session/:id/step`: 1 バー進行 + 副作用同梱。
    Step { session_id: String },
    /// `POST /api/agent/session/:id/order`: 仮想注文（冪等性あり）。
    PlaceOrder {
        session_id: String,
        request: Box<crate::api::order_request::AgentOrderRequest>,
    },
}

/// ナラティブ API コマンド（Phase 4a）。
#[derive(Debug, Clone)]
pub enum NarrativeCommand {
    /// `POST /api/agent/narrative`: ナラティブを新規作成する。
    Create(Box<NarrativeCreateRequest>),
    /// `GET /api/agent/narratives?agent_id=&ticker=&since_ms=&limit=`
    List(NarrativeListQuery),
    /// `GET /api/agent/narrative/:id`: メタ JSON を返す。
    Get { id: uuid::Uuid },
    /// `GET /api/agent/narrative/:id/snapshot`: 本体 JSON を返す（gzip 解凍 + sha256 検証）。
    GetSnapshot { id: uuid::Uuid },
    /// `PATCH /api/agent/narrative/:id {"public": bool}`.
    Patch { id: uuid::Uuid, public: bool },
    /// `GET /api/agent/narratives/storage`.
    StorageStats,
    /// `GET /api/agent/narratives/orphans`.
    Orphans,
}

/// `POST /api/agent/narrative` のリクエストボディ（バリデーション済み）。
#[derive(Debug, Clone)]
pub struct NarrativeCreateRequest {
    pub agent_id: String,
    pub uagent_address: Option<String>,
    pub ticker: String,
    pub timeframe: String,
    pub observation_snapshot: serde_json::Value,
    pub reasoning: String,
    pub action: NarrativeAction,
    pub confidence: f64,
    pub linked_order_id: Option<String>,
    /// クライアント指定がなければサーバー側の `StepClock::now_ms()` を使う。
    pub timestamp_ms: Option<i64>,
    pub idempotency_key: Option<String>,
}

/// `GET /api/agent/narratives` のクエリパラメータ。
#[derive(Debug, Clone, Default)]
pub struct NarrativeListQuery {
    pub agent_id: Option<String>,
    pub ticker: Option<String>,
    pub since_ms: Option<i64>,
    pub limit: Option<usize>,
}

/// 認証状態確認コマンド。
#[derive(Debug, Clone)]
pub enum AuthCommand {
    /// 現在の立花証券セッション有無を返す（`{"session":"present"|"none"}`）。
    TachibanaSessionStatus,
    /// 立花証券セッションを明示的にログアウトする（`POST /api/auth/tachibana/logout`）。
    /// メモリセッション + keyring を両方クリアする。CI teardown での競合防止用。
    TachibanaLogout,
}

/// E2E テスト fixture 注入コマンド。
#[cfg(debug_assertions)]
#[derive(Debug, Clone)]
pub enum TestCommand {
    /// メモリセッション + keyring セッションを両方クリアする（debug ビルドで有効）。
    TachibanaDeletePersistedSession,
}

/// ペイン CRUD 系コマンド（§6.2 #2/#5/#6/#7/#8 テスト用）。
#[derive(Debug, Clone)]
pub enum PaneCommand {
    /// 全ペインのメタ情報 + リプレイバッファ状態を返す
    ListPanes,
    /// ペインを分割する。axis: "Vertical" | "Horizontal"
    /// new_content は無視（既存 pane::Message::SplitPane は Starter しか生成しない）
    Split { pane_id: uuid::Uuid, axis: String },
    /// ペインを閉じる
    Close { pane_id: uuid::Uuid },
    /// ペインのストリームを別 ticker に差し替える（SerTicker 形式 "BinanceLinear:BTCUSDT"）
    SetTicker { pane_id: uuid::Uuid, ticker: String },
    /// ペインのタイムフレームを変更する（"M1" 〜 "D1"）
    SetTimeframe {
        pane_id: uuid::Uuid,
        timeframe: String,
    },
    /// Sidebar::TickerSelected 経路（Phase 8 Fix 4 検証用）。
    /// `kind` が None → `switch_tickers_in_group` 経路、Some → `init_focused_pane` 経路。
    /// どちらの経路でも `SyncReplayBuffers` chain が発火する（main.rs 内の `Message::Sidebar` ハンドラと同じコード）。
    SidebarSelectTicker {
        pane_id: uuid::Uuid,
        ticker: String,
        kind: Option<String>,
    },
    /// 現在の通知（Toast）一覧を取得する。§6.2 #10 backfill 失敗検証用。
    ListNotifications,
    /// 指定ペインのチャートスナップショット（バー数・タイムスタンプ範囲・OHLCV バー配列）を返す。
    /// クエリパラメータ: `?pane_id=<uuid>[&limit=N][&since_ts=<ms>]`
    GetChartSnapshot {
        pane_id: uuid::Uuid,
        limit: Option<usize>,
        since_ts: Option<u64>,
    },
    /// 注文ペインを開く（POST /api/sidebar/open-order-pane）
    /// kind: "OrderEntry" | "OrderList" | "BuyingPower"
    OpenOrderPane { kind: String },
}

type ReplySenderInner = Arc<Mutex<Option<oneshot::Sender<(u16, String)>>>>;

/// oneshot::Sender を Clone 可能にするラッパー（iced の Message は Clone が必要）
/// レスポンスは main.rs 側でシリアライズ済み JSON を送る。
/// タプル (status_code, body) でステータスコードを指定できる。
#[derive(Debug, Clone)]
pub struct ReplySender(ReplySenderInner);

impl ReplySender {
    fn new(tx: oneshot::Sender<(u16, String)>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// HTTP 200 でレスポンスを送信する。2回目以降の呼び出しは何もしない。
    pub fn send(self, body: String) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send((200, body));
        }
    }

    /// 任意のステータスコードでレスポンスを送信する。2回目以降の呼び出しは何もしない。
    pub fn send_status(self, status: u16, body: String) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send((status, body));
        }
    }
}

/// API サーバーから iced に送るメッセージ（コマンド + 応答用チャネル）
pub type ApiMessage = (ApiCommand, ReplySender);

/// channel() パターンで API サーバーを起動し、Message ストリームを返す。
/// exchange/src/connect.rs:111-122 の再利用パターン。
pub fn subscription() -> impl futures::Stream<Item = ApiMessage> {
    exchange::connect::channel(32, |sender| async move {
        run_server(sender).await;
    })
}

/// headless モード向け: 外部から sender を渡して HTTP サーバーを起動する。
pub async fn start_server(sender: futures::channel::mpsc::Sender<ApiMessage>) {
    run_server(sender).await;
}

/// ポート番号を環境変数または デフォルト 9876 から取得
fn api_port() -> u16 {
    std::env::var("FLOWSURFACE_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9876)
}

/// Content-Length ヘッダーの値をパースする（見つからなければ 0）。
fn parse_content_length_from_headers(headers: &str) -> usize {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("content-length:")
            && let Some((_, val)) = line.split_once(':')
            && let Ok(n) = val.trim().parse::<usize>()
        {
            return n;
        }
    }
    0
}

/// 上限: ヘッダー 64KB + ボディ 16MB（Phase 4a のスナップショット 10MB ＋余裕を見た値）。
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

pub(crate) enum ReadRequestOutcome {
    /// リクエストを全量受信した（ヘッダー + ボディ）
    Ok(String),
    /// Content-Length がサーバー上限を超えた → 413 を返すべき
    TooLarge,
    /// 接続断・ヘッダー解析失敗など。呼び出し側は 400/黙って切断を選べる
    Invalid,
}

/// HTTP リクエストを完全に読み込む（Content-Length に従ってボディも確保）。
/// TCP が分割して届いても正しく結合し、ボディサイズに応じてバッファを動的拡張する。
pub(crate) async fn read_full_request(stream: &mut tokio::net::TcpStream) -> ReadRequestOutcome {
    let mut buf = vec![0u8; 16 * 1024];
    let mut total = 0usize;

    loop {
        if total == buf.len() {
            if buf.len() >= MAX_HEADER_BYTES {
                return ReadRequestOutcome::Invalid;
            }
            let new_size = (buf.len() * 2).min(MAX_HEADER_BYTES);
            buf.resize(new_size, 0);
        }

        let n = match stream.read(&mut buf[total..]).await {
            Ok(0) | Err(_) => return ReadRequestOutcome::Invalid,
            Ok(n) => n,
        };
        total += n;

        let Some(header_end) = buf[..total].windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };

        let body_start = header_end + 4;
        let Ok(headers_raw) = std::str::from_utf8(&buf[..header_end]) else {
            return ReadRequestOutcome::Invalid;
        };
        let content_length = parse_content_length_from_headers(headers_raw);

        if content_length > MAX_BODY_BYTES {
            return ReadRequestOutcome::TooLarge;
        }

        let body_received = total - body_start;
        if body_received >= content_length {
            return ReadRequestOutcome::Ok(String::from_utf8_lossy(&buf[..total]).into_owned());
        }

        let total_len = body_start + content_length;
        if buf.len() < total_len {
            buf.resize(total_len, 0);
        }
        match stream.read_exact(&mut buf[total..total_len]).await {
            Ok(_) => {
                return ReadRequestOutcome::Ok(
                    String::from_utf8_lossy(&buf[..total_len]).into_owned(),
                );
            }
            Err(_) => return ReadRequestOutcome::Invalid,
        }
    }
}

async fn run_server(mut sender: mpsc::Sender<ApiMessage>) {
    let port = api_port();
    let addr = format!("127.0.0.1:{port}");

    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => {
            log::info!("Replay API server listening on {addr}");
            l
        }
        Err(e) => {
            log::error!("Failed to bind replay API server on {addr}: {e}");
            return;
        }
    };

    loop {
        let (mut stream, _peer) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                log::warn!("Replay API accept error: {e}");
                continue;
            }
        };

        // 1 リクエスト / 接続（keep-alive なし）
        // ヘッダー 64KB / ボディ 16MB まで動的に拡張する（Phase 4a のスナップショット 10MB 対応）
        let request_string = match read_full_request(&mut stream).await {
            ReadRequestOutcome::Ok(s) => s,
            ReadRequestOutcome::TooLarge => {
                let _ = write_response(&mut stream, 413, r#"{"error":"Payload Too Large"}"#).await;
                continue;
            }
            ReadRequestOutcome::Invalid => continue,
        };
        let request = request_string.as_str();
        let (method, path, body) = match parse_request(request) {
            Some(parsed) => parsed,
            None => {
                let _ = write_response(&mut stream, 400, r#"{"error":"Bad Request"}"#).await;
                continue;
            }
        };

        // スクリーンショット: iced app state 不要なため直接ここで処理
        if method == "POST" && path == "/api/app/screenshot" {
            let json = tokio::task::spawn_blocking(capture_screenshot)
                .await
                .unwrap_or_else(|e| format!(r#"{{"ok":false,"error":"task panic: {e}"}}"#));
            let _ = write_response(&mut stream, 200, &json).await;
            continue;
        }

        let command = match route(&method, &path, &body) {
            Ok(cmd) => cmd,
            Err(RouteError::NotFound) => {
                let _ = write_response(&mut stream, 404, r#"{"error":"Not Found"}"#).await;
                continue;
            }
            Err(RouteError::BadRequest) => {
                let _ = write_response(
                    &mut stream,
                    400,
                    r#"{"error":"Bad Request: invalid JSON body"}"#,
                )
                .await;
                continue;
            }
            Err(RouteError::PayloadTooLarge) => {
                let _ = write_response(&mut stream, 413, r#"{"error":"Payload Too Large"}"#).await;
                continue;
            }
            Err(RouteError::NotImplemented) => {
                let _ =
                    write_response(&mut stream, 501, NOT_IMPLEMENTED_MULTI_SESSION_BODY).await;
                continue;
            }
        };

        // oneshot で iced app からのレスポンスを待つ
        let (reply_tx, reply_rx) = oneshot::channel::<(u16, String)>();
        if sender
            .send((command, ReplySender::new(reply_tx)))
            .await
            .is_err()
        {
            let _ = write_response(&mut stream, 500, r#"{"error":"App channel closed"}"#).await;
            continue;
        }

        match reply_rx.await {
            Ok((status, json)) => {
                let _ = write_response(&mut stream, status, &json).await;
            }
            Err(_) => {
                let _ =
                    write_response(&mut stream, 500, r#"{"error":"No response from app"}"#).await;
            }
        }
    }
}

/// 簡易 HTTP リクエストパーサー。(method, path, body) を返す。
fn parse_request(raw: &str) -> Option<(String, String, String)> {
    let mut lines = raw.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    // HTTP/1.x は無視

    // body はヘッダー後の空行の後
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        raw[pos + 4..].to_string()
    } else {
        String::new()
    };

    Some((method, path, body))
}

#[derive(Debug)]
enum RouteError {
    NotFound,
    BadRequest,
    PayloadTooLarge,
    /// 501 Not Implemented — Phase 4b-1 時点で `session_id != "default"` を
    /// 明示拒否する。詳細は ADR-0001 §Risks。
    NotImplemented,
}

/// `RouteError::NotImplemented` に対応する 501 レスポンス本文。
/// ADR-0001 / phase4b_agent_replay_api.md §4.5 で固定文言として定義。
pub(crate) const NOT_IMPLEMENTED_MULTI_SESSION_BODY: &str =
    r#"{"error":"multi-session not yet implemented; use 'default' until Phase 4c"}"#;

/// body から文字列フィールドを取り出す
fn body_str_field(body: &str, key: &str) -> Result<String, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    parsed
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)
}

/// body から uuid フィールドを取り出す
fn body_uuid_field(body: &str, key: &str) -> Result<uuid::Uuid, RouteError> {
    let s = body_str_field(body, key)?;
    uuid::Uuid::parse_str(&s).map_err(|_| RouteError::BadRequest)
}

/// `/api/agent/session/:id/<suffix>` 形式のパスから `:id` を抽出する。
/// `:id` が空・`/` を含むなどの場合は `BadRequest`。
/// ADR-0001 に基づき `:id != "default"` は `NotImplemented`（501）。
fn extract_agent_session_id<'a>(
    path: &'a str,
    suffix: &str,
) -> Result<&'a str, RouteError> {
    let after_prefix = path
        .strip_prefix("/api/agent/session/")
        .ok_or(RouteError::NotFound)?;
    let end_marker = format!("/{suffix}");
    let session_id = after_prefix
        .strip_suffix(&end_marker)
        .ok_or(RouteError::NotFound)?;
    if session_id.is_empty() || session_id.contains('/') {
        return Err(RouteError::BadRequest);
    }
    Ok(session_id)
}

fn parse_agent_session_step(path: &str) -> Result<ApiCommand, RouteError> {
    let session_id = extract_agent_session_id(path, "step")?;
    if session_id != "default" {
        return Err(RouteError::NotImplemented);
    }
    Ok(ApiCommand::AgentSession(AgentSessionCommand::Step {
        session_id: session_id.to_string(),
    }))
}

fn parse_agent_session_order(path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    let session_id = extract_agent_session_id(path, "order")?;
    if session_id != "default" {
        return Err(RouteError::NotImplemented);
    }
    let request = crate::api::order_request::parse_agent_order_request(body)
        .map_err(|_| RouteError::BadRequest)?;
    Ok(ApiCommand::AgentSession(AgentSessionCommand::PlaceOrder {
        session_id: session_id.to_string(),
        request: Box::new(request),
    }))
}

/// "YYYY-MM-DD HH:MM" 形式の日時文字列を検証する。不正なら RouteError::BadRequest を返す。
fn validate_datetime_str(s: &str) -> Result<(), RouteError> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .map(|_| ())
        .map_err(|_| RouteError::BadRequest)
}

/// パスとメソッドから ApiCommand にルーティング
///
/// C-3: 以前は `return` を使った 2 つの match ブロックに分かれていた。
/// 単一の match 式に統合し、複雑な body パースは専用ヘルパー関数に切り出した。
fn route(method: &str, path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    match (method, path) {
        // ── Replay 制御 ────────────────────────────────────────────────────
        ("GET", "/api/replay/status") => Ok(ApiCommand::Replay(ReplayCommand::GetStatus)),
        ("POST", "/api/replay/toggle") => Ok(ApiCommand::Replay(ReplayCommand::Toggle)),
        ("POST", "/api/replay/play") => parse_play_command(body),
        ("POST", "/api/replay/pause") => Ok(ApiCommand::Replay(ReplayCommand::Pause)),
        ("POST", "/api/replay/resume") => Ok(ApiCommand::Replay(ReplayCommand::Resume)),
        ("POST", "/api/replay/step-forward") => Ok(ApiCommand::Replay(ReplayCommand::StepForward)),
        ("POST", "/api/replay/step-backward") => {
            Ok(ApiCommand::Replay(ReplayCommand::StepBackward))
        }
        ("POST", "/api/replay/speed") => Ok(ApiCommand::Replay(ReplayCommand::CycleSpeed)),

        // ── App 制御 ───────────────────────────────────────────────────────
        ("POST", "/api/app/save") => Ok(ApiCommand::Replay(ReplayCommand::SaveState)),
        ("POST", "/api/app/set-mode") => parse_set_mode_command(body),

        // ── 認証（本番ビルドにも含まれる）────────────────────────────────
        ("GET", "/api/auth/tachibana/status") => {
            Ok(ApiCommand::Auth(AuthCommand::TachibanaSessionStatus))
        }
        ("POST", "/api/auth/tachibana/logout") => {
            Ok(ApiCommand::Auth(AuthCommand::TachibanaLogout))
        }

        // ── ペイン CRUD ────────────────────────────────────────────────────
        ("GET", "/api/pane/list") => Ok(ApiCommand::Pane(PaneCommand::ListPanes)),
        ("GET", p) if p.starts_with("/api/pane/chart-snapshot") => parse_chart_snapshot_command(p),
        ("POST", "/api/pane/split") => parse_split_command(body),
        ("POST", "/api/pane/close") => {
            let pane_id = body_uuid_field(body, "pane_id")?;
            Ok(ApiCommand::Pane(PaneCommand::Close { pane_id }))
        }
        ("POST", "/api/pane/set-ticker") => {
            let pane_id = body_uuid_field(body, "pane_id")?;
            let ticker = body_str_field(body, "ticker")?;
            Ok(ApiCommand::Pane(PaneCommand::SetTicker { pane_id, ticker }))
        }
        ("POST", "/api/pane/set-timeframe") => {
            let pane_id = body_uuid_field(body, "pane_id")?;
            let timeframe = body_str_field(body, "timeframe")?;
            Ok(ApiCommand::Pane(PaneCommand::SetTimeframe {
                pane_id,
                timeframe,
            }))
        }

        // ── その他 ────────────────────────────────────────────────────────
        ("GET", "/api/notification/list") => Ok(ApiCommand::Pane(PaneCommand::ListNotifications)),
        ("POST", "/api/sidebar/select-ticker") => parse_sidebar_select_ticker(body),
        ("POST", "/api/sidebar/open-order-pane") => parse_open_order_pane(body),

        // ── 仮想約定エンジン（Phase 2 互換）──────────────────────────────
        ("POST", "/api/replay/order") => parse_virtual_order_command(body),
        ("GET", "/api/replay/portfolio") => Ok(ApiCommand::VirtualExchange(
            VirtualExchangeCommand::GetPortfolio,
        )),
        ("GET", "/api/replay/state") => Ok(ApiCommand::VirtualExchange(
            VirtualExchangeCommand::GetState,
        )),
        ("GET", "/api/replay/orders") => Ok(ApiCommand::VirtualExchange(
            VirtualExchangeCommand::GetOrders,
        )),

        // ── ナラティブ API（Phase 4a）──────────────────────────────────────
        ("POST", "/api/agent/narrative") => parse_narrative_create(body),
        ("GET", "/api/agent/narratives/storage") => {
            Ok(ApiCommand::Narrative(NarrativeCommand::StorageStats))
        }
        ("GET", "/api/agent/narratives/orphans") => {
            Ok(ApiCommand::Narrative(NarrativeCommand::Orphans))
        }
        ("GET", p) if p == "/api/agent/narratives" || p.starts_with("/api/agent/narratives?") => {
            parse_narrative_list(p)
        }
        ("GET", p) if p.starts_with("/api/agent/narrative/") && p.ends_with("/snapshot") => {
            parse_narrative_get_snapshot(p)
        }
        ("GET", p) if p.starts_with("/api/agent/narrative/") => parse_narrative_get(p),
        ("PATCH", p) if p.starts_with("/api/agent/narrative/") => parse_narrative_patch(p, body),

        // ── Agent 専用 Replay API（Phase 4b-1）────────────────────────────
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/step") => {
            parse_agent_session_step(p)
        }
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/order") => {
            parse_agent_session_order(p, body)
        }

        // ── 立花証券余力情報 ──────────────────────────────────────────────
        ("GET", "/api/buying-power") => Ok(ApiCommand::FetchBuyingPower),

        // ── 立花証券新規注文 ──────────────────────────────────────────────
        ("POST", "/api/tachibana/order") => parse_tachibana_new_order(body),

        // ── 立花証券注文管理 ──────────────────────────────────────────────
        ("GET", p) if p == "/api/tachibana/orders" || p.starts_with("/api/tachibana/orders?") => {
            let eig_day = query_param(p, "eig_day").unwrap_or_default();
            Ok(ApiCommand::FetchTachibanaOrders { eig_day })
        }
        ("GET", p) if p.starts_with("/api/tachibana/order/") => {
            parse_tachibana_order_detail_command(p)
        }
        ("POST", "/api/tachibana/order/correct") => parse_tachibana_correct_order(body),
        ("POST", "/api/tachibana/order/cancel") => parse_tachibana_cancel_order(body),

        // ── 立花証券保有現物株数 ──────────────────────────────────────────
        ("GET", p)
            if p == "/api/tachibana/holdings" || p.starts_with("/api/tachibana/holdings?") =>
        {
            let issue_code = query_param(p, "issue_code").ok_or(RouteError::BadRequest)?;
            Ok(ApiCommand::FetchTachibanaHoldings { issue_code })
        }

        // ── debug ビルドで有効（keyring クリア） ─────────────────────────
        #[cfg(debug_assertions)]
        ("POST", "/api/test/tachibana/delete-persisted-session") => Ok(ApiCommand::Test(
            TestCommand::TachibanaDeletePersistedSession,
        )),

        _ => Err(RouteError::NotFound),
    }
}

/// `POST /api/replay/order` のボディをパースして ApiCommand を返す。
fn parse_virtual_order_command(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;

    let ticker = parsed
        .get("ticker")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)?;

    let side = parsed
        .get("side")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
        .ok_or(RouteError::BadRequest)?;

    if side != "buy" && side != "sell" {
        return Err(RouteError::BadRequest);
    }

    let qty = parsed
        .get("qty")
        .and_then(|v| v.as_f64())
        .ok_or(RouteError::BadRequest)?;

    let (order_type, limit_price) = if let Some(ot) = parsed.get("order_type") {
        match ot {
            serde_json::Value::String(s) if s == "market" => ("market".to_string(), None),
            serde_json::Value::Object(obj) if obj.contains_key("limit") => {
                let lp = obj
                    .get("limit")
                    .and_then(|v| v.as_f64())
                    .ok_or(RouteError::BadRequest)?;
                ("limit".to_string(), Some(lp))
            }
            _ => return Err(RouteError::BadRequest),
        }
    } else {
        // order_type 省略 → market
        ("market".to_string(), None)
    };

    Ok(ApiCommand::VirtualExchange(
        VirtualExchangeCommand::PlaceOrder {
            ticker,
            side,
            qty,
            order_type,
            limit_price,
        },
    ))
}

/// URL パスのクエリ文字列から指定キーの値を取り出す。
/// 例: `/api/pane/chart-snapshot?pane_id=xxx` → `Some("xxx")`
fn query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let mut kv = pair.splitn(2, '=');
        if kv.next() == Some(key) {
            kv.next().map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// `GET /api/pane/chart-snapshot?pane_id=<uuid>[&limit=N][&since_ts=<ms>]` をパースして ApiCommand を返す。
fn parse_chart_snapshot_command(path: &str) -> Result<ApiCommand, RouteError> {
    let id_str = query_param(path, "pane_id").ok_or(RouteError::BadRequest)?;
    let pane_id = uuid::Uuid::parse_str(&id_str).map_err(|_| RouteError::BadRequest)?;
    let limit = query_param(path, "limit").and_then(|s| s.parse::<usize>().ok());
    let since_ts = query_param(path, "since_ts").and_then(|s| s.parse::<u64>().ok());
    Ok(ApiCommand::Pane(PaneCommand::GetChartSnapshot {
        pane_id,
        limit,
        since_ts,
    }))
}

/// `POST /api/app/set-mode` のボディをパースして ApiCommand を返す。
/// body: `{"mode": "live" | "replay"}`
fn parse_set_mode_command(body: &str) -> Result<ApiCommand, RouteError> {
    let mode = body_str_field(body, "mode")?;
    match mode.to_lowercase().as_str() {
        "live" | "replay" => Ok(ApiCommand::Replay(ReplayCommand::SetMode {
            mode: mode.to_lowercase(),
        })),
        _ => Err(RouteError::BadRequest),
    }
}

/// `POST /api/replay/play` のボディをパースして ApiCommand を返す。
fn parse_play_command(body: &str) -> Result<ApiCommand, RouteError> {
    let start = body_str_field(body, "start")?;
    let end = body_str_field(body, "end")?;
    validate_datetime_str(&start)?;
    validate_datetime_str(&end)?;
    Ok(ApiCommand::Replay(ReplayCommand::Play { start, end }))
}

/// `POST /api/pane/split` のボディをパースして ApiCommand を返す。
fn parse_split_command(body: &str) -> Result<ApiCommand, RouteError> {
    let pane_id = body_uuid_field(body, "pane_id")?;
    let axis = body_str_field(body, "axis")?;
    match axis.as_str() {
        "Vertical" | "vertical" | "Horizontal" | "horizontal" => {}
        _ => return Err(RouteError::BadRequest),
    }
    Ok(ApiCommand::Pane(PaneCommand::Split { pane_id, axis }))
}

/// `POST /api/sidebar/open-order-pane` のボディをパースして ApiCommand を返す。
fn parse_open_order_pane(body: &str) -> Result<ApiCommand, RouteError> {
    let kind = body_str_field(body, "kind")?;
    match kind.as_str() {
        "OrderEntry" | "OrderList" | "BuyingPower" => {}
        _ => return Err(RouteError::BadRequest),
    }
    Ok(ApiCommand::Pane(PaneCommand::OpenOrderPane { kind }))
}

/// `POST /api/tachibana/order` のボディをパースして ApiCommand を返す。
/// `second_password` は本文中のフィールドか `DEV_SECOND_PASSWORD` 環境変数から取得する。
fn parse_tachibana_new_order(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;

    let str_field = |key: &str| -> Result<String, RouteError> {
        parsed
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or(RouteError::BadRequest)
    };

    let issue_code = str_field("issue_code")?;
    let qty = str_field("qty")?;
    let side = str_field("side")?;
    let price = str_field("price")?;
    let account_type = parsed
        .get("account_type")
        .and_then(|v| v.as_str())
        .unwrap_or("1")
        .to_string();
    let market_code = parsed
        .get("market_code")
        .and_then(|v| v.as_str())
        .unwrap_or("00")
        .to_string();
    let condition = parsed
        .get("condition")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    let cash_margin = parsed
        .get("cash_margin")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    let expire_day = parsed
        .get("expire_day")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();

    // DEV_SECOND_PASSWORD フォールバックは debug ビルド（E2E テスト）専用
    let second_password = {
        let from_body = parsed
            .get("second_password")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        #[cfg(debug_assertions)]
        let pw = from_body.or_else(|| std::env::var("DEV_SECOND_PASSWORD").ok());
        #[cfg(not(debug_assertions))]
        let pw = from_body;
        pw.ok_or(RouteError::BadRequest)?
    };

    let req = exchange::adapter::tachibana::NewOrderRequest {
        account_type,
        issue_code,
        market_code,
        side,
        condition,
        price,
        qty,
        cash_margin,
        expire_day,
        second_password,
    };
    Ok(ApiCommand::TachibanaNewOrder { req: Box::new(req) })
}

/// `GET /api/tachibana/order/{order_num}[?eig_day=YYYYMMDD]` をパースして ApiCommand を返す。
fn parse_tachibana_order_detail_command(path: &str) -> Result<ApiCommand, RouteError> {
    // パス部分とクエリ部分を分離する
    let (path_part, _) = path.split_once('?').unwrap_or((path, ""));
    let order_num = path_part
        .strip_prefix("/api/tachibana/order/")
        .filter(|s| !s.is_empty())
        .ok_or(RouteError::BadRequest)?
        .to_string();
    let eig_day = query_param(path, "eig_day").unwrap_or_default();
    Ok(ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day })
}

/// `POST /api/tachibana/order/correct` のボディをパースして ApiCommand を返す。
/// `second_password` は本文または `DEV_SECOND_PASSWORD` 環境変数から取得する。
fn parse_tachibana_correct_order(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    let str_field = |key: &str| -> Result<String, RouteError> {
        parsed
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or(RouteError::BadRequest)
    };
    let order_number = str_field("order_number")?;
    let eig_day = str_field("eig_day")?;
    let condition = parsed
        .get("condition")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();
    let price = parsed
        .get("price")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();
    let qty = parsed
        .get("qty")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();
    let expire_day = parsed
        .get("expire_day")
        .and_then(|v| v.as_str())
        .unwrap_or("*")
        .to_string();
    // DEV_SECOND_PASSWORD フォールバックは debug ビルド（E2E テスト）専用
    let second_password = {
        let from_body = parsed
            .get("second_password")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        #[cfg(debug_assertions)]
        let pw = from_body.or_else(|| std::env::var("DEV_SECOND_PASSWORD").ok());
        #[cfg(not(debug_assertions))]
        let pw = from_body;
        pw.ok_or(RouteError::BadRequest)?
    };
    let req = exchange::adapter::tachibana::CorrectOrderRequest {
        order_number,
        eig_day,
        condition,
        price,
        qty,
        expire_day,
        second_password,
    };
    Ok(ApiCommand::TachibanaCorrectOrder { req: Box::new(req) })
}

/// `POST /api/tachibana/order/cancel` のボディをパースして ApiCommand を返す。
/// `second_password` は本文または `DEV_SECOND_PASSWORD` 環境変数から取得する。
fn parse_tachibana_cancel_order(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    let str_field = |key: &str| -> Result<String, RouteError> {
        parsed
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or(RouteError::BadRequest)
    };
    let order_number = str_field("order_number")?;
    let eig_day = str_field("eig_day")?;
    // DEV_SECOND_PASSWORD フォールバックは debug ビルド（E2E テスト）専用
    let second_password = {
        let from_body = parsed
            .get("second_password")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        #[cfg(debug_assertions)]
        let pw = from_body.or_else(|| std::env::var("DEV_SECOND_PASSWORD").ok());
        #[cfg(not(debug_assertions))]
        let pw = from_body;
        pw.ok_or(RouteError::BadRequest)?
    };
    let req = exchange::adapter::tachibana::CancelOrderRequest {
        order_number,
        eig_day,
        second_password,
    };
    Ok(ApiCommand::TachibanaOrderCancel { req: Box::new(req) })
}

/// `POST /api/sidebar/select-ticker` のボディをパースして ApiCommand を返す。
fn parse_sidebar_select_ticker(body: &str) -> Result<ApiCommand, RouteError> {
    let pane_id = body_uuid_field(body, "pane_id")?;
    let ticker = body_str_field(body, "ticker")?;
    let kind = body_opt_str_field(body, "kind")?;
    Ok(ApiCommand::Pane(PaneCommand::SidebarSelectTicker {
        pane_id,
        ticker,
        kind,
    }))
}

/// `POST /api/agent/narrative` のボディを `NarrativeCreateRequest` にパースする。
fn parse_narrative_create(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;

    let agent_id = parsed
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)?;
    if agent_id.trim().is_empty() {
        return Err(RouteError::BadRequest);
    }

    let ticker = parsed
        .get("ticker")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)?;
    let timeframe = parsed
        .get("timeframe")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)?;

    let observation_snapshot = parsed
        .get("observation_snapshot")
        .cloned()
        .ok_or(RouteError::BadRequest)?;
    if !observation_snapshot.is_object() && !observation_snapshot.is_array() {
        return Err(RouteError::BadRequest);
    }
    // 早期サイズチェック（圧縮前）。計画 §3.2 ハード上限 10 MB。
    let serialized =
        serde_json::to_vec(&observation_snapshot).map_err(|_| RouteError::BadRequest)?;
    if serialized.len() as u64 > crate::narrative::snapshot_store::MAX_UNCOMPRESSED_BYTES {
        return Err(RouteError::PayloadTooLarge);
    }

    let reasoning = parsed
        .get("reasoning")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)?;

    let action_value = parsed.get("action").ok_or(RouteError::BadRequest)?;
    let action: NarrativeAction =
        serde_json::from_value(action_value.clone()).map_err(|_| RouteError::BadRequest)?;

    let confidence = parsed
        .get("confidence")
        .and_then(|v| v.as_f64())
        .ok_or(RouteError::BadRequest)?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(RouteError::BadRequest);
    }

    let uagent_address = parsed
        .get("uagent_address")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let linked_order_id = parsed
        .get("linked_order_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timestamp_ms = parsed.get("timestamp_ms").and_then(|v| v.as_i64());
    let idempotency_key = parsed
        .get("idempotency_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let req = NarrativeCreateRequest {
        agent_id,
        uagent_address,
        ticker,
        timeframe,
        observation_snapshot,
        reasoning,
        action,
        confidence,
        linked_order_id,
        timestamp_ms,
        idempotency_key,
    };
    Ok(ApiCommand::Narrative(NarrativeCommand::Create(Box::new(
        req,
    ))))
}

fn parse_narrative_list(path: &str) -> Result<ApiCommand, RouteError> {
    let agent_id = query_param(path, "agent_id");
    let ticker = query_param(path, "ticker");
    let since_ms = query_param(path, "since_ms").and_then(|s| s.parse::<i64>().ok());
    let limit = query_param(path, "limit").and_then(|s| s.parse::<usize>().ok());
    Ok(ApiCommand::Narrative(NarrativeCommand::List(
        NarrativeListQuery {
            agent_id,
            ticker,
            since_ms,
            limit,
        },
    )))
}

fn parse_narrative_id_from_path(path: &str) -> Result<uuid::Uuid, RouteError> {
    let (path_part, _) = path.split_once('?').unwrap_or((path, ""));
    let tail = path_part
        .strip_prefix("/api/agent/narrative/")
        .filter(|s| !s.is_empty())
        .ok_or(RouteError::BadRequest)?;
    // /api/agent/narrative/{id}/snapshot などのサフィックスを落とす
    let id_str = tail.split('/').next().ok_or(RouteError::BadRequest)?;
    uuid::Uuid::parse_str(id_str).map_err(|_| RouteError::BadRequest)
}

fn parse_narrative_get(path: &str) -> Result<ApiCommand, RouteError> {
    let id = parse_narrative_id_from_path(path)?;
    Ok(ApiCommand::Narrative(NarrativeCommand::Get { id }))
}

fn parse_narrative_get_snapshot(path: &str) -> Result<ApiCommand, RouteError> {
    let id = parse_narrative_id_from_path(path)?;
    Ok(ApiCommand::Narrative(NarrativeCommand::GetSnapshot { id }))
}

fn parse_narrative_patch(path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    let id = parse_narrative_id_from_path(path)?;
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    let public = parsed
        .get("public")
        .and_then(|v| v.as_bool())
        .ok_or(RouteError::BadRequest)?;
    Ok(ApiCommand::Narrative(NarrativeCommand::Patch {
        id,
        public,
    }))
}

/// body から省略可能な文字列フィールドを取り出す。
/// フィールドが存在しない場合、または値が JSON `null` の場合は `None` を返す。
/// （必須フィールド用の `body_str_field` は null を 400 として拒否する。）
fn body_opt_str_field(body: &str, key: &str) -> Result<Option<String>, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    Ok(parsed
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

/// HTTP レスポンスを書き込む
async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status_code: u16,
    body: &str,
) -> std::io::Result<()> {
    let status_text = match status_code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        410 => "Gone",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "Unknown",
    };

    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Access-Control-Allow-Origin: *\r\n\
         \r\n\
         {body}",
        body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// デスクトップ全体のスクリーンショットを C:/tmp/screenshot.png に保存する。
/// spawn_blocking から呼ぶこと（sync API）。
fn capture_screenshot() -> String {
    const PATH: &str = "C:/tmp/screenshot.png";
    if let Err(e) = std::fs::create_dir_all("C:/tmp") {
        return format!(r#"{{"ok":false,"error":"mkdir failed: {e}"}}"#);
    }
    let screens = match screenshots::Screen::all() {
        Ok(s) => s,
        Err(e) => return format!(r#"{{"ok":false,"error":"screen enum: {e}"}}"#),
    };
    let Some(screen) = screens.into_iter().next() else {
        return r#"{"ok":false,"error":"no screen found"}"#.to_string();
    };
    match screen.capture() {
        Err(e) => format!(r#"{{"ok":false,"error":"capture: {e}"}}"#),
        Ok(image) => match image.save(PATH) {
            Ok(()) => format!(r#"{{"ok":true,"path":"{PATH}"}}"#),
            Err(e) => format!(r#"{{"ok":false,"error":"save: {e}"}}"#),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::ReplayCommand;

    // ── parse_request tests ──

    #[test]
    fn parse_request_valid_get() {
        let raw = "GET /api/replay/status HTTP/1.1\r\nHost: 127.0.0.1:9876\r\n\r\n";
        let (method, path, body) = parse_request(raw).unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/api/replay/status");
        assert!(body.is_empty());
    }

    #[test]
    fn parse_request_valid_post_with_body() {
        let raw = "POST /api/replay/play HTTP/1.1\r\nHost: 127.0.0.1:9876\r\nContent-Type: application/json\r\n\r\n{\"start\":\"2026-04-01 09:00\",\"end\":\"2026-04-01 15:00\"}";
        let (method, path, body) = parse_request(raw).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/api/replay/play");
        assert!(body.contains("start"));
        assert!(body.contains("end"));
    }

    #[test]
    fn parse_request_empty_string_returns_none() {
        assert!(parse_request("").is_none());
    }

    #[test]
    fn parse_request_malformed_returns_none() {
        // Only method, no path
        assert!(parse_request("GET\r\n\r\n").is_none());
    }

    #[test]
    fn parse_request_post_without_body() {
        let raw = "POST /api/replay/toggle HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (method, path, body) = parse_request(raw).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/api/replay/toggle");
        assert!(body.is_empty());
    }

    #[test]
    fn parse_request_no_double_crlf_body_empty() {
        // No \r\n\r\n separator → body should be empty
        let raw = "GET /api/replay/status HTTP/1.1\r\nHost: localhost";
        let result = parse_request(raw);
        assert!(result.is_some());
        let (_, _, body) = result.unwrap();
        assert!(body.is_empty());
    }

    // ── route tests: replay ──

    fn unwrap_replay(cmd: ApiCommand) -> ReplayCommand {
        match cmd {
            ApiCommand::Replay(c) => c,
            _ => panic!("Expected ApiCommand::Replay, got {cmd:?}"),
        }
    }

    fn unwrap_pane(cmd: ApiCommand) -> PaneCommand {
        match cmd {
            ApiCommand::Pane(c) => c,
            _ => panic!("Expected ApiCommand::Pane, got {cmd:?}"),
        }
    }

    // ── route tests: agent session (Phase 4b-1 サブフェーズ B) ──

    #[test]
    fn route_agent_session_step_accepts_default() {
        let cmd = route("POST", "/api/agent/session/default/step", "").unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::Step { session_id }) => {
                assert_eq!(session_id, "default");
            }
            other => panic!("Expected AgentSession::Step, got {other:?}"),
        }
    }

    #[test]
    fn route_agent_session_step_rejects_non_default_session_with_501() {
        // ADR-0001: session_id != "default" は NotImplemented（501）で拒否。
        let result = route("POST", "/api/agent/session/other/step", "");
        assert!(
            matches!(result, Err(RouteError::NotImplemented)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_agent_session_step_rejects_empty_session_id() {
        // `/api/agent/session//step` — 空セッション ID は BadRequest。
        let result = route("POST", "/api/agent/session//step", "");
        assert!(matches!(result, Err(RouteError::BadRequest)), "got {result:?}");
    }

    #[test]
    fn route_agent_session_step_rejects_get_method() {
        // step は POST のみ。GET では NotFound にフォールバック。
        let result = route("GET", "/api/agent/session/default/step", "");
        assert!(matches!(result, Err(RouteError::NotFound)), "got {result:?}");
    }

    #[test]
    fn route_agent_session_rejects_unknown_suffix() {
        // `/step` / `/advance` / `/order` 以外の suffix は NotFound。
        // サブフェーズ B 時点では `/step` のみルーティング済み。
        let result = route("POST", "/api/agent/session/default/unknown", "");
        assert!(matches!(result, Err(RouteError::NotFound)), "got {result:?}");
    }

    #[test]
    fn route_agent_session_step_rejects_non_default_uuid_like() {
        // UUID 風の session_id も拒否（Phase 4c まで "default" 固定）。
        let result = route(
            "POST",
            "/api/agent/session/550e8400-e29b-41d4-a716-446655440000/step",
            "",
        );
        assert!(matches!(result, Err(RouteError::NotImplemented)));
    }

    // ── route tests: agent session order (Phase 4b-1 サブフェーズ E) ──

    const VALID_ORDER_BODY: &str = r#"{
        "client_order_id": "cli_42",
        "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
        "side": "buy",
        "qty": 0.1,
        "order_type": {"market": {}}
    }"#;

    #[test]
    fn route_agent_session_order_accepts_default_with_valid_body() {
        let cmd = route(
            "POST",
            "/api/agent/session/default/order",
            VALID_ORDER_BODY,
        )
        .unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::PlaceOrder { session_id, request }) => {
                assert_eq!(session_id, "default");
                assert_eq!(request.client_order_id.as_str(), "cli_42");
                assert_eq!(request.ticker.symbol, "BTC");
            }
            other => panic!("expected PlaceOrder, got {other:?}"),
        }
    }

    #[test]
    fn route_agent_session_order_rejects_non_default_session() {
        let result = route("POST", "/api/agent/session/other/order", VALID_ORDER_BODY);
        assert!(matches!(result, Err(RouteError::NotImplemented)));
    }

    #[test]
    fn route_agent_session_order_rejects_string_ticker() {
        let body = r#"{
            "client_order_id": "cli_1",
            "ticker": "HyperliquidLinear:BTC",
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let result = route("POST", "/api/agent/session/default/order", body);
        assert!(matches!(result, Err(RouteError::BadRequest)), "got {result:?}");
    }

    #[test]
    fn route_agent_session_order_rejects_missing_order_type() {
        let body = r#"{
            "client_order_id": "cli_1",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1
        }"#;
        let result = route("POST", "/api/agent/session/default/order", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_agent_session_order_rejects_missing_client_order_id() {
        let body = r#"{
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let result = route("POST", "/api/agent/session/default/order", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_agent_session_order_rejects_invalid_client_order_id_charset() {
        let body = r#"{
            "client_order_id": "cli 42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        let result = route("POST", "/api/agent/session/default/order", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_agent_session_order_rejects_get_method() {
        let result = route("GET", "/api/agent/session/default/order", VALID_ORDER_BODY);
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn not_implemented_body_matches_adr_spec() {
        // ADR-0001 / phase4b_agent_replay_api.md §4.5 固定文言。
        assert_eq!(
            NOT_IMPLEMENTED_MULTI_SESSION_BODY,
            r#"{"error":"multi-session not yet implemented; use 'default' until Phase 4c"}"#
        );
    }

    #[test]
    fn route_get_status() {
        let cmd = route("GET", "/api/replay/status", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::GetStatus));
    }

    #[test]
    fn route_post_toggle() {
        let cmd = route("POST", "/api/replay/toggle", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::Toggle));
    }

    #[test]
    fn route_post_pause() {
        let cmd = route("POST", "/api/replay/pause", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::Pause));
    }

    #[test]
    fn route_post_resume() {
        let cmd = route("POST", "/api/replay/resume", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::Resume));
    }

    #[test]
    fn route_post_step_forward() {
        let cmd = route("POST", "/api/replay/step-forward", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::StepForward));
    }

    #[test]
    fn route_post_step_backward() {
        let cmd = route("POST", "/api/replay/step-backward", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::StepBackward));
    }

    #[test]
    fn route_post_speed() {
        let cmd = route("POST", "/api/replay/speed", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::CycleSpeed));
    }

    #[test]
    fn route_post_play_valid_json() {
        let body = r#"{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}"#;
        let cmd = route("POST", "/api/replay/play", body).unwrap();
        match unwrap_replay(cmd) {
            ReplayCommand::Play { start, end } => {
                assert_eq!(start, "2026-04-01 09:00");
                assert_eq!(end, "2026-04-01 15:00");
            }
            _ => panic!("Expected Play command"),
        }
    }

    #[test]
    fn route_post_play_invalid_json() {
        let result = route("POST", "/api/replay/play", "not json");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_play_missing_start() {
        let body = r#"{"end":"2026-04-01 15:00"}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_play_missing_end() {
        let body = r#"{"start":"2026-04-01 09:00"}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_play_empty_body() {
        let result = route("POST", "/api/replay/play", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_play_invalid_datetime_start_returns_bad_request() {
        let body = r#"{"start":"not-a-date","end":"2026-04-10 15:00"}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_play_invalid_datetime_end_returns_bad_request() {
        let body = r#"{"start":"2026-04-10 09:00","end":"bad-end"}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_unknown_path_not_found() {
        let result = route("GET", "/api/replay/unknown", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_get_on_post_endpoint_not_found() {
        // GET on POST-only endpoints should return NotFound
        let result = route("GET", "/api/replay/toggle", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_post_on_get_endpoint_not_found() {
        let result = route("POST", "/api/replay/status", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_root_path_not_found() {
        let result = route("GET", "/", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_post_play_non_string_values() {
        let body = r#"{"start":123,"end":456}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_app_save() {
        let cmd = route("POST", "/api/app/save", "").unwrap();
        assert!(matches!(unwrap_replay(cmd), ReplayCommand::SaveState));
    }

    #[test]
    fn route_post_app_set_mode_replay() {
        let cmd = route("POST", "/api/app/set-mode", r#"{"mode":"replay"}"#).unwrap();
        assert!(matches!(
            unwrap_replay(cmd),
            ReplayCommand::SetMode { mode } if mode == "replay"
        ));
    }

    #[test]
    fn route_post_app_set_mode_live() {
        let cmd = route("POST", "/api/app/set-mode", r#"{"mode":"live"}"#).unwrap();
        assert!(matches!(
            unwrap_replay(cmd),
            ReplayCommand::SetMode { mode } if mode == "live"
        ));
    }

    #[test]
    fn route_post_app_set_mode_case_insensitive() {
        let cmd = route("POST", "/api/app/set-mode", r#"{"mode":"REPLAY"}"#).unwrap();
        assert!(matches!(
            unwrap_replay(cmd),
            ReplayCommand::SetMode { mode } if mode == "replay"
        ));
    }

    #[test]
    fn route_post_app_set_mode_invalid() {
        let result = route("POST", "/api/app/set-mode", r#"{"mode":"unknown"}"#);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── route tests: narrative（Phase 4a）──

    fn unwrap_narrative(cmd: ApiCommand) -> NarrativeCommand {
        match cmd {
            ApiCommand::Narrative(c) => c,
            _ => panic!("Expected ApiCommand::Narrative, got {cmd:?}"),
        }
    }

    fn sample_narrative_body() -> String {
        serde_json::json!({
            "agent_id": "agent_alpha",
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "observation_snapshot": { "ohlcv": [[1, 2, 3, 4, 5]], "rsi": 28.3 },
            "reasoning": "divergence",
            "action": { "side": "buy", "qty": 0.1, "price": 92500.0 },
            "confidence": 0.76,
        })
        .to_string()
    }

    #[test]
    fn route_post_narrative_create() {
        let cmd = route("POST", "/api/agent/narrative", &sample_narrative_body()).unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::Create(req) => {
                assert_eq!(req.agent_id, "agent_alpha");
                assert_eq!(req.ticker, "BTCUSDT");
                assert_eq!(req.timeframe, "1h");
                assert!((req.confidence - 0.76).abs() < 1e-9);
                assert_eq!(req.action.qty, 0.1);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn route_post_narrative_create_rejects_empty_agent_id() {
        let body = serde_json::json!({
            "agent_id": "",
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "observation_snapshot": {},
            "reasoning": "x",
            "action": { "side": "buy", "qty": 1.0, "price": 1.0 },
            "confidence": 0.5,
        })
        .to_string();
        let result = route("POST", "/api/agent/narrative", &body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_narrative_create_rejects_out_of_range_confidence() {
        let body = serde_json::json!({
            "agent_id": "a",
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "observation_snapshot": {},
            "reasoning": "x",
            "action": { "side": "buy", "qty": 1.0, "price": 1.0 },
            "confidence": 1.5,
        })
        .to_string();
        let result = route("POST", "/api/agent/narrative", &body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_narrative_create_rejects_bad_side() {
        let body = serde_json::json!({
            "agent_id": "a",
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "observation_snapshot": {},
            "reasoning": "x",
            "action": { "side": "hold", "qty": 1.0, "price": 1.0 },
            "confidence": 0.5,
        })
        .to_string();
        let result = route("POST", "/api/agent/narrative", &body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_narrative_create_rejects_oversized_snapshot() {
        // 11MB of JSON string content → exceeds MAX_UNCOMPRESSED_BYTES (10MB)
        let big = "x".repeat(11 * 1024 * 1024);
        let body = serde_json::json!({
            "agent_id": "a",
            "ticker": "BTCUSDT",
            "timeframe": "1h",
            "observation_snapshot": { "blob": big },
            "reasoning": "x",
            "action": { "side": "buy", "qty": 1.0, "price": 1.0 },
            "confidence": 0.5,
        })
        .to_string();
        let result = route("POST", "/api/agent/narrative", &body);
        assert!(matches!(result, Err(RouteError::PayloadTooLarge)));
    }

    #[test]
    fn route_get_narratives_list_without_filters() {
        let cmd = route("GET", "/api/agent/narratives", "").unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::List(q) => {
                assert!(q.agent_id.is_none());
                assert!(q.ticker.is_none());
                assert!(q.since_ms.is_none());
                assert!(q.limit.is_none());
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn route_get_narratives_list_with_filters() {
        let path = "/api/agent/narratives?agent_id=alpha&ticker=BTCUSDT&since_ms=1000&limit=50";
        let cmd = route("GET", path, "").unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::List(q) => {
                assert_eq!(q.agent_id.as_deref(), Some("alpha"));
                assert_eq!(q.ticker.as_deref(), Some("BTCUSDT"));
                assert_eq!(q.since_ms, Some(1000));
                assert_eq!(q.limit, Some(50));
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn route_get_narrative_by_id() {
        let id = uuid::Uuid::new_v4();
        let path = format!("/api/agent/narrative/{id}");
        let cmd = route("GET", &path, "").unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::Get { id: got } => assert_eq!(got, id),
            _ => panic!("expected Get"),
        }
    }

    #[test]
    fn route_get_narrative_snapshot() {
        let id = uuid::Uuid::new_v4();
        let path = format!("/api/agent/narrative/{id}/snapshot");
        let cmd = route("GET", &path, "").unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::GetSnapshot { id: got } => assert_eq!(got, id),
            _ => panic!("expected GetSnapshot"),
        }
    }

    #[test]
    fn route_patch_narrative_public_true() {
        let id = uuid::Uuid::new_v4();
        let path = format!("/api/agent/narrative/{id}");
        let cmd = route("PATCH", &path, r#"{"public": true}"#).unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::Patch { id: got, public } => {
                assert_eq!(got, id);
                assert!(public);
            }
            _ => panic!("expected Patch"),
        }
    }

    #[test]
    fn route_patch_narrative_public_false_allowed() {
        let id = uuid::Uuid::new_v4();
        let path = format!("/api/agent/narrative/{id}");
        let cmd = route("PATCH", &path, r#"{"public": false}"#).unwrap();
        match unwrap_narrative(cmd) {
            NarrativeCommand::Patch { public, .. } => assert!(!public),
            _ => panic!("expected Patch"),
        }
    }

    #[test]
    fn route_patch_narrative_rejects_missing_public() {
        let id = uuid::Uuid::new_v4();
        let path = format!("/api/agent/narrative/{id}");
        let result = route("PATCH", &path, r#"{}"#);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_get_narratives_storage() {
        let cmd = route("GET", "/api/agent/narratives/storage", "").unwrap();
        assert!(matches!(
            unwrap_narrative(cmd),
            NarrativeCommand::StorageStats
        ));
    }

    #[test]
    fn route_get_narratives_orphans() {
        let cmd = route("GET", "/api/agent/narratives/orphans", "").unwrap();
        assert!(matches!(unwrap_narrative(cmd), NarrativeCommand::Orphans));
    }

    #[test]
    fn route_get_narrative_invalid_uuid_is_bad_request() {
        let result = route("GET", "/api/agent/narrative/not-a-uuid", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_app_set_mode_missing_field() {
        let result = route("POST", "/api/app/set-mode", r#"{}"#);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── route tests: pane ──

    #[test]
    fn route_get_pane_list() {
        let cmd = route("GET", "/api/pane/list", "").unwrap();
        assert!(matches!(unwrap_pane(cmd), PaneCommand::ListPanes));
    }

    #[test]
    fn route_post_pane_split_valid() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000001","axis":"Vertical"}"#;
        let cmd = route("POST", "/api/pane/split", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::Split { pane_id, axis } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()
                );
                assert_eq!(axis, "Vertical");
            }
            _ => panic!("Expected Split command"),
        }
    }

    #[test]
    fn route_post_pane_split_missing_axis() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000001"}"#;
        let result = route("POST", "/api/pane/split", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_pane_split_invalid_uuid() {
        let body = r#"{"pane_id":"not-a-uuid","axis":"Vertical"}"#;
        let result = route("POST", "/api/pane/split", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_pane_close_valid() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000002"}"#;
        let cmd = route("POST", "/api/pane/close", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::Close { pane_id } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()
                );
            }
            _ => panic!("Expected Close command"),
        }
    }

    #[test]
    fn route_post_pane_set_ticker_valid() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000003","ticker":"BinanceLinear:ETHUSDT"}"#;
        let cmd = route("POST", "/api/pane/set-ticker", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::SetTicker { pane_id, ticker } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()
                );
                assert_eq!(ticker, "BinanceLinear:ETHUSDT");
            }
            _ => panic!("Expected SetTicker command"),
        }
    }

    #[test]
    fn route_post_pane_set_timeframe_valid() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000004","timeframe":"M5"}"#;
        let cmd = route("POST", "/api/pane/set-timeframe", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::SetTimeframe { pane_id, timeframe } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap()
                );
                assert_eq!(timeframe, "M5");
            }
            _ => panic!("Expected SetTimeframe command"),
        }
    }

    #[test]
    fn route_post_pane_set_timeframe_missing_field() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000004"}"#;
        let result = route("POST", "/api/pane/set-timeframe", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_sidebar_select_ticker_without_kind() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000005","ticker":"BinanceLinear:BTCUSDT"}"#;
        let cmd = route("POST", "/api/sidebar/select-ticker", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::SidebarSelectTicker {
                pane_id,
                ticker,
                kind,
            } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap()
                );
                assert_eq!(ticker, "BinanceLinear:BTCUSDT");
                assert_eq!(kind, None);
            }
            _ => panic!("Expected SidebarSelectTicker command"),
        }
    }

    #[test]
    fn route_post_sidebar_select_ticker_with_kind() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000006","ticker":"BinanceLinear:ETHUSDT","kind":"HeatmapChart"}"#;
        let cmd = route("POST", "/api/sidebar/select-ticker", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::SidebarSelectTicker { kind, .. } => {
                assert_eq!(kind, Some("HeatmapChart".to_string()));
            }
            _ => panic!("Expected SidebarSelectTicker command"),
        }
    }

    #[test]
    fn route_get_notification_list() {
        let cmd = route("GET", "/api/notification/list", "").unwrap();
        assert!(matches!(unwrap_pane(cmd), PaneCommand::ListNotifications));
    }

    // ── route tests: chart-snapshot ──

    #[test]
    fn route_get_chart_snapshot_valid_uuid() {
        let path = "/api/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000010";
        let cmd = route("GET", path, "").unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::GetChartSnapshot {
                pane_id,
                limit,
                since_ts,
            } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()
                );
                assert_eq!(limit, None);
                assert_eq!(since_ts, None);
            }
            _ => panic!("Expected GetChartSnapshot command"),
        }
    }

    #[test]
    fn route_get_chart_snapshot_missing_pane_id_returns_bad_request() {
        let result = route("GET", "/api/pane/chart-snapshot", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_get_chart_snapshot_invalid_uuid_returns_bad_request() {
        let result = route("GET", "/api/pane/chart-snapshot?pane_id=not-a-uuid", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_chart_snapshot_not_found() {
        let result = route(
            "POST",
            "/api/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000010",
            "",
        );
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_get_chart_snapshot_with_limit_and_since_ts() {
        let path = "/api/pane/chart-snapshot?pane_id=00000000-0000-0000-0000-000000000010&limit=100&since_ts=1700000000000";
        let cmd = route("GET", path, "").unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::GetChartSnapshot {
                pane_id,
                limit,
                since_ts,
            } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()
                );
                assert_eq!(limit, Some(100));
                assert_eq!(since_ts, Some(1700000000000));
            }
            _ => panic!("Expected GetChartSnapshot command"),
        }
    }

    // ── query_param ──

    #[test]
    fn query_param_extracts_single_value() {
        assert_eq!(
            query_param("/api/pane/chart-snapshot?pane_id=abc-123", "pane_id"),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn query_param_extracts_from_multiple_params() {
        assert_eq!(
            query_param("/api/foo?a=1&pane_id=xyz&b=2", "pane_id"),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn query_param_returns_none_when_key_absent() {
        assert_eq!(query_param("/api/foo?other=val", "pane_id"), None);
    }

    #[test]
    fn query_param_returns_none_when_no_query() {
        assert_eq!(query_param("/api/foo", "pane_id"), None);
    }

    #[test]
    fn route_post_sidebar_select_ticker_missing_ticker() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000007"}"#;
        let result = route("POST", "/api/sidebar/select-ticker", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── route tests: open-order-pane ──

    #[test]
    fn route_post_sidebar_open_order_pane_order_entry() {
        let body = r#"{"kind":"OrderEntry"}"#;
        let cmd = route("POST", "/api/sidebar/open-order-pane", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::OpenOrderPane { kind } => assert_eq!(kind, "OrderEntry"),
            _ => panic!("Expected OpenOrderPane"),
        }
    }

    #[test]
    fn route_post_sidebar_open_order_pane_order_list() {
        let body = r#"{"kind":"OrderList"}"#;
        let cmd = route("POST", "/api/sidebar/open-order-pane", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::OpenOrderPane { kind } => assert_eq!(kind, "OrderList"),
            _ => panic!("Expected OpenOrderPane"),
        }
    }

    #[test]
    fn route_post_sidebar_open_order_pane_buying_power() {
        let body = r#"{"kind":"BuyingPower"}"#;
        let cmd = route("POST", "/api/sidebar/open-order-pane", body).unwrap();
        match unwrap_pane(cmd) {
            PaneCommand::OpenOrderPane { kind } => assert_eq!(kind, "BuyingPower"),
            _ => panic!("Expected OpenOrderPane"),
        }
    }

    #[test]
    fn route_post_sidebar_open_order_pane_invalid_kind() {
        let body = r#"{"kind":"InvalidKind"}"#;
        let result = route("POST", "/api/sidebar/open-order-pane", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[test]
    fn route_post_sidebar_open_order_pane_missing_kind() {
        let body = r#"{}"#;
        let result = route("POST", "/api/sidebar/open-order-pane", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── route tests: auth ──

    #[test]
    fn route_get_auth_tachibana_status() {
        let cmd = route("GET", "/api/auth/tachibana/status", "").unwrap();
        assert!(matches!(
            cmd,
            ApiCommand::Auth(AuthCommand::TachibanaSessionStatus)
        ));
    }

    #[test]
    fn route_post_auth_tachibana_status_not_found() {
        // POST はマッチしない（GET のみ）
        let result = route("POST", "/api/auth/tachibana/status", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    // ── inject-* エンドポイントは存在しないため 404 ──

    #[test]
    fn route_test_inject_endpoints_not_found() {
        let r1 = route("POST", "/api/test/tachibana/inject-session", "");
        let r2 = route("POST", "/api/test/tachibana/inject-master", "{}");
        let r3 = route("POST", "/api/test/tachibana/inject-daily-history", "{}");
        let r4 = route("POST", "/api/test/tachibana/inject-market-price", "{}");
        let r5 = route("POST", "/api/test/tachibana/persist-session", "");
        assert!(matches!(r1, Err(RouteError::NotFound)));
        assert!(matches!(r2, Err(RouteError::NotFound)));
        assert!(matches!(r3, Err(RouteError::NotFound)));
        assert!(matches!(r4, Err(RouteError::NotFound)));
        assert!(matches!(r5, Err(RouteError::NotFound)));
    }

    // ── route tests: GET /api/replay/orders ──

    #[test]
    fn route_get_replay_orders() {
        let cmd = route("GET", "/api/replay/orders", "").unwrap();
        assert!(matches!(
            cmd,
            ApiCommand::VirtualExchange(VirtualExchangeCommand::GetOrders)
        ));
    }

    #[test]
    fn route_post_replay_orders_not_found() {
        // POST はマッチしない（GET のみ）
        let result = route("POST", "/api/replay/orders", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    // ── body_opt_str_field tests ──

    #[test]
    fn opt_str_field_present() {
        let r = body_opt_str_field(r#"{"kind":"Candles"}"#, "kind").unwrap();
        assert_eq!(r, Some("Candles".to_string()));
    }

    #[test]
    fn opt_str_field_missing_key() {
        let r = body_opt_str_field(r#"{"ticker":"BTCUSDT"}"#, "kind").unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn opt_str_field_null_equals_omission() {
        // 仕様: JSON null はフィールド省略と等価 → None
        let r = body_opt_str_field(r#"{"kind":null}"#, "kind").unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn opt_str_field_invalid_json() {
        let r = body_opt_str_field("not json", "kind");
        assert!(matches!(r, Err(RouteError::BadRequest)));
    }

    // ── Phase 3: 注文管理 4 ルート RED テスト ──────────────────────────────────

    /// GET /api/tachibana/orders → FetchTachibanaOrders { eig_day: "" }
    #[test]
    fn route_get_tachibana_orders_no_param() {
        let cmd = route("GET", "/api/tachibana/orders", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrders { eig_day } => {
                assert_eq!(eig_day, "", "クエリなしは eig_day=空文字");
            }
            _ => panic!("Expected FetchTachibanaOrders, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/orders?eig_day=20260417 → eig_day が取れる
    #[test]
    fn route_get_tachibana_orders_with_eig_day() {
        let cmd = route("GET", "/api/tachibana/orders?eig_day=20260417", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrders { eig_day } => {
                assert_eq!(eig_day, "20260417");
            }
            _ => panic!("Expected FetchTachibanaOrders, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/order/12345678 → FetchTachibanaOrderDetail
    #[test]
    fn route_get_tachibana_order_detail() {
        let cmd = route("GET", "/api/tachibana/order/12345678", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day } => {
                assert_eq!(order_num, "12345678");
                assert_eq!(eig_day, "", "クエリなしは eig_day=空文字");
            }
            _ => panic!("Expected FetchTachibanaOrderDetail, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/order/12345678?eig_day=20260417 → eig_day クエリパラメータ取得
    #[test]
    fn route_get_tachibana_order_detail_with_eig_day() {
        let cmd = route("GET", "/api/tachibana/order/12345678?eig_day=20260417", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day } => {
                assert_eq!(order_num, "12345678");
                assert_eq!(eig_day, "20260417");
            }
            _ => panic!("Expected FetchTachibanaOrderDetail, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/order/ (order_num 空) → BadRequest
    #[test]
    fn route_get_tachibana_order_detail_empty_order_num_bad_request() {
        let result = route("GET", "/api/tachibana/order/", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/correct → TachibanaCorrectOrder
    #[test]
    fn route_post_tachibana_order_correct_valid() {
        let body = r#"{
            "order_number": "12345678",
            "eig_day": "20260417",
            "condition": "*",
            "price": "2600",
            "qty": "*",
            "expire_day": "*",
            "second_password": "testpw"
        }"#;
        let cmd = route("POST", "/api/tachibana/order/correct", body).unwrap();
        match cmd {
            ApiCommand::TachibanaCorrectOrder { req } => {
                assert_eq!(req.order_number, "12345678");
                assert_eq!(req.price, "2600");
                assert_eq!(req.qty, "*");
            }
            _ => panic!("Expected TachibanaCorrectOrder, got {cmd:?}"),
        }
    }

    /// POST /api/tachibana/order/correct: 必須フィールド欠落 → BadRequest
    #[test]
    fn route_post_tachibana_order_correct_missing_field() {
        let body = r#"{"price":"2600"}"#;
        let result = route("POST", "/api/tachibana/order/correct", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/cancel → TachibanaOrderCancel
    #[test]
    fn route_post_tachibana_order_cancel_valid() {
        let body = r#"{
            "order_number": "87654321",
            "eig_day": "20260417",
            "second_password": "testpw"
        }"#;
        let cmd = route("POST", "/api/tachibana/order/cancel", body).unwrap();
        match cmd {
            ApiCommand::TachibanaOrderCancel { req } => {
                assert_eq!(req.order_number, "87654321");
                assert_eq!(req.eig_day, "20260417");
            }
            _ => panic!("Expected TachibanaOrderCancel, got {cmd:?}"),
        }
    }

    /// POST /api/tachibana/order/cancel: 必須フィールド欠落 → BadRequest
    #[test]
    fn route_post_tachibana_order_cancel_missing_order_number() {
        let body = r#"{"eig_day":"20260417"}"#;
        let result = route("POST", "/api/tachibana/order/cancel", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/cancel: 不正 JSON → BadRequest
    #[test]
    fn route_post_tachibana_order_cancel_invalid_json() {
        let result = route("POST", "/api/tachibana/order/cancel", "not json");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// GET /api/tachibana/holdings?issue_code=7203 → FetchTachibanaHoldings
    #[test]
    fn route_get_tachibana_holdings_with_issue_code() {
        let cmd = route("GET", "/api/tachibana/holdings?issue_code=7203", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaHoldings { issue_code } => {
                assert_eq!(issue_code, "7203");
            }
            _ => panic!("Expected FetchTachibanaHoldings, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/holdings (issue_code なし) → BadRequest
    #[test]
    fn route_get_tachibana_holdings_missing_issue_code_bad_request() {
        let result = route("GET", "/api/tachibana/holdings", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }
}
