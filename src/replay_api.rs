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
    /// E2E テスト用の fixture 注入コマンド（`e2e-mock` feature でのみ有効）。
    #[cfg(feature = "e2e-mock")]
    Test(TestCommand),
}

/// 認証状態確認コマンド。
#[derive(Debug, Clone)]
pub enum AuthCommand {
    /// 現在の立花証券セッション有無を返す（`{"session":"present"|"none"}`）。
    TachibanaSessionStatus,
}

/// E2E テスト fixture 注入コマンド。
/// 認証・MASTER I/F・日足取得をバイパスし、mock データで Tachibana D1 リプレイを検証する。
/// 詳細: docs/plan/tachibana_e2e_phase_t1.md
#[cfg(feature = "e2e-mock")]
#[derive(Debug, Clone)]
pub enum TestCommand {
    /// ダミー `TachibanaSession` をメモリに格納する（keyring 非経由）
    TachibanaInjectSession,
    /// `ISSUE_MASTER_CACHE` に MasterRecord を直接注入する。
    /// body は `/api/test/tachibana/inject-master` で受け取った JSON 文字列そのまま。
    TachibanaInjectMaster { raw_body: String },
    /// `MOCK_DAILY_HISTORY` に issue_code → Vec<Kline> を登録する。
    TachibanaInjectDailyHistory { raw_body: String },
    // ── Phase T2 ──────────────────────────────────────────────────────────
    /// `MOCK_MARKET_PRICES` に MarketPriceRecord を登録する（Phase T2）。
    /// 以降の `fetch_market_prices` 呼び出しがネットワークを叩かず mock を返す。
    TachibanaInjectMarketPrice { raw_body: String },
    // ── Phase T3 ──────────────────────────────────────────────────────────
    /// ダミーセッションをメモリ AND keyring 両方に保存する（Phase T3 keyring テスト用）。
    TachibanaInjectPersistSession,
    /// メモリセッション + keyring セッションを両方クリアする（Phase T3 keyring テスト用）。
    TachibanaDeletePersistedSession,
}

/// ペイン CRUD 系コマンド（§6.2 #2/#5/#6/#7/#8 テスト用）。
#[derive(Debug, Clone)]
pub enum PaneCommand {
    /// 全ペインのメタ情報 + リプレイバッファ状態を返す
    ListPanes,
    /// ペインを分割する。axis: "Vertical" | "Horizontal"
    /// new_content は無視（既存 pane::Message::SplitPane は Starter しか生成しない）
    Split {
        pane_id: uuid::Uuid,
        axis: String,
    },
    /// ペインを閉じる
    Close { pane_id: uuid::Uuid },
    /// ペインのストリームを別 ticker に差し替える（SerTicker 形式 "BinanceLinear:BTCUSDT"）
    SetTicker {
        pane_id: uuid::Uuid,
        ticker: String,
    },
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
        let mut buf = vec![0u8; 8192];
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

/// パスとメソッドから ApiCommand にルーティング
fn route(method: &str, path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    // replay / app 系は ReplayCommand にラップ
    match (method, path) {
        ("GET", "/api/replay/status") => return Ok(ApiCommand::Replay(ReplayCommand::GetStatus)),
        ("POST", "/api/replay/toggle") => return Ok(ApiCommand::Replay(ReplayCommand::Toggle)),
        ("POST", "/api/replay/play") => {
            let parsed: serde_json::Value =
                serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
            let start = parsed
                .get("start")
                .and_then(|v| v.as_str())
                .ok_or(RouteError::BadRequest)?
                .to_string();
            let end = parsed
                .get("end")
                .and_then(|v| v.as_str())
                .ok_or(RouteError::BadRequest)?
                .to_string();
            return Ok(ApiCommand::Replay(ReplayCommand::Play { start, end }));
        }
        ("POST", "/api/replay/pause") => return Ok(ApiCommand::Replay(ReplayCommand::Pause)),
        ("POST", "/api/replay/resume") => return Ok(ApiCommand::Replay(ReplayCommand::Resume)),
        ("POST", "/api/replay/step-forward") => {
            return Ok(ApiCommand::Replay(ReplayCommand::StepForward));
        }
        ("POST", "/api/replay/step-backward") => {
            return Ok(ApiCommand::Replay(ReplayCommand::StepBackward));
        }
        ("POST", "/api/replay/speed") => {
            return Ok(ApiCommand::Replay(ReplayCommand::CycleSpeed));
        }
        ("POST", "/api/app/save") => return Ok(ApiCommand::Replay(ReplayCommand::SaveState)),
        // auth 系（本番ビルドにも含まれる）
        ("GET", "/api/auth/tachibana/status") => {
            return Ok(ApiCommand::Auth(AuthCommand::TachibanaSessionStatus));
        }
        _ => {}
    }

    // pane 系
    match (method, path) {
        ("GET", "/api/pane/list") => Ok(ApiCommand::Pane(PaneCommand::ListPanes)),
        ("POST", "/api/pane/split") => {
            let pane_id = body_uuid_field(body, "pane_id")?;
            let axis = body_str_field(body, "axis")?;
            Ok(ApiCommand::Pane(PaneCommand::Split { pane_id, axis }))
        }
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
        ("GET", "/api/notification/list") => {
            Ok(ApiCommand::Pane(PaneCommand::ListNotifications))
        }
        ("POST", "/api/sidebar/select-ticker") => {
            let pane_id = body_uuid_field(body, "pane_id")?;
            let ticker = body_str_field(body, "ticker")?;
            // kind は optional
            let parsed: serde_json::Value =
                serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
            let kind = parsed
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(ApiCommand::Pane(PaneCommand::SidebarSelectTicker {
                pane_id,
                ticker,
                kind,
            }))
        }
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/inject-session") => {
            Ok(ApiCommand::Test(TestCommand::TachibanaInjectSession))
        }
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/inject-master") => {
            // body のフィールド存在だけ先に軽く検証
            let _: serde_json::Value =
                serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
            Ok(ApiCommand::Test(TestCommand::TachibanaInjectMaster {
                raw_body: body.to_string(),
            }))
        }
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/inject-daily-history") => {
            let _: serde_json::Value =
                serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
            Ok(ApiCommand::Test(TestCommand::TachibanaInjectDailyHistory {
                raw_body: body.to_string(),
            }))
        }
        // ── Phase T2: inject-market-price ──────────────────────────────────
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/inject-market-price") => {
            let _: serde_json::Value =
                serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
            Ok(ApiCommand::Test(TestCommand::TachibanaInjectMarketPrice {
                raw_body: body.to_string(),
            }))
        }
        // ── Phase T3: keyring 永続化テスト ─────────────────────────────────
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/persist-session") => {
            Ok(ApiCommand::Test(TestCommand::TachibanaInjectPersistSession))
        }
        #[cfg(feature = "e2e-mock")]
        ("POST", "/api/test/tachibana/delete-persisted-session") => {
            Ok(ApiCommand::Test(TestCommand::TachibanaDeletePersistedSession))
        }
        _ => Err(RouteError::NotFound),
    }
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
        let body =
            r#"{"pane_id":"00000000-0000-0000-0000-000000000004","timeframe":"M5"}"#;
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

    #[test]
    fn route_post_sidebar_select_ticker_missing_ticker() {
        let body = r#"{"pane_id":"00000000-0000-0000-0000-000000000007"}"#;
        let result = route("POST", "/api/sidebar/select-ticker", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── route tests: test backdoor (e2e-mock only) ──

    #[cfg(feature = "e2e-mock")]
    fn unwrap_test(cmd: ApiCommand) -> TestCommand {
        match cmd {
            ApiCommand::Test(c) => c,
            _ => panic!("Expected ApiCommand::Test, got {cmd:?}"),
        }
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_session() {
        let cmd = route("POST", "/api/test/tachibana/inject-session", "").unwrap();
        assert!(matches!(
            unwrap_test(cmd),
            TestCommand::TachibanaInjectSession
        ));
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_master_valid() {
        let body = r#"{"records":[{"sIssueCode":"7203","sIssueName":"トヨタ","sIssueNameEizi":"TOYOTA"}]}"#;
        let cmd = route("POST", "/api/test/tachibana/inject-master", body).unwrap();
        match unwrap_test(cmd) {
            TestCommand::TachibanaInjectMaster { raw_body } => {
                assert!(raw_body.contains("7203"));
            }
            _ => panic!("Expected TachibanaInjectMaster"),
        }
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_master_invalid_json() {
        let result = route("POST", "/api/test/tachibana/inject-master", "not json");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_daily_history_valid() {
        let body = r#"{"issue_code":"7203","klines":[{"time":1700000000000,"open":100.0,"high":110.0,"low":90.0,"close":105.0,"volume":1000.0}]}"#;
        let cmd = route("POST", "/api/test/tachibana/inject-daily-history", body).unwrap();
        match unwrap_test(cmd) {
            TestCommand::TachibanaInjectDailyHistory { raw_body } => {
                assert!(raw_body.contains("7203"));
                assert!(raw_body.contains("klines"));
            }
            _ => panic!("Expected TachibanaInjectDailyHistory"),
        }
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_daily_history_invalid_json() {
        let result = route("POST", "/api/test/tachibana/inject-daily-history", "xx");
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

    // ── Phase T2: inject-market-price ──

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_market_price_valid() {
        let body = r#"{"records":[{"sIssueCode":"7203","pDPP":"3000.0","pDOP":"2990.0","pDHP":"3010.0","pDLP":"2985.0","pDV":"500000.0","pPRP":"2950.0"}]}"#;
        let cmd = route("POST", "/api/test/tachibana/inject-market-price", body).unwrap();
        match unwrap_test(cmd) {
            TestCommand::TachibanaInjectMarketPrice { raw_body } => {
                assert!(raw_body.contains("7203"));
                assert!(raw_body.contains("records"));
            }
            _ => panic!("Expected TachibanaInjectMarketPrice"),
        }
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_inject_market_price_invalid_json() {
        let result = route("POST", "/api/test/tachibana/inject-market-price", "not json");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // ── Phase T3: keyring 永続化テスト ──

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_persist_session() {
        let cmd = route("POST", "/api/test/tachibana/persist-session", "").unwrap();
        assert!(matches!(
            unwrap_test(cmd),
            TestCommand::TachibanaInjectPersistSession
        ));
    }

    #[cfg(feature = "e2e-mock")]
    #[test]
    fn route_post_tachibana_delete_persisted_session() {
        let cmd =
            route("POST", "/api/test/tachibana/delete-persisted-session", "").unwrap();
        assert!(matches!(
            unwrap_test(cmd),
            TestCommand::TachibanaDeletePersistedSession
        ));
    }

    // backdoor エンドポイントは feature OFF 時は 404 であるべき
    #[cfg(not(feature = "e2e-mock"))]
    #[test]
    fn route_test_backdoor_disabled_when_feature_off() {
        let r1 = route("POST", "/api/test/tachibana/inject-session", "");
        let r2 = route("POST", "/api/test/tachibana/inject-master", "{}");
        let r3 = route("POST", "/api/test/tachibana/inject-daily-history", "{}");
        let r4 = route("POST", "/api/test/tachibana/inject-market-price", "{}");
        let r5 = route("POST", "/api/test/tachibana/persist-session", "");
        let r6 = route("POST", "/api/test/tachibana/delete-persisted-session", "");
        assert!(matches!(r1, Err(RouteError::NotFound)));
        assert!(matches!(r2, Err(RouteError::NotFound)));
        assert!(matches!(r3, Err(RouteError::NotFound)));
        assert!(matches!(r4, Err(RouteError::NotFound)));
        assert!(matches!(r5, Err(RouteError::NotFound)));
        assert!(matches!(r6, Err(RouteError::NotFound)));
    }
}
