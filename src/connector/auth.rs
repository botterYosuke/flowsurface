use exchange::adapter::tachibana::{
    self, TachibanaError, TachibanaSession, BASE_URL_DEMO, BASE_URL_PROD,
};
use std::sync::RwLock;

static SESSION: RwLock<Option<TachibanaSession>> = RwLock::new(None);

/// デモ/本番の BASE URL を返す。
pub fn base_url(is_demo: bool) -> &'static str {
    if is_demo { BASE_URL_DEMO } else { BASE_URL_PROD }
}

/// 保存済みセッションを取得する。
pub fn get_session() -> Option<TachibanaSession> {
    SESSION.read().ok()?.clone()
}

/// セッションを保存する。
pub fn store_session(session: TachibanaSession) {
    if let Ok(mut guard) = SESSION.write() {
        *guard = Some(session);
    }
}

/// セッションをクリアする。
pub fn clear_session() {
    if let Ok(mut guard) = SESSION.write() {
        *guard = None;
    }
}

/// ログイン実行。Task::perform から呼び出される非同期関数。
/// 成功時は TachibanaSession を返し、失敗時はユーザー向けエラーメッセージを返す。
pub async fn perform_login(
    user_id: String,
    password: String,
    is_demo: bool,
) -> Result<TachibanaSession, String> {
    let base = base_url(is_demo);
    perform_login_with_base_url(base, user_id, password).await
}

/// テスト可能なログイン実装（base_url を引数で受け取る）。
pub async fn perform_login_with_base_url(
    base_url: &str,
    user_id: String,
    password: String,
) -> Result<TachibanaSession, String> {
    let client = reqwest::Client::new();
    let session = exchange::adapter::tachibana::login(&client, base_url, user_id, password)
        .await
        .map_err(tachibana_error_to_message)?;

    // ログイン完了前に銘柄マスタをダウンロードしキャッシュに格納する。
    // ダッシュボード初期化時の fetch_ticker_metadata で参照されるため、
    // spawn ではなく await で完了を待つ必要がある。
    let client_for_master = reqwest::Client::new();
    if let Err(e) = tachibana::init_issue_master(&client_for_master, &session).await {
        log::error!("Tachibana master download failed: {e}");
    }

    Ok(session)
}

/// TachibanaError をユーザー向けメッセージに変換する。
fn tachibana_error_to_message(err: TachibanaError) -> String {
    use crate::screen::login::tachibana_error_message;
    match &err {
        TachibanaError::UnreadNotices => {
            tachibana_error_message("UNREAD_NOTICES").to_string()
        }
        TachibanaError::LoginFailed(msg) => {
            // "code=10001, message=..." 形式からコードを抽出
            if let Some(code) = msg.strip_prefix("code=").and_then(|s| s.split(',').next()) {
                tachibana_error_message(code).to_string()
            } else {
                tachibana_error_message("").to_string()
            }
        }
        _ => err.to_string(),
    }
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle C1: base_url はデモ/本番URLを返す ─────────────────────────────

    #[test]
    fn base_url_returns_demo_url_when_is_demo_true() {
        assert_eq!(base_url(true), BASE_URL_DEMO);
    }

    #[test]
    fn base_url_returns_prod_url_when_is_demo_false() {
        assert_eq!(base_url(false), BASE_URL_PROD);
    }

    // ── Cycle C2: セッション保存と取得 ──────────────────────────────────────

    #[test]
    fn get_session_returns_none_when_no_session_stored() {
        clear_session();
        assert!(get_session().is_none(), "セッション未保存時は None であるべき");
    }

    #[test]
    fn store_session_makes_get_session_return_stored_value() {
        clear_session();
        let session = TachibanaSession {
            url_request: "https://req.test/".to_string(),
            url_master: "https://master.test/".to_string(),
            url_price: "https://price.test/".to_string(),
            url_event: "https://event.test/".to_string(),
            url_event_ws: "wss://ws.test/".to_string(),
        };
        store_session(session);
        let retrieved = get_session().expect("セッションが取得できるべき");
        assert_eq!(retrieved.url_price, "https://price.test/");
        clear_session();
    }

    // ── Cycle C3: セッションクリア ──────────────────────────────────────────

    #[test]
    fn clear_session_removes_stored_session() {
        let session = TachibanaSession {
            url_request: "https://req.test/".to_string(),
            url_master: "https://master.test/".to_string(),
            url_price: "https://price.test/".to_string(),
            url_event: "https://event.test/".to_string(),
            url_event_ws: "wss://ws.test/".to_string(),
        };
        store_session(session);
        clear_session();
        assert!(get_session().is_none(), "clear 後は None であるべき");
    }

    // ── Cycle C4: perform_login 成功 ────────────────────────────────────────

    #[tokio::test]
    async fn perform_login_returns_session_on_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "0",
                    "sUrlRequest": "https://virtual.test/request/",
                    "sUrlMaster": "https://virtual.test/master/",
                    "sUrlPrice": "https://virtual.test/price/",
                    "sUrlEvent": "https://virtual.test/event/",
                    "sUrlEventWebSocket": "wss://virtual.test/ws/",
                    "sKinsyouhouMidokuFlg": "0",
                    "sResultText": ""
                }"#,
            )
            .create_async()
            .await;

        let base = format!("{}/", server.url());
        let result = perform_login_with_base_url(
            &base,
            "testuser".to_string(),
            "testpass".to_string(),
        )
        .await;

        let session = result.expect("ログイン成功でセッションが返るべき");
        assert_eq!(session.url_price, "https://virtual.test/price/");
    }

    // ── Cycle C5: perform_login 認証失敗 ────────────────────────────────────

    #[tokio::test]
    async fn perform_login_returns_user_facing_error_on_auth_failure() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "10001",
                    "sResultText": "認証エラー"
                }"#,
            )
            .create_async()
            .await;

        let base = format!("{}/", server.url());
        let result = perform_login_with_base_url(
            &base,
            "wrong".to_string(),
            "wrong".to_string(),
        )
        .await;

        let err = result.expect_err("認証失敗でエラーが返るべき");
        assert!(
            err.contains("ユーザID") || err.contains("パスワード"),
            "認証エラーメッセージにユーザIDまたはパスワードが含まれるべき: {err}"
        );
    }

    // ── Cycle C6: perform_login 未読書面エラー ──────────────────────────────

    #[tokio::test]
    async fn perform_login_returns_unread_notices_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "0",
                    "sUrlRequest": "",
                    "sUrlMaster": "",
                    "sUrlPrice": "",
                    "sUrlEvent": "",
                    "sUrlEventWebSocket": "",
                    "sKinsyouhouMidokuFlg": "1",
                    "sResultText": ""
                }"#,
            )
            .create_async()
            .await;

        let base = format!("{}/", server.url());
        let result = perform_login_with_base_url(
            &base,
            "user".to_string(),
            "pass".to_string(),
        )
        .await;

        let err = result.expect_err("未読書面エラーが返るべき");
        assert!(
            err.contains("書面") || err.contains("未読"),
            "未読書面のメッセージであるべき: {err}"
        );
    }
}
