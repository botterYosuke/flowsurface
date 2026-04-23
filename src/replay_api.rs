use crate::narrative::model::NarrativeAction;
use crate::replay::ReplayCommand;
use futures::SinkExt;
use futures::channel::mpsc;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Union of commands exposed through the local replay API server.
#[derive(Debug, Clone)]
pub enum ApiCommand {
    Replay(ReplayCommand),
    Pane(PaneCommand),
    Auth(AuthCommand),
    VirtualExchange(VirtualExchangeCommand),
    Narrative(NarrativeCommand),
    AgentSession(AgentSessionCommand),
    FetchBuyingPower,
    TachibanaNewOrder {
        req: Box<exchange::adapter::tachibana::NewOrderRequest>,
    },
    FetchTachibanaOrders {
        eig_day: String,
    },
    FetchTachibanaOrderDetail {
        order_num: String,
        eig_day: String,
    },
    TachibanaCorrectOrder {
        req: Box<exchange::adapter::tachibana::CorrectOrderRequest>,
    },
    TachibanaOrderCancel {
        req: Box<exchange::adapter::tachibana::CancelOrderRequest>,
    },
    FetchTachibanaHoldings {
        issue_code: String,
    },
    #[cfg(debug_assertions)]
    Test(TestCommand),
}

#[derive(Debug, Clone)]
pub enum VirtualExchangeCommand {
    PlaceOrder {
        ticker: String,
        side: String,
        qty: f64,
        order_type: String,
        limit_price: Option<f64>,
    },
    GetPortfolio,
    GetState,
    GetOrders,
}

#[derive(Debug, Clone)]
pub enum AgentSessionCommand {
    Step {
        session_id: String,
    },
    PlaceOrder {
        session_id: String,
        request: Box<crate::api::order_request::AgentOrderRequest>,
    },
    Advance {
        session_id: String,
        request: Box<crate::api::advance_request::AgentAdvanceRequest>,
    },
    RewindToStart {
        session_id: String,
        init_range: Option<(String, String)>,
    },
}

#[derive(Debug, Clone)]
pub enum NarrativeCommand {
    Create(Box<NarrativeCreateRequest>),
    List(NarrativeListQuery),
    Get { id: uuid::Uuid },
    GetSnapshot { id: uuid::Uuid },
    Patch { id: uuid::Uuid, public: bool },
    StorageStats,
    Orphans,
}

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
    pub timestamp_ms: Option<i64>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NarrativeListQuery {
    pub agent_id: Option<String>,
    pub ticker: Option<String>,
    pub since_ms: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum AuthCommand {
    TachibanaSessionStatus,
    TachibanaLogout,
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone)]
pub enum TestCommand {
    TachibanaDeletePersistedSession,
}

#[derive(Debug, Clone)]
pub enum PaneCommand {
    ListPanes,
    Split {
        pane_id: uuid::Uuid,
        axis: String,
    },
    Close {
        pane_id: uuid::Uuid,
    },
    SetTicker {
        pane_id: uuid::Uuid,
        ticker: String,
    },
    SetTimeframe {
        pane_id: uuid::Uuid,
        timeframe: String,
    },
    SidebarSelectTicker {
        pane_id: uuid::Uuid,
        ticker: String,
        kind: Option<String>,
    },
    ListNotifications,
    GetChartSnapshot {
        pane_id: uuid::Uuid,
        limit: Option<usize>,
        since_ts: Option<u64>,
    },
    OpenOrderPane {
        kind: String,
    },
}

type ReplySenderInner = Arc<Mutex<Option<oneshot::Sender<(u16, String)>>>>;

/// oneshot::Sender 繧・Clone 蜿ｯ閭ｽ縺ｫ縺吶ｋ繝ｩ繝・ヱ繝ｼ・・ced 縺ｮ Message 縺ｯ Clone 縺悟ｿ・ｦ・ｼ・/// 繝ｬ繧ｹ繝昴Φ繧ｹ縺ｯ main.rs 蛛ｴ縺ｧ繧ｷ繝ｪ繧｢繝ｩ繧､繧ｺ貂医∩ JSON 繧帝√ｋ縲・/// 繧ｿ繝励Ν (status_code, body) 縺ｧ繧ｹ繝・・繧ｿ繧ｹ繧ｳ繝ｼ繝峨ｒ謖・ｮ壹〒縺阪ｋ縲・
#[derive(Debug, Clone)]
pub struct ReplySender(ReplySenderInner);

impl ReplySender {
    fn new(tx: oneshot::Sender<(u16, String)>) -> Self {
        Self(Arc::new(Mutex::new(Some(tx))))
    }

    /// HTTP 200 縺ｧ繝ｬ繧ｹ繝昴Φ繧ｹ繧帝∽ｿ｡縺吶ｋ縲・蝗樒岼莉･髯阪・蜻ｼ縺ｳ蜃ｺ縺励・菴輔ｂ縺励↑縺・・    
    pub fn send(self, body: String) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send((200, body));
        }
    }

    /// 莉ｻ諢上・繧ｹ繝・・繧ｿ繧ｹ繧ｳ繝ｼ繝峨〒繝ｬ繧ｹ繝昴Φ繧ｹ繧帝∽ｿ｡縺吶ｋ縲・蝗樒岼莉･髯阪・蜻ｼ縺ｳ蜃ｺ縺励・菴輔ｂ縺励↑縺・・    
    pub fn send_status(self, status: u16, body: String) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send((status, body));
        }
    }
}

/// API 繧ｵ繝ｼ繝舌・縺九ｉ iced 縺ｫ騾√ｋ繝｡繝・そ繝ｼ繧ｸ・医さ繝槭Φ繝・+ 蠢懃ｭ皮畑繝√Ε繝阪Ν・・
pub type ApiMessage = (ApiCommand, ReplySender);

/// channel() 繝代ち繝ｼ繝ｳ縺ｧ API 繧ｵ繝ｼ繝舌・繧定ｵｷ蜍輔＠縲｀essage 繧ｹ繝医Μ繝ｼ繝繧定ｿ斐☆縲・/// exchange/src/connect.rs:111-122 縺ｮ蜀榊茜逕ｨ繝代ち繝ｼ繝ｳ縲・
pub fn subscription() -> impl futures::Stream<Item = ApiMessage> {
    exchange::connect::channel(32, |sender| async move {
        run_server(sender).await;
    })
}

/// headless 繝｢繝ｼ繝牙髄縺・ 螟夜Κ縺九ｉ sender 繧呈ｸ｡縺励※ HTTP 繧ｵ繝ｼ繝舌・繧定ｵｷ蜍輔☆繧九・
pub async fn start_server(sender: futures::channel::mpsc::Sender<ApiMessage>) {
    run_server(sender).await;
}

/// 繝昴・繝育分蜿ｷ繧堤腸蠅・､画焚縺ｾ縺溘・ 繝・ヵ繧ｩ繝ｫ繝・9876 縺九ｉ蜿門ｾ・
fn api_port() -> u16 {
    std::env::var("FLOWSURFACE_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9876)
}

/// Content-Length 繝倥ャ繝繝ｼ縺ｮ蛟､繧偵ヱ繝ｼ繧ｹ縺吶ｋ・郁ｦ九▽縺九ｉ縺ｪ縺代ｌ縺ｰ 0・峨・
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

/// 荳企剞: 繝倥ャ繝繝ｼ 64KB + 繝懊ョ繧｣ 16MB・・hase 4a 縺ｮ繧ｹ繝翫ャ繝励す繝ｧ繝・ヨ 10MB ・倶ｽ呵｣輔ｒ隕九◆蛟､・峨・
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

pub(crate) enum ReadRequestOutcome {
    /// 繝ｪ繧ｯ繧ｨ繧ｹ繝医ｒ蜈ｨ驥丞女菫｡縺励◆・医・繝・ム繝ｼ + 繝懊ョ繧｣・・    
    Ok(String),
    /// Request body exceeded the accepted size.
    TooLarge,
    /// Invalid request bytes or malformed HTTP framing.
    Invalid,
}

/// HTTP 繝ｪ繧ｯ繧ｨ繧ｹ繝医ｒ螳悟・縺ｫ隱ｭ縺ｿ霎ｼ繧・・ontent-Length 縺ｫ蠕薙▲縺ｦ繝懊ョ繧｣繧ら｢ｺ菫晢ｼ峨・/// TCP 縺悟・蜑ｲ縺励※螻翫＞縺ｦ繧よｭ｣縺励￥邨仙粋縺励√・繝・ぅ繧ｵ繧､繧ｺ縺ｫ蠢懊§縺ｦ繝舌ャ繝輔ぃ繧貞虚逧・僑蠑ｵ縺吶ｋ縲・
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
            // bind 螟ｱ謨励ｒ silent 縺ｫ縺吶ｋ縺ｨ縲・2E 繝・せ繝井ｸｦ襍ｰ縺ｧ縺ｮ繝昴・繝郁｡晉ｪ∵凾縺ｫ
            // 繝｡繧､繝ｳ繝ｫ繝ｼ繝励・ API 蜿嶺ｿ｡縺縺第ｭ｢縺ｾ縺｣縺溘∪縺ｾ逕溘″邯壹￠縲√ョ繝舌ャ繧ｰ縺・            // 髱槫ｸｸ縺ｫ蝗ｰ髮｣縺ｫ縺ｪ繧九ょ叉譎ゅ・繝ｭ繧ｻ繧ｹ邨ゆｺ・＠縺ｦ螟ｱ謨励ｒ譏守｢ｺ縺ｫ縺吶ｋ縲・
            log::error!("Failed to bind replay API server on {addr}: {e}");
            eprintln!("fatal: failed to bind replay API server on {addr}: {e}");
            std::process::exit(1);
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

        // 1 繝ｪ繧ｯ繧ｨ繧ｹ繝・/ 謗･邯夲ｼ・eep-alive 縺ｪ縺暦ｼ・        // 繝倥ャ繝繝ｼ 64KB / 繝懊ョ繧｣ 16MB 縺ｾ縺ｧ蜍慕噪縺ｫ諡｡蠑ｵ縺吶ｋ・・hase 4a 縺ｮ繧ｹ繝翫ャ繝励す繝ｧ繝・ヨ 10MB 蟇ｾ蠢懶ｼ・

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

        // 繧ｹ繧ｯ繝ｪ繝ｼ繝ｳ繧ｷ繝ｧ繝・ヨ: iced app state 荳崎ｦ√↑縺溘ａ逶ｴ謗･縺薙％縺ｧ蜃ｦ逅・

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
            Err(RouteError::BadRequestWithMessage(msg)) => {
                let body = serde_json::json!({ "error": msg }).to_string();
                let _ = write_response(&mut stream, 400, &body).await;
                continue;
            }
            Err(RouteError::PayloadTooLarge) => {
                let _ = write_response(&mut stream, 413, r#"{"error":"Payload Too Large"}"#).await;
                continue;
            }
            Err(RouteError::NotImplemented) => {
                let _ = write_response(&mut stream, 501, NOT_IMPLEMENTED_MULTI_SESSION_BODY).await;
                continue;
            }
        };

        // oneshot 縺ｧ iced app 縺九ｉ縺ｮ繝ｬ繧ｹ繝昴Φ繧ｹ繧貞ｾ・▽
        let (reply_tx, reply_rx) = oneshot::channel::<(u16, String)>();
        if sender
            .send((command, ReplySender::new(reply_tx)))
            .await
            .is_err()
        {
            let _ = write_response(&mut stream, 500, r#"{"error":"App channel closed"}"#).await;
            continue;
        }

        // 30s 繧ｿ繧､繝繧｢繧ｦ繝医ょｰ・擂 handle_command 縺・reply 繧定ｿ斐＠蠢倥ｌ縺溷ｴ蜷医ｄ縲・        // panic 縺ｫ繧医ｋ early drop 縺ｧ oneshot 縺碁哩縺倥ｉ繧後★縺ｫ豌ｸ荵・ｾ・ｩ溘＠縺ｪ縺・ｈ縺・        // 髦ｲ蠕｡螻､繧貞ｼｵ繧九る壼ｸｸ縺ｮ narrative 蜃ｦ逅・〒繧・p95 縺ｧ謨ｰ逋ｾ ms 莉･蜀・・縺溘ａ
        // 30s 縺ｯ蜊∝・繝槭・繧ｸ繝ｳ縺後≠繧九・
        match tokio::time::timeout(std::time::Duration::from_secs(30), reply_rx).await {
            Ok(Ok((status, json))) => {
                let _ = write_response(&mut stream, status, &json).await;
            }
            Ok(Err(_)) => {
                let _ =
                    write_response(&mut stream, 500, r#"{"error":"No response from app"}"#).await;
            }
            Err(_) => {
                log::error!(
                    "Replay API: handler did not respond within 30s (method={method} path={path})"
                );
                let _ = write_response(
                    &mut stream,
                    504,
                    r#"{"error":"Gateway Timeout: handler did not respond within 30s"}"#,
                )
                .await;
            }
        }
    }
}

/// 邁｡譏・HTTP 繝ｪ繧ｯ繧ｨ繧ｹ繝医ヱ繝ｼ繧ｵ繝ｼ縲・method, path, body) 繧定ｿ斐☆縲・
fn parse_request(raw: &str) -> Option<(String, String, String)> {
    let mut lines = raw.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    // HTTP/1.x is ignored. The body starts after the blank line.
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
    /// BadRequest 縺ｨ蜷後§ 400 繧ｹ繝・・繧ｿ繧ｹ繧定ｿ斐☆縺後√Ξ繧ｹ繝昴Φ繧ｹ譛ｬ譁・↓蜈ｷ菴鍋噪縺ｪ
    /// 繧ｨ繝ｩ繝ｼ繝｡繝・そ繝ｼ繧ｸ繧貞性繧√ｋ・・hase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ F・峨Ｂgent API 縺ｮ
    /// silent failure 蝗樣∩縺ｮ縺溘ａ縲∝次蝗繧呈・遉ｺ縺励※霑斐☆縲・    
    BadRequestWithMessage(String),
    PayloadTooLarge,
    /// Multi-session routes are reserved until a future phase.
    NotImplemented,
}

/// `RouteError::NotImplemented` 縺ｫ蟇ｾ蠢懊☆繧・501 繝ｬ繧ｹ繝昴Φ繧ｹ譛ｬ譁・・/// ADR-0001 / phase4b_agent_replay_api.md ﾂｧ4.5 縺ｧ蝗ｺ螳壽枚險縺ｨ縺励※螳夂ｾｩ縲・
pub(crate) const NOT_IMPLEMENTED_MULTI_SESSION_BODY: &str =
    r#"{"error":"multi-session not yet implemented; use 'default' until Phase 4c"}"#;

/// body 縺九ｉ譁・ｭ怜・繝輔ぅ繝ｼ繝ｫ繝峨ｒ蜿悶ｊ蜃ｺ縺・
fn body_str_field(body: &str, key: &str) -> Result<String, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    parsed
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(RouteError::BadRequest)
}

/// body 縺九ｉ uuid 繝輔ぅ繝ｼ繝ｫ繝峨ｒ蜿悶ｊ蜃ｺ縺・
fn body_uuid_field(body: &str, key: &str) -> Result<uuid::Uuid, RouteError> {
    let s = body_str_field(body, key)?;
    uuid::Uuid::parse_str(&s).map_err(|_| RouteError::BadRequest)
}

/// `/api/agent/session/:id/<suffix>` 蠖｢蠑上・繝代せ縺九ｉ `:id` 繧呈歓蜃ｺ縺吶ｋ縲・/// `:id` 縺檎ｩｺ繝ｻ`/` 繧貞性繧縺ｪ縺ｩ縺ｮ蝣ｴ蜷医・ `BadRequest`縲・/// ADR-0001 縺ｫ蝓ｺ縺･縺・`:id != "default"` 縺ｯ `NotImplemented`・・01・峨・
fn extract_agent_session_id<'a>(path: &'a str, suffix: &str) -> Result<&'a str, RouteError> {
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

fn parse_agent_session_advance(path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    let session_id = extract_agent_session_id(path, "advance")?;
    if session_id != "default" {
        return Err(RouteError::NotImplemented);
    }
    let request = crate::api::advance_request::parse_agent_advance_request(body)
        .map_err(RouteError::BadRequestWithMessage)?;
    Ok(ApiCommand::AgentSession(AgentSessionCommand::Advance {
        session_id: session_id.to_string(),
        request: Box::new(request),
    }))
}

fn parse_init_range_body(body: &str) -> Result<Option<(String, String)>, RouteError> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    let start = body_str_field(body, "start")
        .map_err(|_| RouteError::BadRequestWithMessage("start field required".to_string()))?;
    let end = body_str_field(body, "end")
        .map_err(|_| RouteError::BadRequestWithMessage("end field required".to_string()))?;
    Ok(Some((start, end)))
}

fn parse_agent_session_rewind(path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    let session_id = extract_agent_session_id(path, "rewind-to-start")?;
    if session_id != "default" {
        return Err(RouteError::NotImplemented);
    }
    let init_range = parse_init_range_body(body)?;
    Ok(ApiCommand::AgentSession(
        AgentSessionCommand::RewindToStart {
            session_id: session_id.to_string(),
            init_range,
        },
    ))
}

fn parse_agent_session_order(path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    let session_id = extract_agent_session_id(path, "order")?;
    if session_id != "default" {
        return Err(RouteError::NotImplemented);
    }
    let request = crate::api::order_request::parse_agent_order_request(body)
        .map_err(RouteError::BadRequestWithMessage)?;
    Ok(ApiCommand::AgentSession(AgentSessionCommand::PlaceOrder {
        session_id: session_id.to_string(),
        request: Box::new(request),
    }))
}

/// 繝代せ縺ｨ繝｡繧ｽ繝・ラ縺九ｉ ApiCommand 縺ｫ繝ｫ繝ｼ繝・ぅ繝ｳ繧ｰ
///
/// C-3: 莉･蜑阪・ `return` 繧剃ｽｿ縺｣縺・2 縺､縺ｮ match 繝悶Ο繝・け縺ｫ蛻・°繧後※縺・◆縲・/// 蜊倅ｸ縺ｮ match 蠑上↓邨ｱ蜷医＠縲∬､・尅縺ｪ body 繝代・繧ｹ縺ｯ蟆ら畑繝倥Ν繝代・髢｢謨ｰ縺ｫ蛻・ｊ蜃ｺ縺励◆縲・
fn route(method: &str, path: &str, body: &str) -> Result<ApiCommand, RouteError> {
    match (method, path) {
        // 笏笏 Replay 蛻ｶ蠕｡ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        // ADR-0001: old play/pause/resume/speed/step-* routes are removed.
        ("GET", "/api/replay/status") => Ok(ApiCommand::Replay(ReplayCommand::GetStatus)),
        ("POST", "/api/replay/toggle") => Ok(ApiCommand::Replay(ReplayCommand::Toggle {
            init_range: parse_init_range_body(body)?,
        })),

        // 笏笏 App 蛻ｶ蠕｡ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("POST", "/api/app/save") => Ok(ApiCommand::Replay(ReplayCommand::SaveState)),
        ("POST", "/api/app/set-mode") => parse_set_mode_command(body),

        // 笏笏 隱崎ｨｼ・域悽逡ｪ繝薙Ν繝峨↓繧ょ性縺ｾ繧後ｋ・俄楳笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("GET", "/api/auth/tachibana/status") => {
            Ok(ApiCommand::Auth(AuthCommand::TachibanaSessionStatus))
        }
        ("POST", "/api/auth/tachibana/logout") => {
            Ok(ApiCommand::Auth(AuthCommand::TachibanaLogout))
        }

        // 笏笏 繝壹う繝ｳ CRUD 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
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

        // 笏笏 縺昴・莉・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("GET", "/api/notification/list") => Ok(ApiCommand::Pane(PaneCommand::ListNotifications)),
        ("POST", "/api/sidebar/select-ticker") => parse_sidebar_select_ticker(body),
        ("POST", "/api/sidebar/open-order-pane") => parse_open_order_pane(body),

        // 笏笏 莉ｮ諠ｳ邏・ｮ壹お繝ｳ繧ｸ繝ｳ・・hase 2 莠呈鋤・俄楳笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
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

        // 笏笏 繝翫Λ繝・ぅ繝・API・・hase 4a・俄楳笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
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

        // 笏笏 Agent 蟆ら畑 Replay API・・hase 4b-1・俄楳笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/step") => {
            parse_agent_session_step(p)
        }
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/order") => {
            parse_agent_session_order(p, body)
        }
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/advance") => {
            parse_agent_session_advance(p, body)
        }
        ("POST", p) if p.starts_with("/api/agent/session/") && p.ends_with("/rewind-to-start") => {
            parse_agent_session_rewind(p, body)
        }

        // 笏笏 遶玖干險ｼ蛻ｸ菴吝鴨諠・ｱ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("GET", "/api/buying-power") => Ok(ApiCommand::FetchBuyingPower),

        // 笏笏 遶玖干險ｼ蛻ｸ譁ｰ隕乗ｳｨ譁・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("POST", "/api/tachibana/order") => parse_tachibana_new_order(body),

        // 笏笏 遶玖干險ｼ蛻ｸ豕ｨ譁・ｮ｡逅・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("GET", p) if p == "/api/tachibana/orders" || p.starts_with("/api/tachibana/orders?") => {
            let eig_day = query_param(p, "eig_day").unwrap_or_default();
            Ok(ApiCommand::FetchTachibanaOrders { eig_day })
        }
        ("GET", p) if p.starts_with("/api/tachibana/order/") => {
            parse_tachibana_order_detail_command(p)
        }
        ("POST", "/api/tachibana/order/correct") => parse_tachibana_correct_order(body),
        ("POST", "/api/tachibana/order/cancel") => parse_tachibana_cancel_order(body),

        // 笏笏 遶玖干險ｼ蛻ｸ菫晄怏迴ｾ迚ｩ譬ｪ謨ｰ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        ("GET", p)
            if p == "/api/tachibana/holdings" || p.starts_with("/api/tachibana/holdings?") =>
        {
            let issue_code = query_param(p, "issue_code").ok_or(RouteError::BadRequest)?;
            Ok(ApiCommand::FetchTachibanaHoldings { issue_code })
        }

        // 笏笏 debug 繝薙Ν繝峨〒譛牙柑・・eyring 繧ｯ繝ｪ繧｢・・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
        #[cfg(debug_assertions)]
        ("POST", "/api/test/tachibana/delete-persisted-session") => Ok(ApiCommand::Test(
            TestCommand::TachibanaDeletePersistedSession,
        )),

        _ => Err(RouteError::NotFound),
    }
}

/// `POST /api/replay/order` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
fn parse_virtual_order_command(body: &str) -> Result<ApiCommand, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;

    let ticker_raw = parsed
        .get("ticker")
        .and_then(|v| v.as_str())
        .ok_or(RouteError::BadRequest)?;
    // SerTicker 蠖｢蠑・`"Exchange:Symbol"` 縺ｧ譚･縺ｦ繧ょ女縺台ｻ倥￠繧九ょ・驛ｨ縺ｮ
    // `on_tick` 豈碑ｼ・・ Symbol 蜊倅ｽ難ｼ・Ticker::Display`・峨↑縺ｮ縺ｧ縲√％縺薙〒
    // prefix 繧貞翁縺後＆縺ｪ縺・→繧ｵ繧､繝ｬ繝ｳ繝医↓ Pending 縺ｮ縺ｾ縺ｾ谿九ｋ縲・
    let ticker = ticker_raw
        .split_once(':')
        .map(|(_, sym)| sym.to_string())
        .unwrap_or_else(|| ticker_raw.to_string());
    if ticker.is_empty() {
        return Err(RouteError::BadRequest);
    }

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
        // order_type 逵∫払 竊・market
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

/// Returns a decoded query parameter from a URL path.
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

/// `GET /api/pane/chart-snapshot?pane_id=<uuid>[&limit=N][&since_ts=<ms>]` 繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
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

/// `POST /api/app/set-mode` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・/// body: `{"mode": "live" | "replay"}`
fn parse_set_mode_command(body: &str) -> Result<ApiCommand, RouteError> {
    let mode = body_str_field(body, "mode")?;
    match mode.to_lowercase().as_str() {
        "live" | "replay" => Ok(ApiCommand::Replay(ReplayCommand::SetMode {
            mode: mode.to_lowercase(),
        })),
        _ => Err(RouteError::BadRequest),
    }
}

/// `POST /api/pane/split` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
fn parse_split_command(body: &str) -> Result<ApiCommand, RouteError> {
    let pane_id = body_uuid_field(body, "pane_id")?;
    let axis = body_str_field(body, "axis")?;
    match axis.as_str() {
        "Vertical" | "vertical" | "Horizontal" | "horizontal" => {}
        _ => return Err(RouteError::BadRequest),
    }
    Ok(ApiCommand::Pane(PaneCommand::Split { pane_id, axis }))
}

/// `POST /api/sidebar/open-order-pane` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
fn parse_open_order_pane(body: &str) -> Result<ApiCommand, RouteError> {
    let kind = body_str_field(body, "kind")?;
    match kind.as_str() {
        "OrderEntry" | "OrderList" | "BuyingPower" => {}
        _ => return Err(RouteError::BadRequest),
    }
    Ok(ApiCommand::Pane(PaneCommand::OpenOrderPane { kind }))
}

/// `POST /api/tachibana/order` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・/// `second_password` 縺ｯ譛ｬ譁・ｸｭ縺ｮ繝輔ぅ繝ｼ繝ｫ繝峨° `DEV_SECOND_PASSWORD` 迺ｰ蠅・､画焚縺九ｉ蜿門ｾ励☆繧九・
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

    // DEV_SECOND_PASSWORD 繝輔か繝ｼ繝ｫ繝舌ャ繧ｯ縺ｯ debug 繝薙Ν繝会ｼ・2E 繝・せ繝茨ｼ牙ｰら畑
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

/// `GET /api/tachibana/order/{order_num}[?eig_day=YYYYMMDD]` 繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
fn parse_tachibana_order_detail_command(path: &str) -> Result<ApiCommand, RouteError> {
    // 繝代せ驛ｨ蛻・→繧ｯ繧ｨ繝ｪ驛ｨ蛻・ｒ蛻・屬縺吶ｋ
    let (path_part, _) = path.split_once('?').unwrap_or((path, ""));
    let order_num = path_part
        .strip_prefix("/api/tachibana/order/")
        .filter(|s| !s.is_empty())
        .ok_or(RouteError::BadRequest)?
        .to_string();
    let eig_day = query_param(path, "eig_day").unwrap_or_default();
    Ok(ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day })
}

/// `POST /api/tachibana/order/correct` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・/// `second_password` 縺ｯ譛ｬ譁・∪縺溘・ `DEV_SECOND_PASSWORD` 迺ｰ蠅・､画焚縺九ｉ蜿門ｾ励☆繧九・
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
    // DEV_SECOND_PASSWORD 繝輔か繝ｼ繝ｫ繝舌ャ繧ｯ縺ｯ debug 繝薙Ν繝会ｼ・2E 繝・せ繝茨ｼ牙ｰら畑
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

/// `POST /api/tachibana/order/cancel` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・/// `second_password` 縺ｯ譛ｬ譁・∪縺溘・ `DEV_SECOND_PASSWORD` 迺ｰ蠅・､画焚縺九ｉ蜿門ｾ励☆繧九・
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
    // DEV_SECOND_PASSWORD 繝輔か繝ｼ繝ｫ繝舌ャ繧ｯ縺ｯ debug 繝薙Ν繝会ｼ・2E 繝・せ繝茨ｼ牙ｰら畑
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

/// `POST /api/sidebar/select-ticker` 縺ｮ繝懊ョ繧｣繧偵ヱ繝ｼ繧ｹ縺励※ ApiCommand 繧定ｿ斐☆縲・
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

/// `POST /api/agent/narrative` 縺ｮ繝懊ョ繧｣繧・`NarrativeCreateRequest` 縺ｫ繝代・繧ｹ縺吶ｋ縲・
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
    // 譌ｩ譛溘し繧､繧ｺ繝√ぉ繝・け・亥悸邵ｮ蜑搾ｼ峨りｨ育判 ﾂｧ3.2 繝上・繝我ｸ企剞 10 MB縲・
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
    // /api/agent/narrative/{id}/snapshot 縺ｪ縺ｩ縺ｮ繧ｵ繝輔ぅ繝・け繧ｹ繧定誠縺ｨ縺・
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

/// body 縺九ｉ逵∫払蜿ｯ閭ｽ縺ｪ譁・ｭ怜・繝輔ぅ繝ｼ繝ｫ繝峨ｒ蜿悶ｊ蜃ｺ縺吶・/// 繝輔ぅ繝ｼ繝ｫ繝峨′蟄伜惠縺励↑縺・ｴ蜷医√∪縺溘・蛟､縺・JSON `null` 縺ｮ蝣ｴ蜷医・ `None` 繧定ｿ斐☆縲・/// ・亥ｿ・医ヵ繧｣繝ｼ繝ｫ繝臥畑縺ｮ `body_str_field` 縺ｯ null 繧・400 縺ｨ縺励※諡貞凄縺吶ｋ縲ゑｼ・
fn body_opt_str_field(body: &str, key: &str) -> Result<Option<String>, RouteError> {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    Ok(parsed
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

/// HTTP 繝ｬ繧ｹ繝昴Φ繧ｹ繧呈嶌縺崎ｾｼ繧
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

/// 繝・せ繧ｯ繝医ャ繝怜・菴薙・繧ｹ繧ｯ繝ｪ繝ｼ繝ｳ繧ｷ繝ｧ繝・ヨ繧・C:/tmp/screenshot.png 縺ｫ菫晏ｭ倥☆繧九・/// spawn_blocking 縺九ｉ蜻ｼ縺ｶ縺薙→・・ync API・峨・
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

    // 笏笏 parse_request tests 笏笏

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
        // No \r\n\r\n separator 竊・body should be empty
        let raw = "GET /api/replay/status HTTP/1.1\r\nHost: localhost";
        let result = parse_request(raw);
        assert!(result.is_some());
        let (_, _, body) = result.unwrap();
        assert!(body.is_empty());
    }

    // 笏笏 route tests: replay 笏笏

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

    // 笏笏 route tests: agent session (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ B) 笏笏

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
        // ADR-0001: session_id != "default" 縺ｯ NotImplemented・・01・峨〒諡貞凄縲・
        let result = route("POST", "/api/agent/session/other/step", "");
        assert!(
            matches!(result, Err(RouteError::NotImplemented)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_agent_session_step_rejects_empty_session_id() {
        // `/api/agent/session//step` 窶・遨ｺ繧ｻ繝・す繝ｧ繝ｳ ID 縺ｯ BadRequest縲・
        let result = route("POST", "/api/agent/session//step", "");
        assert!(
            matches!(result, Err(RouteError::BadRequest)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_agent_session_step_rejects_get_method() {
        // step 縺ｯ POST 縺ｮ縺ｿ縲・ET 縺ｧ縺ｯ NotFound 縺ｫ繝輔か繝ｼ繝ｫ繝舌ャ繧ｯ縲・
        let result = route("GET", "/api/agent/session/default/step", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_agent_session_rejects_unknown_suffix() {
        // `/step` / `/advance` / `/order` 莉･螟悶・ suffix 縺ｯ NotFound縲・        // 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ B 譎らせ縺ｧ縺ｯ `/step` 縺ｮ縺ｿ繝ｫ繝ｼ繝・ぅ繝ｳ繧ｰ貂医∩縲・
        let result = route("POST", "/api/agent/session/default/unknown", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_agent_session_step_rejects_non_default_uuid_like() {
        // UUID 鬚ｨ縺ｮ session_id 繧よ拠蜷ｦ・・hase 4c 縺ｾ縺ｧ "default" 蝗ｺ螳夲ｼ峨・
        let result = route(
            "POST",
            "/api/agent/session/550e8400-e29b-41d4-a716-446655440000/step",
            "",
        );
        assert!(matches!(result, Err(RouteError::NotImplemented)));
    }

    // 笏笏 route tests: agent session order (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ E) 笏笏

    const VALID_ORDER_BODY: &str = r#"{
        "client_order_id": "cli_42",
        "ticker": {"exchange": "HyperliquidLinear", "symbol": "BTC"},
        "side": "buy",
        "qty": 0.1,
        "order_type": {"market": {}}
    }"#;

    #[test]
    fn route_agent_session_order_accepts_default_with_valid_body() {
        let cmd = route("POST", "/api/agent/session/default/order", VALID_ORDER_BODY).unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::PlaceOrder {
                session_id,
                request,
            }) => {
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

    fn assert_bad_request_with_keyword(result: Result<ApiCommand, RouteError>, keyword: &str) {
        match result {
            Err(RouteError::BadRequestWithMessage(msg)) => {
                assert!(
                    msg.to_ascii_lowercase()
                        .contains(&keyword.to_ascii_lowercase()),
                    "error message missing keyword {keyword:?}: {msg}"
                );
            }
            other => panic!("expected BadRequestWithMessage containing {keyword:?}, got {other:?}"),
        }
    }

    #[test]
    fn route_agent_session_order_rejects_string_ticker_with_specific_message() {
        let body = r#"{
            "client_order_id": "cli_1",
            "ticker": "HyperliquidLinear:BTC",
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        assert_bad_request_with_keyword(
            route("POST", "/api/agent/session/default/order", body),
            "ticker",
        );
    }

    #[test]
    fn route_agent_session_order_rejects_missing_order_type_with_specific_message() {
        let body = r#"{
            "client_order_id": "cli_1",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1
        }"#;
        assert_bad_request_with_keyword(
            route("POST", "/api/agent/session/default/order", body),
            "order_type",
        );
    }

    #[test]
    fn route_agent_session_order_rejects_missing_client_order_id_with_specific_message() {
        let body = r#"{
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        assert_bad_request_with_keyword(
            route("POST", "/api/agent/session/default/order", body),
            "client_order_id",
        );
    }

    #[test]
    fn route_agent_session_order_rejects_invalid_client_order_id_charset_with_specific_message() {
        let body = r#"{
            "client_order_id": "cli 42",
            "ticker": {"exchange": "X", "symbol": "Y"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}}
        }"#;
        assert_bad_request_with_keyword(
            route("POST", "/api/agent/session/default/order", body),
            "client_order_id",
        );
    }

    #[test]
    fn route_agent_session_order_rejects_get_method() {
        let result = route("GET", "/api/agent/session/default/order", VALID_ORDER_BODY);
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    // 笏笏 route tests: agent session advance (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ G) 笏笏

    #[test]
    fn route_agent_session_advance_accepts_default_with_valid_body() {
        let body = r#"{"until_ms": 1704067200000}"#;
        let cmd = route("POST", "/api/agent/session/default/advance", body).unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::Advance {
                session_id,
                request,
            }) => {
                assert_eq!(session_id, "default");
                assert_eq!(request.until_ms.as_u64(), 1_704_067_200_000);
            }
            other => panic!("expected Advance, got {other:?}"),
        }
    }

    #[test]
    fn route_agent_session_advance_rejects_non_default_session() {
        let body = r#"{"until_ms": 100}"#;
        let result = route("POST", "/api/agent/session/other/advance", body);
        assert!(matches!(result, Err(RouteError::NotImplemented)));
    }

    #[test]
    fn route_agent_session_advance_rejects_missing_until_ms() {
        let result = route("POST", "/api/agent/session/default/advance", "{}");
        assert_bad_request_with_keyword(result, "until_ms");
    }

    #[test]
    fn route_agent_session_advance_rejects_end_in_stop_on() {
        // plan ﾂｧ4.3: "end" 縺ｯ荳肴ｭ｣蛟､・育ｯ・峇邨らｫｯ縺ｯ蟶ｸ縺ｫ蛛懈ｭ｢縺吶ｋ縺溘ａ譏守､ｺ荳崎ｦ・ｼ・
        let body = r#"{"until_ms": 100, "stop_on": ["end"]}"#;
        let result = route("POST", "/api/agent/session/default/advance", body);
        // serde reports "end" as an unknown variant.
        assert_bad_request_with_keyword(result, "end");
    }

    #[test]
    fn route_agent_session_advance_rejects_unknown_field() {
        let body = r#"{"until_ms": 100, "unknown": "x"}"#;
        let result = route("POST", "/api/agent/session/default/advance", body);
        assert_bad_request_with_keyword(result, "unknown");
    }

    #[test]
    fn not_implemented_body_matches_adr_spec() {
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
        assert!(matches!(
            unwrap_replay(cmd),
            ReplayCommand::Toggle { init_range: None }
        ));
    }

    #[test]
    fn route_post_toggle_with_init_range() {
        let body = r#"{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}"#;
        let cmd = route("POST", "/api/replay/toggle", body).unwrap();
        assert!(matches!(
            unwrap_replay(cmd),
            ReplayCommand::Toggle {
                init_range: Some((start, end))
            } if start == "2026-04-01 09:00" && end == "2026-04-01 15:00"
        ));
    }

    #[test]
    fn route_post_toggle_rejects_missing_end() {
        let body = r#"{"start":"2026-04-01 09:00"}"#;
        assert_bad_request_with_keyword(route("POST", "/api/replay/toggle", body), "end");
    }

    #[test]
    fn route_unknown_path_not_found() {
        let result = route("GET", "/api/replay/unknown", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    #[test]
    fn route_get_on_post_endpoint_not_found() {
        // GET on POST-only endpoints should return NotFound.
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

    // 笏笏 route tests: narrative・・hase 4a・俄楳笏

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
        // 11MB of JSON string content 竊・exceeds MAX_UNCOMPRESSED_BYTES (10MB)
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

    // 笏笏 route tests: pane 笏笏

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

    // 笏笏 route tests: chart-snapshot 笏笏

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

    // 笏笏 query_param 笏笏

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

    // 笏笏 route tests: open-order-pane 笏笏

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

    // 笏笏 route tests: auth 笏笏

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
        // POST 縺ｯ繝槭ャ繝√＠縺ｪ縺・ｼ・ET 縺ｮ縺ｿ・・
        let result = route("POST", "/api/auth/tachibana/status", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    // 笏笏 inject-* 繧ｨ繝ｳ繝峨・繧､繝ｳ繝医・蟄伜惠縺励↑縺・◆繧・404 笏笏

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

    // 笏笏 route tests: GET /api/replay/orders 笏笏

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
        // POST 縺ｯ繝槭ャ繝√＠縺ｪ縺・ｼ・ET 縺ｮ縺ｿ・・
        let result = route("POST", "/api/replay/orders", "");
        assert!(matches!(result, Err(RouteError::NotFound)));
    }

    /// SerTicker 蠖｢蠑擾ｼ・Exchange:Symbol"・峨ｂ bare symbol 縺ｫ豁｣隕丞喧縺励※蜿励￠繧九・    /// `/api/replay/state` 縺・`"stream":"BinanceLinear:BTCUSDT:1m"` 繧定ｿ斐☆縺溘ａ
    /// 繧ｨ繝ｼ繧ｸ繧ｧ繝ｳ繝医′閾ｪ辟ｶ縺ｫ SerTicker 繧帝√ｊ霑斐☆繝代せ縺後≠繧九Ｐn_tick 蛛ｴ縺ｯ
    /// bare symbol 縺ｧ縺励°豈碑ｼ・＠縺ｪ縺・・縺ｧ縲√％縺薙〒蜑･縺後＆縺ｪ縺・→繧ｵ繧､繝ｬ繝ｳ繝医↓
    /// Pending 縺ｮ縺ｾ縺ｾ fill 縺帙★縲〕inked narrative 縺ｮ outcome 繧ょ沂縺ｾ繧峨↑縺・・    
    #[test]
    fn place_order_normalizes_ser_ticker_prefix() {
        let body =
            r#"{"ticker":"BinanceLinear:BTCUSDT","side":"buy","qty":0.005,"order_type":"market"}"#;
        let cmd = route("POST", "/api/replay/order", body).unwrap();
        match cmd {
            ApiCommand::VirtualExchange(VirtualExchangeCommand::PlaceOrder { ticker, .. }) => {
                assert_eq!(ticker, "BTCUSDT", "SerTicker prefix must be stripped");
            }
            _ => panic!("expected PlaceOrder"),
        }
    }

    /// bare symbol 縺ｯ縺昴・縺ｾ縺ｾ騾壹☆・域里蟄・E2E 縺ｨ縺ｮ蠕梧婿莠呈鋤・峨・    
    #[test]
    fn place_order_accepts_bare_symbol() {
        let body = r#"{"ticker":"BTCUSDT","side":"sell","qty":0.01,"order_type":"market"}"#;
        let cmd = route("POST", "/api/replay/order", body).unwrap();
        match cmd {
            ApiCommand::VirtualExchange(VirtualExchangeCommand::PlaceOrder {
                ticker,
                side,
                ..
            }) => {
                assert_eq!(ticker, "BTCUSDT");
                assert_eq!(side, "sell");
            }
            _ => panic!("expected PlaceOrder"),
        }
    }

    /// 繝励Ξ繝輔ぅ繝・け繧ｹ縺縺代・ "Exchange:" 縺ｯ遨ｺ譁・ｭ・ticker 縺ｨ縺励※ 400縲・    
    #[test]
    fn place_order_rejects_empty_symbol_after_prefix() {
        let body = r#"{"ticker":"BinanceLinear:","side":"buy","qty":0.005}"#;
        let result = route("POST", "/api/replay/order", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // 笏笏 body_opt_str_field tests 笏笏

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
        // 莉墓ｧ・ JSON null 縺ｯ繝輔ぅ繝ｼ繝ｫ繝臥怐逡･縺ｨ遲我ｾ｡ 竊・None
        let r = body_opt_str_field(r#"{"kind":null}"#, "kind").unwrap();
        assert_eq!(r, None);
    }

    #[test]
    fn opt_str_field_invalid_json() {
        let r = body_opt_str_field("not json", "kind");
        assert!(matches!(r, Err(RouteError::BadRequest)));
    }

    // 笏笏 Phase 3: 豕ｨ譁・ｮ｡逅・4 繝ｫ繝ｼ繝・RED 繝・せ繝・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    /// GET /api/tachibana/orders -> FetchTachibanaOrders { eig_day: "" }.
    #[test]
    fn route_get_tachibana_orders_no_param() {
        let cmd = route("GET", "/api/tachibana/orders", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrders { eig_day } => {
                assert_eq!(eig_day, "");
            }
            _ => panic!("Expected FetchTachibanaOrders, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/orders?eig_day=20260417 竊・eig_day 縺悟叙繧後ｋ
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

    /// GET /api/tachibana/order/12345678 竊・FetchTachibanaOrderDetail
    #[test]
    fn route_get_tachibana_order_detail() {
        let cmd = route("GET", "/api/tachibana/order/12345678", "").unwrap();
        match cmd {
            ApiCommand::FetchTachibanaOrderDetail { order_num, eig_day } => {
                assert_eq!(order_num, "12345678");
                assert_eq!(eig_day, "");
            }
            _ => panic!("Expected FetchTachibanaOrderDetail, got {cmd:?}"),
        }
    }

    /// GET /api/tachibana/order/12345678?eig_day=20260417 竊・eig_day 繧ｯ繧ｨ繝ｪ繝代Λ繝｡繝ｼ繧ｿ蜿門ｾ・    
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

    /// GET /api/tachibana/order/ (order_num 遨ｺ) 竊・BadRequest
    #[test]
    fn route_get_tachibana_order_detail_empty_order_num_bad_request() {
        let result = route("GET", "/api/tachibana/order/", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/correct 竊・TachibanaCorrectOrder
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

    /// POST /api/tachibana/order/correct: 蠢・医ヵ繧｣繝ｼ繝ｫ繝画ｬ關ｽ 竊・BadRequest
    #[test]
    fn route_post_tachibana_order_correct_missing_field() {
        let body = r#"{"price":"2600"}"#;
        let result = route("POST", "/api/tachibana/order/correct", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/cancel 竊・TachibanaOrderCancel
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

    /// POST /api/tachibana/order/cancel: 蠢・医ヵ繧｣繝ｼ繝ｫ繝画ｬ關ｽ 竊・BadRequest
    #[test]
    fn route_post_tachibana_order_cancel_missing_order_number() {
        let body = r#"{"eig_day":"20260417"}"#;
        let result = route("POST", "/api/tachibana/order/cancel", body);
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// POST /api/tachibana/order/cancel: 荳肴ｭ｣ JSON 竊・BadRequest
    #[test]
    fn route_post_tachibana_order_cancel_invalid_json() {
        let result = route("POST", "/api/tachibana/order/cancel", "not json");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    /// GET /api/tachibana/holdings?issue_code=7203 竊・FetchTachibanaHoldings
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

    /// GET /api/tachibana/holdings (issue_code 縺ｪ縺・ 竊・BadRequest
    #[test]
    fn route_get_tachibana_holdings_missing_issue_code_bad_request() {
        let result = route("GET", "/api/tachibana/holdings", "");
        assert!(matches!(result, Err(RouteError::BadRequest)));
    }

    // Sub-phase L: deleted routes must return NotFound.
    #[test]
    fn route_post_replay_play_is_deleted() {
        let body = r#"{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}"#;
        let result = route("POST", "/api/replay/play", body);
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/play must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    #[test]
    fn route_post_replay_pause_is_deleted() {
        let result = route("POST", "/api/replay/pause", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/pause must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    #[test]
    fn route_post_replay_resume_is_deleted() {
        let result = route("POST", "/api/replay/resume", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/resume must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    #[test]
    fn route_post_replay_speed_is_deleted() {
        let result = route("POST", "/api/replay/speed", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/speed must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    #[test]
    fn route_post_replay_step_forward_is_deleted() {
        let result = route("POST", "/api/replay/step-forward", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/step-forward must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    #[test]
    fn route_post_replay_step_backward_is_deleted() {
        let result = route("POST", "/api/replay/step-backward", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "POST /api/replay/step-backward must be deleted per ADR-0001 ﾂｧ3, got {result:?}"
        );
    }

    // 笏笏 Sub-phase Q hotfix: rewind-to-start HTTP route 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏
    #[test]
    fn route_post_agent_rewind_default_empty_body() {
        let cmd = route("POST", "/api/agent/session/default/rewind-to-start", "").unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::RewindToStart {
                session_id,
                init_range,
            }) => {
                assert_eq!(session_id, "default");
                assert!(init_range.is_none(), "empty body 竊・init_range None");
            }
            other => panic!("expected RewindToStart, got {other:?}"),
        }
    }

    #[test]
    fn route_post_agent_rewind_default_with_init_body() {
        let body = r#"{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}"#;
        let cmd = route("POST", "/api/agent/session/default/rewind-to-start", body).unwrap();
        match cmd {
            ApiCommand::AgentSession(AgentSessionCommand::RewindToStart {
                init_range: Some((start, end)),
                ..
            }) => {
                assert_eq!(start, "2026-04-01 09:00");
                assert_eq!(end, "2026-04-01 15:00");
            }
            other => panic!("expected RewindToStart with init_range, got {other:?}"),
        }
    }

    #[test]
    fn route_post_agent_rewind_rejects_non_default_session() {
        let result = route("POST", "/api/agent/session/other/rewind-to-start", "");
        assert!(
            matches!(result, Err(RouteError::NotImplemented)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_post_agent_rewind_rejects_get_method() {
        let result = route("GET", "/api/agent/session/default/rewind-to-start", "");
        assert!(
            matches!(result, Err(RouteError::NotFound)),
            "got {result:?}"
        );
    }

    #[test]
    fn route_post_agent_rewind_rejects_partial_body() {
        // start 縺ｮ縺ｿ 竊・BadRequest
        let body = r#"{"start":"2026-04-01 09:00"}"#;
        let result = route("POST", "/api/agent/session/default/rewind-to-start", body);
        assert!(
            matches!(result, Err(RouteError::BadRequestWithMessage(_))),
            "got {result:?}"
        );
    }
}
