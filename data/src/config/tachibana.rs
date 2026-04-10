use exchange::adapter::tachibana::TachibanaSession;

const KEYCHAIN_SERVICE: &str = "flowsurface.tachibana";
const KEYCHAIN_KEY: &str = "session";

/// keyring からセッションを読み込む。
pub fn load_session() -> Option<TachibanaSession> {
    let entry = match keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("Tachibana session keyring entry init failed: {e}");
            return None;
        }
    };
    let secret = match entry.get_password() {
        Ok(s) => s,
        Err(e) => {
            log::info!("No tachibana session in keyring: {e}");
            return None;
        }
    };
    match serde_json::from_str(&secret) {
        Ok(session) => {
            log::info!("Loaded tachibana session from keyring");
            Some(session)
        }
        Err(e) => {
            log::warn!("Tachibana session in keyring is invalid JSON: {e}");
            None
        }
    }
}

/// keyring にセッションを保存する。
pub fn save_session(session: &TachibanaSession) {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY) else {
        log::warn!("Tachibana session keyring entry init failed on save");
        return;
    };
    let Ok(json) = serde_json::to_string(session) else {
        log::warn!("Failed to serialize tachibana session");
        return;
    };
    match entry.set_password(&json) {
        Ok(()) => log::info!("Saved tachibana session to keyring"),
        Err(e) => log::warn!("Failed to save tachibana session to keyring: {e}"),
    }
}

/// keyring からセッションを削除する。
pub fn delete_session() {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY) else {
        return;
    };
    let _ = entry.delete_credential();
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle K1: save → load ラウンドトリップ ──────────────────────────────

    #[test]
    fn save_then_load_returns_same_session() {
        let session = TachibanaSession {
            url_request: "https://req.test/".to_string(),
            url_master: "https://master.test/".to_string(),
            url_price: "https://price.test/".to_string(),
            url_event: "https://event.test/".to_string(),
            url_event_ws: "wss://ws.test/".to_string(),
        };

        save_session(&session);
        let loaded = load_session().expect("保存後は load できるべき");

        assert_eq!(session.url_request, loaded.url_request);
        assert_eq!(session.url_master, loaded.url_master);
        assert_eq!(session.url_price, loaded.url_price);
        assert_eq!(session.url_event, loaded.url_event);
        assert_eq!(session.url_event_ws, loaded.url_event_ws);

        // クリーンアップ
        delete_session();
    }

    // ── Cycle K2: delete 後は load が None を返す ───────────────────────────

    #[test]
    fn load_returns_none_after_delete() {
        let session = TachibanaSession {
            url_request: "https://req.test/".to_string(),
            url_master: "https://master.test/".to_string(),
            url_price: "https://price.test/".to_string(),
            url_event: "https://event.test/".to_string(),
            url_event_ws: "wss://ws.test/".to_string(),
        };

        save_session(&session);
        delete_session();
        assert!(load_session().is_none(), "delete 後は None であるべき");
    }

    // ── Cycle K3: 未保存時は load が None を返す ────────────────────────────

    #[test]
    fn load_returns_none_when_nothing_saved() {
        delete_session(); // 念のためクリア
        assert!(load_session().is_none(), "未保存時は None であるべき");
    }
}
