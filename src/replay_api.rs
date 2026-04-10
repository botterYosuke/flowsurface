use crate::replay::{ReplayCommand, ReplayStatus};
use futures::channel::mpsc;
use futures::SinkExt;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// oneshot::Sender を Clone 可能にするラッパー（iced の Message は Clone が必要）
#[derive(Debug, Clone)]
pub struct ReplySender(Arc<Mutex<Option<oneshot::Sender<ReplayStatus>>>>);

impl ReplySender {
    fn new(tx: oneshot::Sender<ReplayStatus>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// 応答を送信する。2回目以降の呼び出しは何もしない。
    pub fn send(self, status: ReplayStatus) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(status);
            }
        }
    }
}

/// API サーバーから iced に送るメッセージ（コマンド + 応答用チャネル）
pub type ApiMessage = (ReplayCommand, ReplySender);

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

        let command = match route(&method, &path, &body) {
            Ok(cmd) => cmd,
            Err(RouteError::NotFound) => {
                let _ = write_response(&mut stream, 404, r#"{"error":"Not Found"}"#).await;
                continue;
            }
            Err(RouteError::BadRequest) => {
                let _ = write_response(&mut stream, 400, r#"{"error":"Bad Request: invalid JSON body"}"#).await;
                continue;
            }
        };

        // oneshot で iced app からのレスポンスを待つ
        let (reply_tx, reply_rx) = oneshot::channel();
        if sender.send((command, ReplySender::new(reply_tx))).await.is_err() {
            let _ = write_response(&mut stream, 500, r#"{"error":"App channel closed"}"#).await;
            continue;
        }

        match reply_rx.await {
            Ok(status) => {
                let json = serde_json::to_string(&status).unwrap_or_default();
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

enum RouteError {
    NotFound,
    BadRequest,
}

/// パスとメソッドから ReplayCommand にルーティング
fn route(method: &str, path: &str, body: &str) -> Result<ReplayCommand, RouteError> {
    match (method, path) {
        ("GET", "/api/replay/status") => Ok(ReplayCommand::GetStatus),
        ("POST", "/api/replay/toggle") => Ok(ReplayCommand::Toggle),
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
            Ok(ReplayCommand::Play { start, end })
        }
        ("POST", "/api/replay/pause") => Ok(ReplayCommand::Pause),
        ("POST", "/api/replay/resume") => Ok(ReplayCommand::Resume),
        ("POST", "/api/replay/step-forward") => Ok(ReplayCommand::StepForward),
        ("POST", "/api/replay/step-backward") => Ok(ReplayCommand::StepBackward),
        ("POST", "/api/replay/speed") => Ok(ReplayCommand::CycleSpeed),
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
