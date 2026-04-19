use exchange::adapter::tachibana::{
    BASE_URL_DEMO, BASE_URL_PROD, TachibanaError, TachibanaSession,
};
use std::sync::RwLock;

static SESSION: RwLock<Option<TachibanaSession>> = RwLock::new(None);

/// デモ/本番の BASE URL を返す。
pub fn base_url(is_demo: bool) -> &'static str {
    if is_demo {
        BASE_URL_DEMO
    } else {
        BASE_URL_PROD
    }
}

/// 保存済みセッションを取得する。
pub fn get_session() -> Option<TachibanaSession> {
    SESSION.read().ok()?.clone()
}

/// セッションを保存する。
/// EVENT I/F WebSocket URL も exchange crate 側に設定する。
pub fn store_session(session: TachibanaSession) {
    exchange::adapter::tachibana::set_event_ws_url(session.url_event_ws.clone());
    exchange::adapter::tachibana::set_event_http_url(session.url_event.clone());
    if let Ok(mut guard) = SESSION.write() {
        *guard = Some(session);
    }
}

/// セッションをクリアする（メモリ + keyring）。
/// 現時点ではログアウト機能未実装のため未使用。
#[allow(dead_code)]
pub fn clear_session() {
    if let Ok(mut guard) = SESSION.write() {
        *guard = None;
    }
    data::config::tachibana::delete_session();
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

    Ok(session)
}

/// ログイン成功後にセッションを keyring に永続化する。
pub fn persist_session(session: &TachibanaSession) {
    data::config::tachibana::save_session(session);
}

/// keyring から保存済みセッションを復元し、有効性を検証する。
/// 有効なセッションがあれば返す。失効/未保存なら None。
pub async fn try_restore_session() -> Option<TachibanaSession> {
    log::info!("Attempting to restore tachibana session from keyring");
    let session = match data::config::tachibana::load_session() {
        Some(s) => s,
        None => {
            log::info!("No saved tachibana session found in keyring");
            return None;
        }
    };

    let client = reqwest::Client::new();
    match exchange::adapter::tachibana::validate_session(&client, &session).await {
        Ok(()) => {
            log::info!("Tachibana session validated successfully, restoring");
            Some(session)
        }
        Err(e) => {
            log::warn!("Tachibana session restore failed: {e}");
            data::config::tachibana::delete_session();
            // CI / 開発環境: keyring validation 失敗後に DEV_USER_ID/DEV_PASSWORD で自動再ログイン。
            // 本番環境では env vars が未設定のため None を返してフォールスルーする。
            let user_id = std::env::var("DEV_USER_ID").ok()?;
            let password = std::env::var("DEV_PASSWORD").ok()?;
            let is_demo = std::env::var("DEV_IS_DEMO")
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .unwrap_or(false);
            log::info!("Falling back to DEV_USER_ID/DEV_PASSWORD re-login (is_demo={is_demo})");
            match perform_login(user_id, password, is_demo).await {
                Ok(session) => {
                    persist_session(&session);
                    Some(session)
                }
                Err(login_err) => {
                    log::warn!("Tachibana re-login also failed: {login_err}");
                    None
                }
            }
        }
    }
}

/// E2E テスト用（Phase T3）: メモリセッションと keyring セッションを両方クリアする。
/// テスト間のクリーンアップや「セッション未存在」状態の確認に使用。
/// debug ビルドで有効（release ビルドには含まれない）。
#[cfg(debug_assertions)]
pub fn delete_all_sessions() {
    clear_session();
    data::config::tachibana::delete_session();
    log::info!("Tachibana: all sessions cleared (memory + keyring)");
}

/// TachibanaError をユーザー向けメッセージに変換する。
fn tachibana_error_to_message(err: TachibanaError) -> String {
    use crate::screen::login::tachibana_error_message;
    match &err {
        TachibanaError::UnreadNotices => tachibana_error_message("UNREAD_NOTICES").to_string(),
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
        assert!(
            get_session().is_none(),
            "セッション未保存時は None であるべき"
        );
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

    // ── Cycle P1: persist_session → keyring に保存される ──────────────────

    #[test]
    fn persist_session_saves_to_keyring() {
        let session = TachibanaSession {
            url_request: "https://persist.test/request/".to_string(),
            url_master: "https://persist.test/master/".to_string(),
            url_price: "https://persist.test/price/".to_string(),
            url_event: "https://persist.test/event/".to_string(),
            url_event_ws: "wss://persist.test/ws/".to_string(),
        };

        persist_session(&session);
        let loaded = data::config::tachibana::load_session()
            .expect("persist 後は keyring から load できるべき");
        assert_eq!(session.url_price, loaded.url_price);

        // クリーンアップ
        data::config::tachibana::delete_session();
    }

    // ── Cycle C4: perform_login 成功 ────────────────────────────────────────

    #[tokio::test]
    async fn perform_login_returns_session_on_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
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
        let result =
            perform_login_with_base_url(&base, "testuser".to_string(), "testpass".to_string())
                .await;

        let session = result.expect("ログイン成功でセッションが返るべき");
        assert_eq!(session.url_price, "https://virtual.test/price/");
    }

    // ── Cycle C5: perform_login 認証失敗 ────────────────────────────────────

    #[tokio::test]
    async fn perform_login_returns_user_facing_error_on_auth_failure() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
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
        let result =
            perform_login_with_base_url(&base, "wrong".to_string(), "wrong".to_string()).await;

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
            .mock("POST", mockito::Matcher::Any)
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
        let result =
            perform_login_with_base_url(&base, "user".to_string(), "pass".to_string()).await;

        let err = result.expect_err("未読書面エラーが返るべき");
        assert!(
            err.contains("書面") || err.contains("未読"),
            "未読書面のメッセージであるべき: {err}"
        );
    }
}
