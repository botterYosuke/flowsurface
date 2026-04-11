use crate::replay::{ReplayCommand, ReplayStatus};
use futures::SinkExt;
use futures::channel::mpsc;
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
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(status);
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

#[derive(Debug)]
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
        ("POST", "/api/app/save") => Ok(ReplayCommand::SaveState),
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

    // ── route tests ──

    #[test]
    fn route_get_status() {
        let cmd = route("GET", "/api/replay/status", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::GetStatus));
    }

    #[test]
    fn route_post_toggle() {
        let cmd = route("POST", "/api/replay/toggle", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::Toggle));
    }

    #[test]
    fn route_post_pause() {
        let cmd = route("POST", "/api/replay/pause", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::Pause));
    }

    #[test]
    fn route_post_resume() {
        let cmd = route("POST", "/api/replay/resume", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::Resume));
    }

    #[test]
    fn route_post_step_forward() {
        let cmd = route("POST", "/api/replay/step-forward", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::StepForward));
    }

    #[test]
    fn route_post_step_backward() {
        let cmd = route("POST", "/api/replay/step-backward", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::StepBackward));
    }

    #[test]
    fn route_post_speed() {
        let cmd = route("POST", "/api/replay/speed", "").unwrap();
        assert!(matches!(cmd, ReplayCommand::CycleSpeed));
    }

    #[test]
    fn route_post_play_valid_json() {
        let body = r#"{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}"#;
        let cmd = route("POST", "/api/replay/play", body).unwrap();
        match cmd {
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
        assert!(matches!(cmd, ReplayCommand::SaveState));
    }
}
