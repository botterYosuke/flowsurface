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
        side: String,   // "buy" | "sell"
        qty: f64,
        order_type: String,  // "market" | "limit"
        limit_price: Option<f64>,
    },
    /// ポートフォリオスナップショットを取得する（GET /api/replay/portfolio）
    GetPortfolio,
    /// 観測データを取得する（GET /api/replay/state）— Phase 1 骨格のみ
    GetState,
}

/// 認証状態確認コマンド。
#[derive(Debug, Clone)]
pub enum AuthCommand {
    /// 現在の立花証券セッション有無を返す（`{"session":"present"|"none"}`）。
    TachibanaSessionStatus,
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
    /// 指定ペインのチャートスナップショット（バー数・タイムスタンプ範囲）を返す。
    /// クエリパラメータ: `?pane_id=<uuid>`
    GetChartSnapshot { pane_id: uuid::Uuid },
}

/// oneshot::Sender を Clone 可能にするラッパー（iced の Message は Clone が必要）
/// レスポンスは main.rs 側でシリアライズ済み JSON を送る。
#[derive(Debug, Clone)]
pub struct ReplySender(Arc<Mutex<Option<oneshot::Sender<String>>>>);

impl ReplySender {
    fn new(tx: oneshot::Sender<String>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// 応答を送信する。2回目以降の呼び出しは何もしない。
    pub fn send(self, body: String) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(body);
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

/// ポート番号を環境変数または デフォルト 9876 から取得
fn api_port() -> u16 {
    std::env::var("FLOWSURFACE_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9876)
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
        // 512KB バッファ: inject-daily-history で多数 kline を送る場合に備えて確保
        let mut buf = vec![0u8; 524288];
        let n = match stream.read(&mut buf).await {
            Ok(0) => continue,
            Ok(n) => n,
            Err(_) => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);
        let (method, path, body) = match parse_request(&request) {
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
        };

        // oneshot で iced app からのレスポンスを待つ
        let (reply_tx, reply_rx) = oneshot::channel();
        if sender
            .send((command, ReplySender::new(reply_tx)))
            .await
            .is_err()
        {
            let _ = write_response(&mut stream, 500, r#"{"error":"App channel closed"}"#).await;
            continue;
        }

        match reply_rx.await {
            Ok(json) => {
                let _ = write_response(&mut stream, 200, &json).await;
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
}

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

        // ── 認証（本番ビルドにも含まれる）────────────────────────────────
        ("GET", "/api/auth/tachibana/status") => {
            Ok(ApiCommand::Auth(AuthCommand::TachibanaSessionStatus))
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

        // ── 仮想約定エンジン（Phase 2 互換）──────────────────────────────
        ("POST", "/api/replay/order") => parse_virtual_order_command(body),
        ("GET", "/api/replay/portfolio") => {
            Ok(ApiCommand::VirtualExchange(VirtualExchangeCommand::GetPortfolio))
        }
        ("GET", "/api/replay/state") => {
            Ok(ApiCommand::VirtualExchange(VirtualExchangeCommand::GetState))
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

    Ok(ApiCommand::VirtualExchange(VirtualExchangeCommand::PlaceOrder {
        ticker,
        side,
        qty,
        order_type,
        limit_price,
    }))
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

/// `GET /api/pane/chart-snapshot?pane_id=<uuid>` をパースして ApiCommand を返す。
fn parse_chart_snapshot_command(path: &str) -> Result<ApiCommand, RouteError> {
    let id_str = query_param(path, "pane_id").ok_or(RouteError::BadRequest)?;
    let pane_id = uuid::Uuid::parse_str(&id_str).map_err(|_| RouteError::BadRequest)?;
    Ok(ApiCommand::Pane(PaneCommand::GetChartSnapshot { pane_id }))
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

/// body から省略可能な文字列フィールドを取り出す（存在しなければ None）。
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
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
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
            PaneCommand::GetChartSnapshot { pane_id } => {
                assert_eq!(
                    pane_id,
                    uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()
                );
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
}
