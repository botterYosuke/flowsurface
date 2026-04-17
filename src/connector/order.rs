use exchange::adapter::tachibana::{
    CancelOrderRequest, CorrectOrderRequest, ExecutionRecord, ModifyOrderResponse, NewOrderRequest,
    NewOrderResponse, OrderRecord, TachibanaError,
};

use super::auth::get_session;

// ── 内部ヘルパー ───────────────────────────────────────────────────────────────

/// セッションと HTTP クライアントを取得する。
/// セッションが存在しない場合は `Err` を返す。
fn session_and_client() -> Result<
    (
        exchange::adapter::tachibana::TachibanaSession,
        reqwest::Client,
    ),
    TachibanaError,
> {
    let session = get_session().ok_or_else(|| TachibanaError::ApiError {
        code: "NO_SESSION".to_string(),
        message: "セッションが存在しません".to_string(),
    })?;
    let client = reqwest::Client::new();
    Ok((session, client))
}

// ── 注文 API 関数 ─────────────────────────────────────────────────────────────

/// CLMKabuNewOrder — 新規注文を発注する。
/// `dashboard.rs` の `Task::perform` から呼び出す。
pub async fn submit_new_order(req: NewOrderRequest) -> Result<NewOrderResponse, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::submit_new_order(&client, &session, &req)
        .await
        .map_err(|e| e.to_string())
}

/// CLMKabuCorrectOrder — 訂正注文を発注する。
pub async fn submit_correct_order(req: CorrectOrderRequest) -> Result<ModifyOrderResponse, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::submit_correct_order(&client, &session, &req)
        .await
        .map_err(|e| e.to_string())
}

/// CLMKabuCancelOrder — 取消注文を発注する。
pub async fn submit_cancel_order(req: CancelOrderRequest) -> Result<ModifyOrderResponse, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::submit_cancel_order(&client, &session, &req)
        .await
        .map_err(|e| e.to_string())
}

/// CLMOrderList — 注文一覧を取得する。
/// `eig_day`: 執行予定日 (YYYYMMDD)。空文字 = 今日。
pub async fn fetch_orders(eig_day: String) -> Result<Vec<OrderRecord>, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::fetch_orders(&client, &session, &eig_day)
        .await
        .map_err(|e| e.to_string())
}

/// CLMOrderListDetail — 約定明細を取得する。
pub async fn fetch_order_detail(
    order_num: String,
    eig_day: String,
) -> Result<Vec<ExecutionRecord>, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::fetch_order_detail(&client, &session, &order_num, &eig_day)
        .await
        .map_err(|e| e.to_string())
}

/// CLMZanKaiKanougaku + CLMZanShinkiKanoIjiritu — 余力情報を取得する。
/// 現物余力と信用余力を並列取得して BuyingPowerPanel へ渡す。
pub async fn fetch_buying_power() -> Result<
    (
        exchange::adapter::tachibana::BuyingPowerResponse,
        exchange::adapter::tachibana::MarginPowerResponse,
    ),
    String,
> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    let (cash_result, margin_result) = tokio::join!(
        exchange::adapter::tachibana::fetch_buying_power(&client, &session),
        exchange::adapter::tachibana::fetch_margin_power(&client, &session),
    );
    Ok((
        cash_result.map_err(|e| e.to_string())?,
        margin_result.map_err(|e| e.to_string())?,
    ))
}

/// CLMGenbutuKabuList — 保有株数（売付可能株数）を取得する。
pub async fn fetch_holdings(issue_code: String) -> Result<u64, String> {
    let (session, client) = session_and_client().map_err(|e| e.to_string())?;
    exchange::adapter::tachibana::fetch_holdings(&client, &session, &issue_code)
        .await
        .map_err(|e| e.to_string())
}
