//! 立花証券 e支店 API アダプター
//!
//! ## API アクセスモデル
//!
//! すべてのリクエストは `{virtual_url}?{json_query}` 形式。
//! `virtual_url` はログイン応答で取得するセッション固有のURLで、1日間有効。
//!
//! ## 認証フロー
//! 1. 事前に電話認証（ユーザーが手動実施）
//! 2. `{BASE_URL}/auth/?{"sCLMID":"CLMAuthLoginRequest",...}` でログイン
//! 3. 応答から仮想URL群を取得・保存
//! 4. 以降は仮想URLでアクセス

use serde::{Deserialize, Serialize};

// ── エラー型 ─────────────────────────────────────────────────────────────────

#[derive(thiserror::Error, Debug)]
pub enum TachibanaError {
    #[error("ログイン失敗: {0}")]
    LoginFailed(String),
    #[error("未読書面があるため仮想URLが発行されていません")]
    UnreadNotices,
    #[error("HTTP エラー: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON エラー: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API エラー: code={code}, message={message}")]
    ApiError { code: String, message: String },
}

// ── セッション ────────────────────────────────────────────────────────────────

/// ログイン成功後に取得するセッション固有の仮想URL群。
/// セッション毎に異なる（1日間有効）。
#[derive(Debug, Clone)]
pub struct TachibanaSession {
    pub url_request: String,
    pub url_master: String,
    pub url_price: String,
    pub url_event: String,
    pub url_event_ws: String,
}

// ── ログイン型 ────────────────────────────────────────────────────────────────

/// CLMAuthLoginRequest リクエスト。
/// sCLMID フィールドは常に "CLMAuthLoginRequest" 固定。
#[derive(Debug, Serialize)]
pub struct LoginRequest {
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    #[serde(rename = "sUserId")]
    pub user_id: String,
    #[serde(rename = "sPassword")]
    pub password: String,
}

impl LoginRequest {
    pub fn new(user_id: String, password: String) -> Self {
        Self {
            clm_id: "CLMAuthLoginRequest",
            user_id,
            password,
        }
    }
}

/// CLMAuthLoginAck 応答。
/// sResultCode が "0" 以外はエラー。
/// sKinsyouhouMidokuFlg が "1" の場合、仮想URLは空で利用不可。
#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    #[serde(rename = "sCLMID")]
    pub clm_id: String,
    #[serde(rename = "sResultCode")]
    pub result_code: String,
    #[serde(rename = "sUrlRequest", default)]
    pub url_request: String,
    #[serde(rename = "sUrlMaster", default)]
    pub url_master: String,
    #[serde(rename = "sUrlPrice", default)]
    pub url_price: String,
    #[serde(rename = "sUrlEvent", default)]
    pub url_event: String,
    #[serde(rename = "sUrlEventWebSocket", default)]
    pub url_event_ws: String,
    /// 未読書面フラグ。"1" の場合は仮想URLが空。
    #[serde(rename = "sKinsyouhouMidokuFlg", default)]
    pub unread_notice_flag: String,
    #[serde(rename = "sResultText", default)]
    pub result_text: String,
}

impl TryFrom<LoginResponse> for TachibanaSession {
    type Error = TachibanaError;

    fn try_from(resp: LoginResponse) -> Result<Self, Self::Error> {
        if resp.result_code != "0" {
            return Err(TachibanaError::LoginFailed(format!(
                "code={}, message={}",
                resp.result_code, resp.result_text
            )));
        }
        if resp.unread_notice_flag == "1" {
            return Err(TachibanaError::UnreadNotices);
        }
        Ok(TachibanaSession {
            url_request: resp.url_request,
            url_master: resp.url_master,
            url_price: resp.url_price,
            url_event: resp.url_event,
            url_event_ws: resp.url_event_ws,
        })
    }
}

// ── URL 構築 ──────────────────────────────────────────────────────────────────

/// 立花証券 API の URL を構築する。
///
/// 形式: `{base_url}?{json_query}`
///
/// Note: 通常のクエリパラメータ形式ではなく JSON 文字列をそのまま `?` 以降に付加する
/// 独自形式のため、reqwest の query() メソッドは使えない。
pub fn build_api_url(base_url: &str, json_query: &str) -> String {
    format!("{}?{}", base_url, json_query)
}

/// リクエスト構造体をシリアライズして API URL を構築する。
pub fn build_api_url_from<T: Serialize>(
    base_url: &str,
    request: &T,
) -> Result<String, TachibanaError> {
    let json = serde_json::to_string(request)?;
    Ok(build_api_url(base_url, &json))
}

// ── 時価情報型 ────────────────────────────────────────────────────────────────

/// CLMMfdsGetMarketPrice リクエスト（スナップショット取得）。
/// 最大120銘柄まで同時取得可能。
#[derive(Debug, Serialize)]
pub struct MarketPriceRequest {
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    /// カンマ区切りの銘柄コード (例: "6501,7203")
    #[serde(rename = "sTargetIssueCode")]
    pub target_issue_codes: String,
    /// カンマ区切りの情報コード
    #[serde(rename = "sTargetColumn")]
    pub target_columns: String,
}

impl MarketPriceRequest {
    /// デフォルトの情報コード（現在値・四本値・出来高・前日終値）
    pub const DEFAULT_COLUMNS: &'static str = "pDPP,pDOP,pDHP,pDLP,pDV,pPRP";

    pub fn new(issue_codes: &[&str]) -> Self {
        Self {
            clm_id: "CLMMfdsGetMarketPrice",
            target_issue_codes: issue_codes.join(","),
            target_columns: Self::DEFAULT_COLUMNS.to_string(),
        }
    }
}

/// 単一銘柄の時価情報レコード。
/// 値はすべて文字列で返される（"*" は未取得/非対応）。
#[derive(Debug, Deserialize, Clone)]
pub struct MarketPriceRecord {
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,
    /// 現在値 (pDPP)
    #[serde(rename = "pDPP", default)]
    pub current_price: String,
    /// 始値 (pDOP)
    #[serde(rename = "pDOP", default)]
    pub open: String,
    /// 高値 (pDHP)
    #[serde(rename = "pDHP", default)]
    pub high: String,
    /// 安値 (pDLP)
    #[serde(rename = "pDLP", default)]
    pub low: String,
    /// 出来高 (pDV)
    #[serde(rename = "pDV", default)]
    pub volume: String,
    /// 前日終値 (pPRP)
    #[serde(rename = "pPRP", default)]
    pub prev_close: String,
}

/// CLMMfdsGetMarketPrice 応答。
#[derive(Debug, Deserialize)]
pub struct MarketPriceResponse {
    #[serde(rename = "aCLMMfdsMarketPrice")]
    pub records: Vec<MarketPriceRecord>,
}

// ── 日足履歴型 ────────────────────────────────────────────────────────────────

/// CLMMfdsGetMarketPriceHistory リクエスト（日足履歴取得）。
/// 1リクエスト1銘柄、最大約20年分のデータを取得可能。
#[derive(Debug, Serialize)]
pub struct DailyHistoryRequest {
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,
    /// 市場コード (東証: "00")
    #[serde(rename = "sSizyouC")]
    pub market_code: String,
}

impl DailyHistoryRequest {
    /// 東証の市場コード
    pub const TSE_MARKET_CODE: &'static str = "00";

    pub fn new(issue_code: &str) -> Self {
        Self {
            clm_id: "CLMMfdsGetMarketPriceHistory",
            issue_code: issue_code.to_string(),
            market_code: Self::TSE_MARKET_CODE.to_string(),
        }
    }
}

/// 日足1件のレコード。
/// OHLCV + 株式分割調整値（`*xK` サフィックス）。
#[derive(Debug, Deserialize, Clone)]
pub struct DailyHistoryRecord {
    /// 日付 (YYYYMMDD 形式)
    #[serde(rename = "sDate")]
    pub date: String,
    /// 始値
    #[serde(rename = "pDOP")]
    pub open: String,
    /// 高値
    #[serde(rename = "pDHP")]
    pub high: String,
    /// 安値
    #[serde(rename = "pDLP")]
    pub low: String,
    /// 終値
    #[serde(rename = "pDPP")]
    pub close: String,
    /// 出来高
    #[serde(rename = "pDV")]
    pub volume: String,
    // 株式分割調整値
    #[serde(rename = "pDOPxK", default)]
    pub open_adj: String,
    #[serde(rename = "pDHPxK", default)]
    pub high_adj: String,
    #[serde(rename = "pDLPxK", default)]
    pub low_adj: String,
    #[serde(rename = "pDPPxK", default)]
    pub close_adj: String,
    #[serde(rename = "pDVxK", default)]
    pub volume_adj: String,
}

/// CLMMfdsGetMarketPriceHistory 応答。
#[derive(Debug, Deserialize)]
pub struct DailyHistoryResponse {
    #[serde(rename = "aCLMMfdsGetMarketPriceHistory")]
    pub records: Vec<DailyHistoryRecord>,
}

// ── HTTP クライアント ─────────────────────────────────────────────────────────

/// 立花証券 API の BASE URL（本番）
pub const BASE_URL_PROD: &str = "https://kabuka.e-shiten.jp/e_api_v4r8/";

/// 立花証券 API の BASE URL（デモ）
pub const BASE_URL_DEMO: &str = "https://demo-kabuka.e-shiten.jp/e_api_v4r8/";

/// 認証エンドポイントのパス
pub const AUTH_PATH: &str = "auth/";

/// ログイン処理。
/// 成功時は `TachibanaSession` を返す。
/// 未読書面がある場合は `TachibanaError::UnreadNotices`。
pub async fn login(
    client: &reqwest::Client,
    base_url: &str,
    user_id: String,
    password: String,
) -> Result<TachibanaSession, TachibanaError> {
    let req = LoginRequest::new(user_id, password);
    let auth_url = format!("{}{}", base_url, AUTH_PATH);
    let url = build_api_url_from(&auth_url, &req)?;

    let resp = client.get(&url).send().await?;
    let text = resp.text().await?;
    let login_resp: LoginResponse = serde_json::from_str(&text)?;
    TachibanaSession::try_from(login_resp)
}

/// 時価情報スナップショット取得。
/// `issue_codes`: 4桁銘柄コードのスライス（最大120銘柄）。
pub async fn fetch_market_prices(
    client: &reqwest::Client,
    session: &TachibanaSession,
    issue_codes: &[&str],
) -> Result<Vec<MarketPriceRecord>, TachibanaError> {
    let req = MarketPriceRequest::new(issue_codes);
    let url = build_api_url_from(&session.url_price, &req)?;

    let resp = client.get(&url).send().await?;
    let text = resp.text().await?;
    let price_resp: MarketPriceResponse = serde_json::from_str(&text)?;
    Ok(price_resp.records)
}

/// 日足履歴取得（最大約20年分）。
pub async fn fetch_daily_history(
    client: &reqwest::Client,
    session: &TachibanaSession,
    issue_code: &str,
) -> Result<Vec<DailyHistoryRecord>, TachibanaError> {
    let req = DailyHistoryRequest::new(issue_code);
    let url = build_api_url_from(&session.url_price, &req)?;

    let resp = client.get(&url).send().await?;
    let text = resp.text().await?;
    let hist_resp: DailyHistoryResponse = serde_json::from_str(&text)?;
    Ok(hist_resp.records)
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle 1: LoginRequest シリアライズ ────────────────────────────────────

    #[test]
    fn login_request_includes_clm_id_field() {
        let req = LoginRequest::new("testuser".to_string(), "testpass".to_string());
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains(r#""sCLMID":"CLMAuthLoginRequest""#),
            "JSON に sCLMID フィールドが必要: {json}"
        );
    }

    #[test]
    fn login_request_serializes_user_credentials() {
        let req = LoginRequest::new("user123".to_string(), "secret!".to_string());
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains(r#""sUserId":"user123""#),
            "JSON: {json}"
        );
        assert!(
            json.contains(r#""sPassword":"secret!""#),
            "JSON: {json}"
        );
    }

    // ── Cycle 2: LoginResponse デシリアライズ ─────────────────────────────────

    #[test]
    fn login_response_success_deserializes_correctly() {
        let json = r#"{
            "sCLMID": "CLMAuthLoginAck",
            "sResultCode": "0",
            "sUrlRequest": "https://virtual.example.com/request/",
            "sUrlMaster": "https://virtual.example.com/master/",
            "sUrlPrice": "https://virtual.example.com/price/",
            "sUrlEvent": "https://virtual.example.com/event/",
            "sUrlEventWebSocket": "wss://virtual.example.com/event-ws/",
            "sKinsyouhouMidokuFlg": "0",
            "sResultText": ""
        }"#;
        let response: LoginResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.result_code, "0");
        assert_eq!(response.url_price, "https://virtual.example.com/price/");
        assert_eq!(
            response.url_event_ws,
            "wss://virtual.example.com/event-ws/"
        );
    }

    #[test]
    fn login_response_error_deserializes_result_code() {
        let json = r#"{
            "sCLMID": "CLMAuthLoginAck",
            "sResultCode": "10001",
            "sResultText": "認証エラー"
        }"#;
        let response: LoginResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.result_code, "10001");
        assert_eq!(response.result_text, "認証エラー");
        // 仮想URLはデフォルト空
        assert!(response.url_price.is_empty());
    }

    // ── Cycle 3: TachibanaSession 生成 ────────────────────────────────────────

    #[test]
    fn session_created_from_successful_login() {
        let response = LoginResponse {
            clm_id: "CLMAuthLoginAck".to_string(),
            result_code: "0".to_string(),
            url_request: "https://req.example.com/".to_string(),
            url_master: "https://master.example.com/".to_string(),
            url_price: "https://price.example.com/".to_string(),
            url_event: "https://event.example.com/".to_string(),
            url_event_ws: "wss://ws.example.com/".to_string(),
            unread_notice_flag: "0".to_string(),
            result_text: String::new(),
        };
        let session = TachibanaSession::try_from(response).unwrap();
        assert_eq!(session.url_price, "https://price.example.com/");
        assert_eq!(session.url_event_ws, "wss://ws.example.com/");
    }

    #[test]
    fn session_creation_fails_on_login_error() {
        let response = LoginResponse {
            clm_id: "CLMAuthLoginAck".to_string(),
            result_code: "10001".to_string(),
            url_request: String::new(),
            url_master: String::new(),
            url_price: String::new(),
            url_event: String::new(),
            url_event_ws: String::new(),
            unread_notice_flag: "0".to_string(),
            result_text: "Invalid credentials".to_string(),
        };
        let result = TachibanaSession::try_from(response);
        assert!(
            matches!(result, Err(TachibanaError::LoginFailed(_))),
            "認証エラーコードで LoginFailed が返るべき"
        );
    }

    #[test]
    fn session_creation_fails_when_unread_notices_flag_set() {
        let response = LoginResponse {
            clm_id: "CLMAuthLoginAck".to_string(),
            result_code: "0".to_string(),
            // 未読書面があると仮想URLが空になる
            url_request: String::new(),
            url_master: String::new(),
            url_price: String::new(),
            url_event: String::new(),
            url_event_ws: String::new(),
            unread_notice_flag: "1".to_string(),
            result_text: String::new(),
        };
        let result = TachibanaSession::try_from(response);
        assert!(
            matches!(result, Err(TachibanaError::UnreadNotices)),
            "未読書面フラグが '1' の場合 UnreadNotices エラーが返るべき"
        );
    }

    // ── Cycle 4: URL 構築 ─────────────────────────────────────────────────────

    #[test]
    fn build_api_url_appends_json_after_question_mark() {
        let base = "https://kabuka.e-shiten.jp/e_api_v4r8/auth/";
        let json = r#"{"sCLMID":"CLMAuthLoginRequest"}"#;
        let url = build_api_url(base, json);
        assert_eq!(url, format!("{base}?{json}"));
    }

    #[test]
    fn build_api_url_from_serializes_request_into_url() {
        let req = LoginRequest::new("user".to_string(), "pass".to_string());
        let base = "https://kabuka.e-shiten.jp/e_api_v4r8/auth/";
        let url = build_api_url_from(base, &req).unwrap();
        assert!(url.starts_with(base), "URL はベース URL で始まるべき: {url}");
        assert!(
            url.contains("CLMAuthLoginRequest"),
            "URL に CLMAuthLoginRequest が含まれるべき: {url}"
        );
        assert!(url.contains("user"), "URL にユーザーIDが含まれるべき: {url}");
    }

    // ── Cycle 5: MarketPriceRequest シリアライズ ──────────────────────────────

    #[test]
    fn market_price_request_serializes_clm_id() {
        let req = MarketPriceRequest::new(&["6501", "7203"]);
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains(r#""sCLMID":"CLMMfdsGetMarketPrice""#),
            "JSON: {json}"
        );
    }

    #[test]
    fn market_price_request_joins_issue_codes_with_comma() {
        let req = MarketPriceRequest::new(&["6501", "7203", "9984"]);
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains(r#""sTargetIssueCode":"6501,7203,9984""#),
            "JSON: {json}"
        );
    }

    // ── Cycle 6: MarketPriceResponse デシリアライズ ───────────────────────────

    #[test]
    fn market_price_response_deserializes_single_record() {
        let json = r#"{
            "aCLMMfdsMarketPrice": [
                {
                    "sIssueCode": "6501",
                    "pDPP": "3250",
                    "pDOP": "3200",
                    "pDHP": "3280",
                    "pDLP": "3195",
                    "pDV": "1500000",
                    "pPRP": "3220"
                }
            ]
        }"#;
        let response: MarketPriceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.records.len(), 1);
        let record = &response.records[0];
        assert_eq!(record.issue_code, "6501");
        assert_eq!(record.current_price, "3250");
        assert_eq!(record.open, "3200");
        assert_eq!(record.high, "3280");
        assert_eq!(record.low, "3195");
        assert_eq!(record.volume, "1500000");
        assert_eq!(record.prev_close, "3220");
    }

    #[test]
    fn market_price_response_deserializes_multiple_records() {
        let json = r#"{
            "aCLMMfdsMarketPrice": [
                {"sIssueCode": "6501", "pDPP": "3250"},
                {"sIssueCode": "7203", "pDPP": "2100"},
                {"sIssueCode": "9984", "pDPP": "8500"}
            ]
        }"#;
        let response: MarketPriceResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.records.len(), 3);
        assert_eq!(response.records[0].issue_code, "6501");
        assert_eq!(response.records[2].current_price, "8500");
    }

    // ── Cycle 7: DailyHistoryRequest シリアライズ ─────────────────────────────

    #[test]
    fn daily_history_request_serializes_clm_id() {
        let req = DailyHistoryRequest::new("6501");
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains(r#""sCLMID":"CLMMfdsGetMarketPriceHistory""#),
            "JSON: {json}"
        );
    }

    #[test]
    fn daily_history_request_serializes_issue_code_and_market() {
        let req = DailyHistoryRequest::new("6501");
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""sIssueCode":"6501""#), "JSON: {json}");
        assert!(json.contains(r#""sSizyouC":"00""#), "JSON: {json}");
    }

    // ── Cycle 8: DailyHistoryResponse デシリアライズ ──────────────────────────

    #[test]
    fn daily_history_response_deserializes_ohlcv() {
        let json = r#"{
            "aCLMMfdsGetMarketPriceHistory": [
                {
                    "sDate": "20240101",
                    "pDOP": "3200",
                    "pDHP": "3280",
                    "pDLP": "3150",
                    "pDPP": "3250",
                    "pDV": "1500000"
                }
            ]
        }"#;
        let response: DailyHistoryResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.records.len(), 1);
        let record = &response.records[0];
        assert_eq!(record.date, "20240101");
        assert_eq!(record.open, "3200");
        assert_eq!(record.high, "3280");
        assert_eq!(record.low, "3150");
        assert_eq!(record.close, "3250");
        assert_eq!(record.volume, "1500000");
    }

    #[test]
    fn daily_history_response_deserializes_split_adjusted_values() {
        let json = r#"{
            "aCLMMfdsGetMarketPriceHistory": [
                {
                    "sDate": "20200101",
                    "pDOP": "6400",
                    "pDHP": "6560",
                    "pDLP": "6300",
                    "pDPP": "6500",
                    "pDV": "750000",
                    "pDOPxK": "3200",
                    "pDHPxK": "3280",
                    "pDLPxK": "3150",
                    "pDPPxK": "3250",
                    "pDVxK": "1500000"
                }
            ]
        }"#;
        let response: DailyHistoryResponse = serde_json::from_str(json).unwrap();
        let record = &response.records[0];
        // 生値
        assert_eq!(record.open, "6400");
        assert_eq!(record.volume, "750000");
        // 分割調整値（株式分割後の調整値）
        assert_eq!(record.open_adj, "3200");
        assert_eq!(record.close_adj, "3250");
        assert_eq!(record.volume_adj, "1500000");
    }

    // ── Cycle 9: HTTP クライアント (mockito) ──────────────────────────────────

    #[tokio::test]
    async fn login_returns_session_on_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "0",
                    "sUrlRequest": "https://virtual.example.com/request/",
                    "sUrlMaster": "https://virtual.example.com/master/",
                    "sUrlPrice": "https://virtual.example.com/price/",
                    "sUrlEvent": "https://virtual.example.com/event/",
                    "sUrlEventWebSocket": "wss://virtual.example.com/ws/",
                    "sKinsyouhouMidokuFlg": "0",
                    "sResultText": ""
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let base_url = format!("{}/", server.url());
        let session = login(
            &client,
            &base_url,
            "testuser".to_string(),
            "testpass".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(session.url_price, "https://virtual.example.com/price/");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn login_returns_error_on_auth_failure() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "10001",
                    "sResultText": "ユーザIDまたはパスワードが違います"
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let base_url = format!("{}/", server.url());
        let result = login(
            &client,
            &base_url,
            "wronguser".to_string(),
            "wrongpass".to_string(),
        )
        .await;

        assert!(
            matches!(result, Err(TachibanaError::LoginFailed(_))),
            "認証失敗時は LoginFailed が返るべき: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn fetch_market_prices_returns_records() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "aCLMMfdsMarketPrice": [
                        {"sIssueCode": "6501", "pDPP": "3250", "pDOP": "3200",
                         "pDHP": "3280", "pDLP": "3195", "pDV": "1500000", "pPRP": "3220"}
                    ]
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let session = TachibanaSession {
            url_request: String::new(),
            url_master: String::new(),
            url_price: format!("{}/price/", server.url()),
            url_event: String::new(),
            url_event_ws: String::new(),
        };

        let records = fetch_market_prices(&client, &session, &["6501"])
            .await
            .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].issue_code, "6501");
        assert_eq!(records[0].current_price, "3250");
    }

    #[tokio::test]
    async fn fetch_daily_history_returns_candles() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "aCLMMfdsGetMarketPriceHistory": [
                        {"sDate":"20240101","pDOP":"3200","pDHP":"3280","pDLP":"3150","pDPP":"3250","pDV":"1500000"},
                        {"sDate":"20240102","pDOP":"3250","pDHP":"3300","pDLP":"3230","pDPP":"3280","pDV":"1200000"}
                    ]
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let session = TachibanaSession {
            url_request: String::new(),
            url_master: String::new(),
            url_price: format!("{}/price/", server.url()),
            url_event: String::new(),
            url_event_ws: String::new(),
        };

        let records = fetch_daily_history(&client, &session, "6501")
            .await
            .unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].date, "20240101");
        assert_eq!(records[1].date, "20240102");
        assert_eq!(records[1].close, "3280");
    }
}
