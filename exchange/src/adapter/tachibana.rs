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
use std::sync::atomic::{AtomicU64, Ordering};

/// リクエスト通番カウンタ。全リクエストで共有し、インクリメントする。
/// 初期値は起動時のUnix秒を使用し、セッション復元時に前回の値を常に超える。
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 次のリクエスト通番を生成する。
/// 初回呼び出し時にタイムスタンプベースで初期化される。
/// `compare_exchange` で初期化を排他し、複数スレッドが同時に呼んでも安全。
pub fn next_p_no() -> String {
    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // CAS: カウンタが 0（未初期化）の場合のみ epoch_secs で初期化。
    // 複数スレッドが同時に呼んでも 1 つだけが成功し、残りは失敗して既存値を使う。
    let _ = REQUEST_COUNTER.compare_exchange(0, epoch_secs, Ordering::Relaxed, Ordering::Relaxed);
    let val = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    val.to_string()
}

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
///
/// `sJsonOfmt: "5"` は必須。これがないと応答が数値キー形式になる。
/// "5" = bit1 ON（見やすい形式）+ bit3 ON（引数項目名称で応答）。
#[derive(Debug, Serialize)]
pub struct LoginRequest {
    /// リクエスト通番（ログイン時は "1"）
    pub p_no: String,
    /// リクエスト送信日時 (YYYY.MM.DD-hh:mm:ss.sss)
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    #[serde(rename = "sUserId")]
    pub user_id: String,
    #[serde(rename = "sPassword")]
    pub password: String,
    /// 応答の表示形式。"5" = 項目名称付きJSON
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,
}

impl LoginRequest {
    pub fn new(user_id: String, password: String) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMAuthLoginRequest",
            user_id,
            password,
            json_ofmt: "5",
        }
    }
}

/// 現在時刻を p_sd_date 形式 (YYYY.MM.DD-hh:mm:ss.sss) で返す。
fn current_p_sd_date() -> String {
    let now = chrono::Local::now();
    now.format("%Y.%m.%d-%H:%M:%S%.3f").to_string()
}

/// CLMAuthLoginAck 応答。
/// sResultCode が "0" 以外はエラー。
/// sKinsyouhouMidokuFlg が "1" の場合、仮想URLは空で利用不可。
#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    #[serde(rename = "sCLMID", default)]
    pub clm_id: String,
    /// API共通エラーコード。"0" = 正常。
    #[serde(default)]
    pub p_errno: String,
    /// API共通エラーメッセージ。
    #[serde(default)]
    pub p_err: String,
    #[serde(rename = "sResultCode", default)]
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
        // p_errno チェック（API共通エラー）
        if !resp.p_errno.is_empty() && resp.p_errno != "0" {
            return Err(TachibanaError::LoginFailed(format!(
                "code={}, p_err={}",
                resp.p_errno, resp.p_err
            )));
        }
        // sResultCode チェック
        if !resp.result_code.is_empty() && resp.result_code != "0" {
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

/// リクエスト構造体を JSON にシリアライズする。
fn serialize_request<T: Serialize>(request: &T) -> Result<String, TachibanaError> {
    Ok(serde_json::to_string(request)?)
}

/// POST リクエストを送信し、Shift-JIS デコードしたレスポンスボディを返す。
async fn post_request(
    client: &reqwest::Client,
    url: &str,
    json_body: &str,
) -> Result<String, TachibanaError> {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(json_body.to_string())
        .send()
        .await?;
    decode_response_body(resp).await
}

// ── 時価情報型 ────────────────────────────────────────────────────────────────

/// CLMMfdsGetMarketPrice リクエスト（スナップショット取得）。
/// 最大120銘柄まで同時取得可能。
#[derive(Debug, Serialize)]
pub struct MarketPriceRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    /// カンマ区切りの銘柄コード (例: "6501,7203")
    #[serde(rename = "sTargetIssueCode")]
    pub target_issue_codes: String,
    /// カンマ区切りの情報コード
    #[serde(rename = "sTargetColumn")]
    pub target_columns: String,
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,
}

impl MarketPriceRequest {
    /// デフォルトの情報コード（現在値・四本値・出来高・前日終値）
    pub const DEFAULT_COLUMNS: &'static str = "pDPP,pDOP,pDHP,pDLP,pDV,pPRP";

    pub fn new(issue_codes: &[&str]) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMMfdsGetMarketPrice",
            target_issue_codes: issue_codes.join(","),
            target_columns: Self::DEFAULT_COLUMNS.to_string(),
            json_ofmt: "5",
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
    #[serde(rename = "aCLMMfdsMarketPrice", default)]
    pub records: Vec<MarketPriceRecord>,
}

// ── 日足履歴型 ────────────────────────────────────────────────────────────────

/// CLMMfdsGetMarketPriceHistory リクエスト（日足履歴取得）。
/// 1リクエスト1銘柄、最大約20年分のデータを取得可能。
#[derive(Debug, Serialize)]
pub struct DailyHistoryRequest {
    pub p_no: String,
    pub p_sd_date: String,
    #[serde(rename = "sCLMID")]
    pub clm_id: &'static str,
    #[serde(rename = "sIssueCode")]
    pub issue_code: String,
    /// 市場コード (東証: "00")
    #[serde(rename = "sSizyouC")]
    pub market_code: String,
    #[serde(rename = "sJsonOfmt")]
    pub json_ofmt: &'static str,
}

impl DailyHistoryRequest {
    /// 東証の市場コード
    pub const TSE_MARKET_CODE: &'static str = "00";

    pub fn new(issue_code: &str) -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMMfdsGetMarketPriceHistory",
            issue_code: issue_code.to_string(),
            market_code: Self::TSE_MARKET_CODE.to_string(),
            json_ofmt: "5",
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
    #[serde(rename = "aCLMMfdsMarketPriceHistory", default)]
    pub records: Vec<DailyHistoryRecord>,
}

// ── 共通レスポンスラッパー ───────────────────────────────────────────────────

/// 業務 API レスポンスの共通ラッパー。
/// `p_errno` / `sResultCode` でエラーチェックを行う。
#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    #[serde(default)]
    pub p_errno: String,
    #[serde(default)]
    pub p_err: String,
    #[serde(rename = "sResultCode", default)]
    pub result_code: String,
    #[serde(rename = "sResultText", default)]
    pub result_text: String,
    #[serde(flatten)]
    pub data: T,
}

impl<T> ApiResponse<T> {
    /// エラーチェックを行い、正常時はデータを返す。
    pub fn check(self) -> Result<T, TachibanaError> {
        if !self.p_errno.is_empty() && self.p_errno != "0" {
            return Err(TachibanaError::ApiError {
                code: self.p_errno,
                message: self.p_err,
            });
        }
        if !self.result_code.is_empty() && self.result_code != "0" {
            return Err(TachibanaError::ApiError {
                code: self.result_code,
                message: self.result_text,
            });
        }
        Ok(self.data)
    }
}

// ── HTTP クライアント ─────────────────────────────────────────────────────────

/// 立花証券 API の BASE URL（本番）
pub const BASE_URL_PROD: &str = "https://kabuka.e-shiten.jp/e_api_v4r8/";

/// 立花証券 API の BASE URL（デモ）
pub const BASE_URL_DEMO: &str = "https://demo-kabuka.e-shiten.jp/e_api_v4r8/";

/// 認証エンドポイントのパス
pub const AUTH_PATH: &str = "auth/";

/// レスポンスボディを Shift-JIS からデコードする。
/// 立花証券 API のレスポンスは Shift-JIS エンコーディング。
async fn decode_response_body(resp: reqwest::Response) -> Result<String, TachibanaError> {
    let bytes = resp.bytes().await?;
    let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.decode(&bytes);
    if had_errors {
        log::warn!(
            "Shift-JIS decode produced lossy output ({} bytes)",
            bytes.len()
        );
    }
    Ok(cow.into_owned())
}

/// ログイン処理。
/// 成功時は `TachibanaSession` を返す。
/// 未読書面がある場合は `TachibanaError::UnreadNotices`。
pub async fn login(
    client: &reqwest::Client,
    base_url: &str,
    user_id: String,
    password: String,
) -> Result<TachibanaSession, TachibanaError> {
    let encoded_password = urlencoding::encode(&password).into_owned();
    let req = LoginRequest::new(user_id, encoded_password);
    let auth_url = format!("{}{}", base_url, AUTH_PATH);
    let json_body = serialize_request(&req)?;

    log::debug!("Tachibana login URL: {auth_url}");

    let text = post_request(client, &auth_url, &json_body).await?;

    log::debug!("Tachibana login response: {text}");

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
    // E2E テスト: mock 経路。feature 有効かつ mock データが注入されている場合、
    // ネットワークを叩かず mock データをそのまま返す。
    #[cfg(feature = "e2e-mock")]
    if let Some(mock) = e2e_mock::get_mock_market_prices() {
        log::info!(
            "Tachibana [e2e-mock]: returned {} market price records",
            mock.len()
        );
        return Ok(mock);
    }

    let req = MarketPriceRequest::new(issue_codes);
    let json_body = serialize_request(&req)?;

    let text = post_request(client, &session.url_price, &json_body).await?;
    let api_resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(&text)?;
    let data = api_resp.check()?;
    Ok(data.records)
}

/// 保存済みセッションの仮想URLがまだ有効か確認する。
/// url_price に対して軽量リクエストを送り、p_errno で判定する。
/// 有効なら Ok(()), 失効（p_errno="2"）なら Err を返す。
pub async fn validate_session(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(), TachibanaError> {
    log::debug!(
        "Validating tachibana session: url_price={}",
        session.url_price
    );
    let req = MarketPriceRequest::new(&["0000"]);
    let json_body = serialize_request(&req)?;
    let text = post_request(client, &session.url_price, &json_body).await?;
    log::debug!(
        "validate_session response: {}",
        &text[..text.len().min(500)]
    );
    let api_resp: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
    // 許可リスト: p_errno が "0" または空文字のみ有効。
    // "2" はセッション失効、それ以外の未知コードもエラーとして扱う。
    match api_resp.p_errno.as_str() {
        "0" | "" => Ok(()),
        other => {
            log::warn!(
                "validate_session: p_errno={}, p_err={}",
                other,
                api_resp.p_err,
            );
            Err(TachibanaError::ApiError {
                code: api_resp.p_errno,
                message: api_resp.p_err,
            })
        }
    }
}

/// 日足履歴取得（最大約20年分）。
pub async fn fetch_daily_history(
    client: &reqwest::Client,
    session: &TachibanaSession,
    issue_code: &str,
) -> Result<Vec<DailyHistoryRecord>, TachibanaError> {
    let req = DailyHistoryRequest::new(issue_code);
    let json_body = serialize_request(&req)?;

    let text = post_request(client, &session.url_price, &json_body).await?;
    let api_resp: ApiResponse<DailyHistoryResponse> = serde_json::from_str(&text)?;
    let data = api_resp.check()?;
    Ok(data.records)
}

// ── 日足データ変換 ────────────────────────────────────────────────────────────

use crate::unit::{MinTicksize, qty::Qty};
use crate::{Kline, Volume};

/// `DailyHistoryRecord` を `Kline` に変換する。
///
/// - `use_adjusted`: `true` のとき株式分割調整値（`*xK` フィールド）を使用。
/// - OHLCV のいずれかが `"*"`（未取得）の場合は `None` を返す。
/// - `time` は当日の 0:00:00 JST (UTC+9) を Unix epoch ミリ秒で表す。
pub fn daily_record_to_kline(record: &DailyHistoryRecord, use_adjusted: bool) -> Option<Kline> {
    let (open_s, high_s, low_s, close_s, volume_s) = if use_adjusted {
        (
            &record.open_adj,
            &record.high_adj,
            &record.low_adj,
            &record.close_adj,
            &record.volume_adj,
        )
    } else {
        (
            &record.open,
            &record.high,
            &record.low,
            &record.close,
            &record.volume,
        )
    };

    // "*" は未取得を意味する
    let parse = |s: &str| -> Option<f32> {
        if s == "*" || s.is_empty() {
            None
        } else {
            s.parse().ok()
        }
    };

    let open = parse(open_s)?;
    let high = parse(high_s)?;
    let low = parse(low_s)?;
    let close = parse(close_s)?;
    let volume = parse(volume_s)?;

    // "YYYYMMDD" → epoch ミリ秒 (JST 深夜0時 = UTC - 9h)
    let time = date_str_to_epoch_ms(&record.date)?;

    // 日本株は整数円なので min_ticksize = 10^0 = 1
    let min_ticksize = MinTicksize::new(0);
    let qty = Qty::from_f32(volume);

    Some(Kline::new(
        time,
        open,
        high,
        low,
        close,
        Volume::TotalOnly(qty),
        min_ticksize,
    ))
}

/// "YYYYMMDD" 形式の日付文字列を JST 深夜0時の Unix epoch ミリ秒に変換する。
fn date_str_to_epoch_ms(date: &str) -> Option<u64> {
    if date.len() != 8 {
        return None;
    }
    let year: i32 = date[0..4].parse().ok()?;
    let month: u32 = date[4..6].parse().ok()?;
    let day: u32 = date[6..8].parse().ok()?;

    use chrono::NaiveDate;
    let naive = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(0, 0, 0)?;

    // JST は UTC+9 なので -9h して UTC に変換
    const JST_OFFSET_SECS: i64 = 9 * 3600;
    let epoch_secs = naive.and_utc().timestamp() - JST_OFFSET_SECS;
    Some((epoch_secs as u64) * 1000)
}

// ── 銘柄マスタ（MASTER I/F） ─────────────────────────────────────────────────

use crate::{Exchange, Ticker, TickerInfo};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// 全マスタダウンロードの各レコードをパースするための汎用型。
/// sCLMID でレコード種別を判定し、CLMIssueMstKabu のみ抽出する。
#[derive(Debug, Deserialize, Clone)]
pub struct MasterRecord {
    #[serde(rename = "sCLMID", default)]
    pub clm_id: String,
    #[serde(rename = "sIssueCode", default)]
    pub issue_code: String,
    #[serde(rename = "sIssueName", default)]
    pub issue_name: String,
    #[serde(rename = "sIssueNameRyaku", default)]
    pub issue_name_short: String,
    #[serde(rename = "sIssueNameKana", default)]
    pub issue_name_kana: String,
    #[serde(rename = "sIssueNameEizi", default)]
    pub issue_name_english: String,
    #[serde(rename = "sYusenSizyou", default)]
    pub primary_market: String,
    #[serde(rename = "sGyousyuCode", default)]
    pub sector_code: String,
    #[serde(rename = "sGyousyuName", default)]
    pub sector_name: String,
}

/// MasterRecord (CLMIssueMstKabu) → (Ticker, TickerInfo) に変換。
/// display_symbol には ASCII の英語名 (sIssueNameEizi) を使用する
/// （Ticker は ASCII のみ対応のため日本語名は不可）。
pub fn master_record_to_ticker_info(record: &MasterRecord) -> Option<(Ticker, TickerInfo)> {
    if record.clm_id != "CLMIssueMstKabu" {
        return None;
    }
    if record.issue_code.is_empty() {
        return None;
    }

    let display = if record.issue_name_english.is_empty() {
        None
    } else {
        Some(record.issue_name_english.as_str())
    };

    // display_symbol が MAX_LEN (28) を超える場合は切り捨て
    let display = display.map(|d| if d.len() > 28 { &d[..28] } else { d });

    // display が非 ASCII なら Ticker がパニックするので None にフォールバック
    let display = display.filter(|d| d.is_ascii());

    let ticker = Ticker::new_with_display(&record.issue_code, Exchange::Tachibana, display);

    let info = TickerInfo::new(
        ticker, 1.0,   // min_ticksize (暫定: 呼値テーブルで正確化可能)
        100.0, // min_qty = 日本株デフォルト売買単位
        None,  // contract_size (現物なので不要)
    );

    Some((ticker, info))
}

/// マスタダウンロード用リクエスト。
#[derive(Debug, Serialize)]
struct MasterDownloadRequest {
    p_no: String,
    p_sd_date: String,
    #[serde(rename = "sCLMID")]
    clm_id: &'static str,
    #[serde(rename = "sJsonOfmt")]
    json_ofmt: &'static str,
}

impl MasterDownloadRequest {
    fn new() -> Self {
        Self {
            p_no: next_p_no(),
            p_sd_date: current_p_sd_date(),
            clm_id: "CLMEventDownload",
            json_ofmt: "4",
        }
    }
}

/// Shift-JIS バイトストリームを `}` (0x7D) で JSON レコードに分割する。
///
/// Shift-JIS の2バイト文字はリードバイト (0x81-0x9F, 0xE0-0xEF) の直後に
/// 0x7D が来ることがある。このトレイルバイトはレコード境界ではなく文字の一部なので
/// 誤検知を防ぐためリードバイト追跡を行う。
/// 各エントリには末尾の `}` を含む。末尾に `}` のない残余バイトもそのまま返す。
#[cfg(test)]
pub(crate) fn parse_sjis_stream_records(data: &[u8]) -> Vec<Vec<u8>> {
    let mut records = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut in_multibyte = false;

    for &byte in data {
        buf.push(byte);
        if in_multibyte {
            // Shift-JIS 2バイト文字のトレイルバイト: 次のバイトは通常の1バイトとして扱う
            in_multibyte = false;
        } else if matches!(byte, 0x81..=0x9F | 0xE0..=0xEF) {
            // Shift-JIS リードバイト: 次のバイトはトレイルバイトとして扱う
            in_multibyte = true;
        } else if byte == b'}' {
            records.push(buf.clone());
            buf.clear();
        }
    }

    if !buf.is_empty() {
        records.push(buf);
    }

    records
}

/// MASTER I/F で全マスタを一括ダウンロードする。
/// CLMEventDownloadComplete を受信するまでストリーミングで読み取り、
/// CLMIssueMstKabu レコードのみを抽出して返す。
pub async fn fetch_all_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<Vec<MasterRecord>, TachibanaError> {
    use futures::StreamExt;

    let req = MasterDownloadRequest::new();
    let url = build_api_url_from(&session.url_master, &req)?;

    log::debug!("Tachibana master download URL: {url}");

    let resp = client.get(&url).send().await?;
    let mut stream = resp.bytes_stream();

    let mut buf = Vec::new();
    let mut records = Vec::new();
    let mut seen_kabu = false;
    let mut chunk_count = 0usize;
    // Shift-JIS の2バイト文字でトレイルバイトが 0x7D になる場合があるため
    // リードバイト後の次のバイトをトレイルバイトとして扱うフラグ
    let mut in_multibyte = false;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(e) => {
                if !records.is_empty() {
                    log::warn!(
                        "Tachibana master stream interrupted at chunk #{chunk_count} ({} records so far): {e}. \
                         Returning partial data.",
                        records.len()
                    );
                    return Ok(records);
                } else {
                    log::error!("Tachibana master stream failed at chunk #{chunk_count} (no records yet): {e}");
                    return Err(TachibanaError::Http(e));
                }
            }
        };
        chunk_count += 1;
        for &byte in chunk.iter() {
            buf.push(byte);
            if in_multibyte {
                // Shift-JIS トレイルバイト: レコード境界チェックをスキップ
                in_multibyte = false;
                continue;
            } else if matches!(byte, 0x81..=0x9F | 0xE0..=0xEF) {
                // Shift-JIS リードバイト: 次のバイトはトレイルバイト
                in_multibyte = true;
                continue;
            }
            if byte == b'}' {
                // `}` でレコード境界を判定（Shift-JIS 2バイト文字を考慮済み）
                let (decoded, _, had_errors) = encoding_rs::SHIFT_JIS.decode(&buf);
                if had_errors {
                    log::warn!(
                        "Shift-JIS decode produced lossy output in master stream ({} bytes)",
                        buf.len()
                    );
                }
                let decoded = decoded.into_owned();
                buf.clear();

                let parsed: Result<MasterRecord, _> = serde_json::from_str(&decoded);
                match parsed {
                    Ok(record) => {
                        if record.clm_id == "CLMEventDownloadComplete" {
                            log::info!(
                                "Tachibana master download complete: {} issue records",
                                records.len()
                            );
                            return Ok(records);
                        }
                        if record.clm_id == "CLMIssueMstKabu" && !record.issue_code.is_empty() {
                            seen_kabu = true;
                            records.push(record);
                        } else if seen_kabu {
                            // CLMIssueMstKabu の区間を過ぎた → 早期リターン
                            // 残りのマスタ（呼値テーブル等）は不要なので読み捨てる
                            // ※ 公式Pythonサンプル準拠: マスタデータは種別ごとに連続配信される前提。
                            //   API仕様変更で非連続配信になった場合、レコード欠損の可能性あり。
                            log::warn!(
                                "Tachibana master early return after kabu section: {} records (next: {}). \
                                 Assumption: CLMIssueMstKabu records are contiguous in the stream.",
                                records.len(),
                                record.clm_id
                            );
                            return Ok(records);
                        }
                    }
                    Err(e) => {
                        log::trace!("Skipping unparseable master record: {e}");
                    }
                }
            }
        }
    }

    log::warn!(
        "Tachibana master stream ended without CLMEventDownloadComplete ({} records so far)",
        records.len()
    );
    Ok(records)
}

// ── マスタキャッシュ ─────────────────────────────────────────────────────────

static ISSUE_MASTER_CACHE: RwLock<Option<Arc<Vec<MasterRecord>>>> = RwLock::new(None);

/// ログイン成功時に呼び出し、銘柄マスタをキャッシュに格納する。
pub async fn init_issue_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(), TachibanaError> {
    let records = fetch_all_master(client, session).await?;
    if let Ok(mut guard) = ISSUE_MASTER_CACHE.write() {
        *guard = Some(Arc::new(records));
    }
    Ok(())
}

/// キャッシュ済みの銘柄マスタを返す。未取得なら None。
pub async fn get_cached_issue_master() -> Option<Arc<Vec<MasterRecord>>> {
    ISSUE_MASTER_CACHE.read().ok()?.clone()
}

/// バックグラウンドで銘柄マスタをダウンロードしキャッシュに格納する。
/// ログイン成功後に呼び出す。tokio::spawn でタスクを起動するため、
/// 呼び出し元は完了を待つ必要がない。
pub fn spawn_init_issue_master(session: TachibanaSession) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        if let Err(e) = init_issue_master(&client, &session).await {
            log::error!("Tachibana master download failed: {e}");
        }
    });
}

/// キャッシュから Ticker → TickerInfo の HashMap を構築する。
///
/// ペイン設定は display_symbol なしで `TachibanaSpot:7203` と保存されるが、
/// `master_record_to_ticker_info` は英語名付き Ticker をキーとして生成する。
/// Ticker の Hash/Eq は display_bytes を含むため、両方のキーで引けるよう
/// display なしのエントリも追加で挿入する。
pub async fn cached_ticker_metadata() -> HashMap<Ticker, Option<TickerInfo>> {
    let mut out = HashMap::new();
    let cache = get_cached_issue_master().await;
    if let Some(records) = cache {
        for record in records.iter() {
            if let Some((ticker, info)) = master_record_to_ticker_info(record) {
                // display なしキーも同じ TickerInfo で登録しておく。
                // ペイン設定は display_symbol なしで保存されるため、こちらで
                // stream resolution の resolver(&ticker) が正しくヒットする。
                let ticker_no_display =
                    Ticker::new(&record.issue_code, Exchange::Tachibana);
                out.entry(ticker_no_display).or_insert(Some(info));
                out.insert(ticker, Some(info));
            }
        }
    }
    out
}

// ── E2E テスト用 fixture 注入バックドア ─────────────────────────────────────
//
// `e2e-mock` feature を有効にした場合のみコンパイルされる。本番ビルドには
// 一切含まれない。認証フロー・MASTER I/F・日足取得をバイパスし、
// Tachibana D1 リプレイ経路の E2E 検証を可能にする。
// 詳細: docs/plan/tachibana_e2e_phase_t1.md

#[cfg(feature = "e2e-mock")]
pub mod e2e_mock {
    use super::{ISSUE_MASTER_CACHE, MasterRecord};
    use crate::Kline;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    /// `fetch_daily_history` を mock 経路へ分岐させるためのテーブル。
    /// キーは issue_code（例: "7203"）。
    /// std::sync::RwLock を使うのは、test backdoor が iced の update() ループから
    /// 同期的に呼び出されるため（tokio::sync::RwLock だと await が必要で、
    /// block_in_place の利用条件を満たさないケースで動作しない）。
    pub(super) static MOCK_DAILY_HISTORY: RwLock<Option<HashMap<String, Vec<Kline>>>> =
        RwLock::new(None);

    /// ISSUE_MASTER_CACHE にレコードを直接格納する。
    /// MASTER I/F ストリーミングをバイパスするため、`init_issue_master` を経由せずに
    /// `cached_ticker_metadata` から参照可能な状態を作れる。
    pub fn inject_master_cache(records: Vec<MasterRecord>) {
        if let Ok(mut guard) = ISSUE_MASTER_CACHE.write() {
            *guard = Some(Arc::new(records));
        }
    }

    /// 指定 issue_code の日足 mock データを設定する（累積）。
    /// 以降の `fetch_tachibana_daily_klines` 呼び出しはネットワークを叩かず、
    /// ここで設定した Kline 列を返す（range フィルタは呼び出し側でそのまま効く）。
    pub fn inject_daily_klines(issue_code: String, klines: Vec<Kline>) {
        if let Ok(mut guard) = MOCK_DAILY_HISTORY.write() {
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(issue_code, klines);
        }
    }

    /// Mock テーブル全体をクリアする。テスト後のクリーンアップ用。
    pub fn clear_daily_klines() {
        if let Ok(mut guard) = MOCK_DAILY_HISTORY.write() {
            *guard = None;
        }
    }

    /// ISSUE_MASTER_CACHE をクリアする。テスト後のクリーンアップ用。
    pub fn clear_master_cache() {
        if let Ok(mut guard) = ISSUE_MASTER_CACHE.write() {
            *guard = None;
        }
    }

    /// 指定 issue_code の mock 日足を取り出す。`fetch_tachibana_daily_klines` から参照される。
    pub fn get_mock_daily_klines(issue_code: &str) -> Option<Vec<Kline>> {
        let guard = MOCK_DAILY_HISTORY.read().ok()?;
        guard.as_ref()?.get(issue_code).cloned()
    }

    // ── fetch_market_prices mock (Phase T2) ─────────────────────────────────

    /// `fetch_market_prices` を mock 経路へ分岐させるためのストア。
    /// `inject_market_prices` で注入されたデータを `fetch_market_prices` が参照する。
    pub(super) static MOCK_MARKET_PRICES: RwLock<Option<Vec<super::MarketPriceRecord>>> =
        RwLock::new(None);

    /// mock 時価情報レコードを登録する。
    /// 以降の `fetch_market_prices` 呼び出しはネットワークを叩かず、ここで設定したレコードを返す。
    pub fn inject_market_prices(records: Vec<super::MarketPriceRecord>) {
        if let Ok(mut guard) = MOCK_MARKET_PRICES.write() {
            *guard = Some(records);
        }
    }

    /// mock 時価情報レコードを取り出す。`fetch_market_prices` から参照される。
    pub fn get_mock_market_prices() -> Option<Vec<super::MarketPriceRecord>> {
        let guard = MOCK_MARKET_PRICES.read().ok()?;
        guard.clone()
    }

    /// mock 時価情報ストアをクリアする。テスト後のクリーンアップ用。
    pub fn clear_market_prices() {
        if let Ok(mut guard) = MOCK_MARKET_PRICES.write() {
            *guard = None;
        }
    }
}

// ── EVENT I/F パーサー ───────────────────────────────────────────────────────
//
// EVENT I/F WebSocket はカスタムバイナリ区切りフォーマット:
//   SOH (\x01) = フィールド区切り
//   STX (\x02) = カラム名:値 区切り
//   ETX (\x03) = 値のサブ区切り（複数値を持つフィールド内）
// エンコーディングは ASCII（REST の Shift-JIS とは異なる）。

use crate::depth::{DeOrder, DepthPayload};
use crate::{Price, Trade};

/// EVENT I/F の1フレームをパースし、(カラム名, 値) のペア列に分解する。
pub fn parse_event_frame(data: &str) -> Vec<(&str, &str)> {
    data.split('\x01')
        .filter(|r| !r.is_empty())
        .filter_map(|record| {
            let mut parts = record.splitn(2, '\x02');
            match (parts.next(), parts.next()) {
                (Some(col), Some(val)) if !col.is_empty() => Some((col, val)),
                _ => None,
            }
        })
        .collect()
}

/// パース済みフィールドから板情報に変換する。
/// EVENT I/F のフィールド名は `p_{行番号}_{情報コード}` 形式。
/// 売気配: GAP1(最良)〜GAP10(上位) + GAV1〜GAV10
/// 買気配: GBP1(最良)〜GBP10(下位) + GBV1〜GBV10
pub fn fields_to_depth(fields: &[(&str, &str)]) -> Option<DepthPayload> {
    /// フィールド名の末尾が `_suffix` と一致するか（例: `p_1_GAP1` → suffix `_QAP1`）
    fn find_val_suffix(fields: &[(&str, &str)], suffix: &str) -> Option<f32> {
        fields
            .iter()
            .find(|(k, _)| k.ends_with(suffix))
            .and_then(|(_, v)| {
                if *v == "*" || v.is_empty() {
                    None
                } else {
                    v.parse().ok()
                }
            })
    }

    // FD コマンドでのみ板情報を処理
    let cmd = fields.iter().find(|(k, _)| *k == "p_cmd").map(|(_, v)| *v);
    if cmd != Some("FD") {
        return None;
    }

    let mut asks = Vec::new();
    let mut bids = Vec::new();

    for i in 1..=10 {
        let price_suffix = format!("_GAP{i}");
        let qty_suffix = format!("_GAV{i}");
        if let (Some(price), Some(qty)) = (
            find_val_suffix(fields, &price_suffix),
            find_val_suffix(fields, &qty_suffix),
        ) {
            asks.push(DeOrder { price, qty });
        }
    }

    for i in 1..=10 {
        let price_suffix = format!("_GBP{i}");
        let qty_suffix = format!("_GBV{i}");
        if let (Some(price), Some(qty)) = (
            find_val_suffix(fields, &price_suffix),
            find_val_suffix(fields, &qty_suffix),
        ) {
            bids.push(DeOrder { price, qty });
        }
    }

    if asks.is_empty() && bids.is_empty() {
        return None;
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Some(DepthPayload {
        last_update_id: now_ms,
        time: now_ms,
        bids,
        asks,
    })
}

/// パース済みフィールドから Trade に変換する。
/// EVENT I/F フィールド名: `p_{行番号}_{情報コード}` 形式。
/// DPP = 約定価格, DV = 約定数量, DYSS = 売買区分 ("1"=売)
/// `_DPP` で末尾マッチ（`_XDPP` 等との誤マッチを防ぐため `_` 付き）。
pub fn fields_to_trade(fields: &[(&str, &str)]) -> Option<Trade> {
    let get_suffix = |suffix: &str| -> Option<&str> {
        fields
            .iter()
            .find(|(k, _)| k.ends_with(suffix))
            .map(|(_, v)| *v)
    };

    // ST（歩み値）コマンドでのみ有効。FD/KP では Trade を生成しない。
    let cmd = get_suffix("p_cmd");
    if cmd != Some("ST") {
        return None;
    }

    let price_str = get_suffix("_DPP")?;
    if price_str == "*" || price_str.is_empty() {
        return None;
    }
    let price: f32 = price_str.parse().ok()?;

    let qty_str = get_suffix("_DV")?;
    if qty_str == "*" || qty_str.is_empty() {
        return None;
    }
    let qty: f32 = qty_str.parse().ok()?;

    let is_sell = get_suffix("_DYSS").map(|v| v == "1").unwrap_or(false);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Some(Trade {
        time: now_ms,
        is_sell,
        price: Price::from_f32(price),
        qty: Qty::from_f32(qty),
    })
}

// ── EVENT I/F WebSocket 接続 ─────────────────────────────────────────────────

use crate::PushFrequency;
use crate::adapter::{Event, StreamKind, StreamTicksize};
use crate::connect::channel;
use crate::depth::{DepthUpdate, LocalDepthCache};
use futures::Stream;
use std::time::Duration;

/// EVENT I/F URL を exchange crate 内で保持する。
/// HTTP Long-polling 用の url_event と WebSocket 用の url_event_ws を両方保持。
/// auth.rs の store_session() から set_event_urls() 経由で設定される。
static EVENT_HTTP_URL: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);
static EVENT_WS_URL: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// セッション取得時に EVENT I/F URL を設定する。
pub fn set_event_ws_url(url: String) {
    if let Ok(mut guard) = EVENT_WS_URL.write() {
        *guard = Some(url);
    }
}

/// セッション取得時に EVENT I/F HTTP URL を設定する。
pub fn set_event_http_url(url: String) {
    if let Ok(mut guard) = EVENT_HTTP_URL.write() {
        *guard = Some(url);
    }
}

#[cfg(test)]
fn get_event_ws_url() -> Option<String> {
    EVENT_WS_URL.read().ok()?.clone()
}

fn get_event_http_url() -> Option<String> {
    EVENT_HTTP_URL.read().ok()?.clone()
}

/// EVENT I/F の接続パラメータを構築する。
/// 公式サンプル準拠: パラメータ順序は固定（順番の変更は不可）。
/// p_rid → p_board_no → p_gyou_no → p_mkt_code → p_eno → p_evt_cmd → p_issue_code
fn build_event_params(issue_code: &str, market_code: &str) -> String {
    format!(
        "p_rid=22&p_board_no=1000&p_gyou_no=1&p_mkt_code={}&p_eno=0&p_evt_cmd=ST,KP,FD&p_issue_code={}",
        market_code, issue_code,
    )
}

/// EVENT I/F に接続し、板情報・歩み値を統合した Event ストリームを返す。
///
/// HTTP Long-polling（`sUrlEvent`）で接続する。
/// 公式サンプル `e_api_sample_v4r8.py` の HTTP ストリーミング方式:
///   requests.session().get(url, stream=True).iter_lines()
///
/// `trade_stream` は空のまま。本ストリームで TradesReceived も発行する。
pub fn connect_event_stream(
    ticker_info: TickerInfo,
    push_freq: PushFrequency,
) -> impl Stream<Item = Event> {
    channel(100, move |mut output| async move {
        use futures::SinkExt;

        log::info!(
            "Tachibana EVENT I/F stream started for {:?}",
            ticker_info.ticker
        );

        let exchange = Exchange::Tachibana;
        let mut orderbook = LocalDepthCache::default();

        let stream_kind_depth = StreamKind::Depth {
            ticker_info,
            depth_aggr: StreamTicksize::default(),
            push_freq,
        };
        let stream_kind_trades = StreamKind::Trades { ticker_info };

        loop {
            let event_url = match get_event_http_url() {
                Some(url) => url,
                None => {
                    log::warn!("[e2e-live] Tachibana EVENT I/F URL not available (no session), waiting 3s...");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let (issue_code, _) = ticker_info.ticker.to_full_symbol_and_type();
            let params = build_event_params(&issue_code, "00");
            let url = format!("{}?{}", event_url, params);
            log::info!("[e2e-live] Tachibana EVENT I/F connecting: issue={} url_domain={}", issue_code,
                url.split('/').nth(2).unwrap_or("unknown"));

            let client = reqwest::Client::new();
            match client.get(&url).send().await {
                Ok(response) => {
                    if !response.status().is_success() {
                        log::error!(
                            "Tachibana EVENT I/F HTTP error: status={}",
                            response.status()
                        );
                        let _ = output
                            .send(Event::Disconnected(
                                exchange,
                                format!("HTTP {}", response.status()),
                            ))
                            .await;
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        continue;
                    }

                    log::info!("Tachibana EVENT I/F connected");
                    let _ = output.send(Event::Connected(exchange)).await;

                    // ストリーミングレスポンスを行ごとに読む
                    use futures::StreamExt;
                    let mut byte_stream = response.bytes_stream();

                    let mut line_buf = String::new();

                    while let Some(chunk_result) = byte_stream.next().await {
                        match chunk_result {
                            Ok(chunk) => {
                                // ASCII デコード（公式サンプル: p_rec.decode('ascii')）
                                let text = String::from_utf8_lossy(&chunk);
                                // チャンクは行境界を跨ぐ可能性があるため蓄積して行分割
                                line_buf.push_str(&text);

                                // 改行で分割
                                while let Some(newline_pos) = line_buf.find('\n') {
                                    let line: String = line_buf.drain(..=newline_pos).collect();
                                    let line = line.trim();
                                    if line.is_empty() {
                                        continue;
                                    }

                                    let fields = parse_event_frame(line);
                                    if fields.is_empty() {
                                        continue;
                                    }
                                    if let Some(depth_payload) = fields_to_depth(&fields) {
                                        let time = depth_payload.time;
                                        orderbook.update(
                                            DepthUpdate::Snapshot(depth_payload),
                                            ticker_info.min_ticksize,
                                        );
                                        let _ = output
                                            .send(Event::DepthReceived(
                                                stream_kind_depth,
                                                time,
                                                orderbook.depth.clone(),
                                            ))
                                            .await;
                                    }

                                    if let Some(trade) = fields_to_trade(&fields) {
                                        let time = trade.time;
                                        let _ = output
                                            .send(Event::TradesReceived(
                                                stream_kind_trades,
                                                time,
                                                Box::new([trade]),
                                            ))
                                            .await;
                                    }
                                }

                                // 残りの未完了行は line_buf に残る（次のチャンクで処理）
                            }
                            Err(e) => {
                                log::error!("Tachibana EVENT I/F stream error: {}", e);
                                break;
                            }
                        }
                    }

                    // ストリーム終了
                    log::warn!("Tachibana EVENT I/F stream ended, reconnecting...");
                    let _ = output
                        .send(Event::Disconnected(exchange, "Stream ended".to_string()))
                        .await;
                }
                Err(e) => {
                    log::error!("Tachibana EVENT I/F connect failed: {}", e);
                    let _ = output
                        .send(Event::Disconnected(exchange, e.to_string()))
                        .await;
                }
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
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
        assert!(json.contains(r#""sUserId":"user123""#), "JSON: {json}");
        assert!(json.contains(r#""sPassword":"secret!""#), "JSON: {json}");
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
        assert_eq!(response.url_event_ws, "wss://virtual.example.com/event-ws/");
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
            p_errno: "0".to_string(),
            p_err: String::new(),
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
            p_errno: "0".to_string(),
            p_err: String::new(),
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
            p_errno: "0".to_string(),
            p_err: String::new(),
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
        assert!(
            url.starts_with(base),
            "URL はベース URL で始まるべき: {url}"
        );
        assert!(
            url.contains("CLMAuthLoginRequest"),
            "URL に CLMAuthLoginRequest が含まれるべき: {url}"
        );
        assert!(
            url.contains("user"),
            "URL にユーザーIDが含まれるべき: {url}"
        );
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
            "aCLMMfdsMarketPriceHistory": [
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
            "aCLMMfdsMarketPriceHistory": [
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
            .mock("POST", mockito::Matcher::Any)
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
            .mock("POST", mockito::Matcher::Any)
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
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "p_errno": "0",
                    "p_err": "",
                    "sResultCode": "0",
                    "sResultText": "",
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
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "p_errno": "0",
                    "p_err": "",
                    "sResultCode": "0",
                    "sResultText": "",
                    "aCLMMfdsMarketPriceHistory": [
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

    // ── Cycle B4: 業務 API エラーチェック ────────────────────────────────────

    #[test]
    fn api_response_check_returns_data_on_success() {
        let json = r#"{
            "p_errno": "0",
            "p_err": "",
            "sResultCode": "0",
            "sResultText": "",
            "aCLMMfdsMarketPrice": [
                {"sIssueCode": "6501", "pDPP": "3250"}
            ]
        }"#;
        let resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(json).unwrap();
        let data = resp.check().unwrap();
        assert_eq!(data.records.len(), 1);
    }

    #[test]
    fn api_response_check_returns_error_on_p_errno() {
        let json = r#"{
            "p_errno": "2",
            "p_err": "セッションが切断されました。",
            "sResultCode": "0",
            "sResultText": "",
            "aCLMMfdsMarketPrice": []
        }"#;
        let resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(json).unwrap();
        let result = resp.check();
        assert!(
            matches!(result, Err(TachibanaError::ApiError { ref code, .. }) if code == "2"),
            "p_errno が 0 でない場合は ApiError が返るべき: {:?}",
            result
        );
    }

    #[test]
    fn api_response_check_returns_error_on_result_code() {
        let json = r#"{
            "p_errno": "0",
            "p_err": "",
            "sResultCode": "-62",
            "sResultText": "稼働時間外です",
            "aCLMMfdsMarketPrice": []
        }"#;
        let resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(json).unwrap();
        let result = resp.check();
        assert!(
            matches!(result, Err(TachibanaError::ApiError { ref code, .. }) if code == "-62"),
            "sResultCode が 0 でない場合は ApiError が返るべき: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn fetch_market_prices_returns_api_error_on_session_expired() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_body(
                r#"{
                    "p_errno": "2",
                    "p_err": "セッションが切断されました。",
                    "sResultCode": "0",
                    "sResultText": "",
                    "aCLMMfdsMarketPrice": []
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

        let result = fetch_market_prices(&client, &session, &["6501"]).await;
        assert!(
            matches!(result, Err(TachibanaError::ApiError { .. })),
            "セッション切れ時は ApiError が返るべき: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn fetch_daily_history_returns_api_error_on_p_errno() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_body(
                r#"{
                    "p_errno": "-62",
                    "p_err": "稼働時間外です",
                    "sResultCode": "0",
                    "sResultText": "",
                    "aCLMMfdsMarketPriceHistory": []
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

        let result = fetch_daily_history(&client, &session, "6501").await;
        assert!(
            matches!(result, Err(TachibanaError::ApiError { .. })),
            "稼働時間外時は ApiError が返るべき: {:?}",
            result
        );
    }

    // ── Cycle B3: パスワード URL エンコード ─────────────────────────────────

    #[tokio::test]
    async fn login_url_encodes_password_with_special_chars() {
        let mut server = mockito::Server::new_async().await;
        // POST body に URL エンコードされたパスワードが含まれることを確認
        let mock = server
            .mock("POST", mockito::Matcher::Any)
            .match_body(mockito::Matcher::Regex(
                // "pass{word}" → "pass%7Bword%7D" がJSONに含まれる
                r#"pass%7Bword%7D"#.to_string(),
            ))
            .with_status(200)
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "0",
                    "sUrlRequest": "https://r.example.com/",
                    "sUrlMaster": "https://m.example.com/",
                    "sUrlPrice": "https://p.example.com/",
                    "sUrlEvent": "https://e.example.com/",
                    "sUrlEventWebSocket": "wss://ws.example.com/",
                    "sKinsyouhouMidokuFlg": "0",
                    "sResultText": ""
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let base_url = format!("{}/", server.url());
        let _session = login(&client, &base_url, "user".into(), "pass{word}".into())
            .await
            .unwrap();

        mock.assert_async().await;
    }

    // ── Cycle B2: p_no インクリメンタルカウンタ ────────────────────────────

    #[test]
    fn next_p_no_returns_incrementing_values() {
        let a = next_p_no();
        let b = next_p_no();
        let a_num: u64 = a.parse().expect("p_no は数値であるべき");
        let b_num: u64 = b.parse().expect("p_no は数値であるべき");
        assert_eq!(b_num, a_num + 1, "p_no は連続してインクリメントされるべき");
    }

    #[test]
    fn login_request_p_no_is_not_hardcoded_one() {
        // next_p_no を何回か呼んだ後に LoginRequest を生成
        let _ = next_p_no();
        let _ = next_p_no();
        let req = LoginRequest::new("u".into(), "p".into());
        assert_ne!(
            req.p_no, "1",
            "p_no はハードコードの '1' であってはならない"
        );
    }

    #[test]
    fn market_price_request_uses_dynamic_p_no() {
        let req1 = MarketPriceRequest::new(&["6501"]);
        let req2 = MarketPriceRequest::new(&["7203"]);
        assert_ne!(req1.p_no, req2.p_no, "連続リクエストで p_no が異なるべき");
    }

    #[test]
    fn daily_history_request_uses_dynamic_p_no() {
        let req1 = DailyHistoryRequest::new("6501");
        let req2 = DailyHistoryRequest::new("7203");
        assert_ne!(req1.p_no, req2.p_no, "連続リクエストで p_no が異なるべき");
    }

    #[test]
    fn next_p_no_concurrent_calls_return_unique_values() {
        use std::collections::HashSet;
        let handles: Vec<_> = (0..10).map(|_| std::thread::spawn(next_p_no)).collect();
        let values: HashSet<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            values.len(),
            10,
            "並行呼び出しでも全 p_no がユニークであるべき"
        );
    }

    // ── Cycle B1: HTTP POST 対応 ───────────────────────────────────────────

    #[tokio::test]
    async fn login_sends_post_request_with_json_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", mockito::Matcher::Any)
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_body(
                r#"{
                    "sCLMID": "CLMAuthLoginAck",
                    "sResultCode": "0",
                    "sUrlRequest": "https://r.example.com/",
                    "sUrlMaster": "https://m.example.com/",
                    "sUrlPrice": "https://p.example.com/",
                    "sUrlEvent": "https://e.example.com/",
                    "sUrlEventWebSocket": "wss://ws.example.com/",
                    "sKinsyouhouMidokuFlg": "0",
                    "sResultText": ""
                }"#,
            )
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let base_url = format!("{}/", server.url());
        let _session = login(&client, &base_url, "u".into(), "p".into())
            .await
            .unwrap();

        mock.assert_async().await; // POST でなければ mock がマッチせず失敗
    }

    #[tokio::test]
    async fn fetch_market_prices_sends_post_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", mockito::Matcher::Any)
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_body(
                r#"{
                    "p_errno": "0",
                    "p_err": "",
                    "sResultCode": "0",
                    "sResultText": "",
                    "aCLMMfdsMarketPrice": [
                        {"sIssueCode": "6501", "pDPP": "3250"}
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

        let _records = fetch_market_prices(&client, &session, &["6501"])
            .await
            .unwrap();

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_daily_history_sends_post_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", mockito::Matcher::Any)
            .match_header("content-type", "application/json")
            .with_status(200)
            .with_body(
                r#"{
                    "p_errno": "0",
                    "p_err": "",
                    "sResultCode": "0",
                    "sResultText": "",
                    "aCLMMfdsMarketPriceHistory": [
                        {"sDate":"20240101","pDOP":"3200","pDHP":"3280","pDLP":"3150","pDPP":"3250","pDV":"1500000"}
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

        let _records = fetch_daily_history(&client, &session, "6501")
            .await
            .unwrap();

        mock.assert_async().await;
    }

    // ── Cycle A1: DailyHistoryRecord → Kline 変換 ────────────────────────────

    #[test]
    fn daily_record_converts_to_kline_with_correct_ohlcv() {
        let record = DailyHistoryRecord {
            date: "20240101".to_string(),
            open: "3200".to_string(),
            high: "3280".to_string(),
            low: "3150".to_string(),
            close: "3250".to_string(),
            volume: "1500000".to_string(),
            open_adj: String::new(),
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };

        let kline = daily_record_to_kline(&record, false).expect("変換できるはず");

        // 価格を f32 に戻して確認（Price は整数ベースなので近似比較）
        let open_f32 = kline.open.to_f32();
        let high_f32 = kline.high.to_f32();
        let low_f32 = kline.low.to_f32();
        let close_f32 = kline.close.to_f32();

        assert!((open_f32 - 3200.0).abs() < 1.0, "open: {open_f32}");
        assert!((high_f32 - 3280.0).abs() < 1.0, "high: {high_f32}");
        assert!((low_f32 - 3150.0).abs() < 1.0, "low: {low_f32}");
        assert!((close_f32 - 3250.0).abs() < 1.0, "close: {close_f32}");
    }

    // ── Cycle A2: "*" フィールドで None を返す ────────────────────────────────

    #[test]
    fn daily_record_returns_none_when_close_is_asterisk() {
        let record = DailyHistoryRecord {
            date: "20240101".to_string(),
            open: "3200".to_string(),
            high: "3280".to_string(),
            low: "3150".to_string(),
            close: "*".to_string(), // 未取得
            volume: "1500000".to_string(),
            open_adj: String::new(),
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };

        let result = daily_record_to_kline(&record, false);
        assert!(result.is_none(), "close が \"*\" の場合は None を返すべき");
    }

    // ── Cycle A3: 日付 YYYYMMDD → epoch ミリ秒 ───────────────────────────────

    #[test]
    fn daily_record_time_is_midnight_jst_of_given_date() {
        let record = DailyHistoryRecord {
            date: "20240101".to_string(),
            open: "100".to_string(),
            high: "110".to_string(),
            low: "90".to_string(),
            close: "105".to_string(),
            volume: "1000".to_string(),
            open_adj: String::new(),
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };

        let kline = daily_record_to_kline(&record, false).expect("変換できるはず");

        // 2024-01-01 00:00:00 JST = 2023-12-31 15:00:00 UTC
        // UTC epoch: 2023-12-31 15:00:00 = 1704034800 seconds = 1704034800000 ms
        let expected_ms: u64 = 1704034800000;
        assert_eq!(
            kline.time, expected_ms,
            "time は JST 深夜0時の epoch ms であるべき"
        );
    }

    // ── Cycle A4: 調整値を使用する ────────────────────────────────────────────

    #[test]
    fn daily_record_uses_adjusted_values_when_flag_is_true() {
        let record = DailyHistoryRecord {
            date: "20200101".to_string(),
            open: "6400".to_string(),
            high: "6560".to_string(),
            low: "6300".to_string(),
            close: "6500".to_string(),
            volume: "750000".to_string(),
            open_adj: "3200".to_string(),
            high_adj: "3280".to_string(),
            low_adj: "3150".to_string(),
            close_adj: "3250".to_string(),
            volume_adj: "1500000".to_string(),
        };

        let kline = daily_record_to_kline(&record, true).expect("変換できるはず");

        let close_f32 = kline.close.to_f32();
        assert!(
            (close_f32 - 3250.0).abs() < 1.0,
            "調整後終値は 3250 であるべき: {close_f32}"
        );
    }

    // ── Cycle S1: TachibanaSession の JSON ラウンドトリップ ─────────────────────

    #[test]
    fn tachibana_session_json_roundtrip_preserves_all_fields() {
        let session = TachibanaSession {
            url_request: "https://virt.test/request/".to_string(),
            url_master: "https://virt.test/master/".to_string(),
            url_price: "https://virt.test/price/".to_string(),
            url_event: "https://virt.test/event/".to_string(),
            url_event_ws: "wss://virt.test/ws/".to_string(),
        };

        let json = serde_json::to_string(&session).expect("serialize すべき");
        let restored: TachibanaSession = serde_json::from_str(&json).expect("deserialize すべき");

        assert_eq!(session.url_request, restored.url_request);
        assert_eq!(session.url_master, restored.url_master);
        assert_eq!(session.url_price, restored.url_price);
        assert_eq!(session.url_event, restored.url_event);
        assert_eq!(session.url_event_ws, restored.url_event_ws);
    }

    // ── Cycle V1: validate_session — セッション有効 ────────────────────────────

    #[tokio::test]
    async fn validate_session_returns_ok_when_session_valid() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"p_errno":"0","p_err":"","sResultCode":"0","sResultText":""}"#)
            .create_async()
            .await;

        let session = TachibanaSession {
            url_request: String::new(),
            url_master: String::new(),
            url_price: server.url(),
            url_event: String::new(),
            url_event_ws: String::new(),
        };

        let client = reqwest::Client::new();
        let result = validate_session(&client, &session).await;
        assert!(result.is_ok(), "有効なセッションは Ok を返すべき");
    }

    // ── Cycle V2: validate_session — セッション失効 ────────────────────────────

    #[tokio::test]
    async fn validate_session_returns_err_when_session_expired() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"p_errno":"2","p_err":"セッション失効","sResultCode":"","sResultText":""}"#,
            )
            .create_async()
            .await;

        let session = TachibanaSession {
            url_request: String::new(),
            url_master: String::new(),
            url_price: server.url(),
            url_event: String::new(),
            url_event_ws: String::new(),
        };

        let client = reqwest::Client::new();
        let result = validate_session(&client, &session).await;
        assert!(result.is_err(), "失効セッションは Err を返すべき");
    }

    // ── Cycle V3: validate_session — 未知の p_errno ────────────────────────────

    #[tokio::test]
    async fn validate_session_returns_err_on_unknown_p_errno() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"p_errno":"99","p_err":"不明なエラー","sResultCode":"","sResultText":""}"#,
            )
            .create_async()
            .await;

        let session = TachibanaSession {
            url_request: String::new(),
            url_master: String::new(),
            url_price: server.url(),
            url_event: String::new(),
            url_event_ws: String::new(),
        };

        let client = reqwest::Client::new();
        let result = validate_session(&client, &session).await;
        assert!(result.is_err(), "未知の p_errno は Err を返すべき");
    }

    // ── EVENT I/F パーサーテスト ────────────────────────────────────────────

    #[test]
    fn parse_event_frame_basic() {
        // 実際のデータ形式: p_{行番号}_{情報コード}
        let data = "\x01p_1_DPP\x023250\x01p_1_GAP1\x02500\x01p_1_GBP1\x023249";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("p_1_DPP", "3250"));
        assert_eq!(fields[1], ("p_1_GAP1", "500"));
        assert_eq!(fields[2], ("p_1_GBP1", "3249"));
    }

    #[test]
    fn parse_event_frame_empty_data() {
        let fields = parse_event_frame("");
        assert!(fields.is_empty());
    }

    #[test]
    fn parse_event_frame_no_stx_skips_record() {
        let data = "\x01novalue\x01pDPP\x023250";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("pDPP", "3250"));
    }

    #[test]
    fn parse_event_frame_empty_column_name_skipped() {
        let data = "\x01\x02value\x01pDPP\x023250";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("pDPP", "3250"));
    }

    #[test]
    fn fields_to_depth_full_10_levels() {
        // 実際のデータ形式: p_{行番号}_{情報コード}、p_cmd=FD 必須
        let mut data = String::from("\x01p_cmd\x02FD");
        for i in 1..=10 {
            let ask_price = 3250 + i;
            let bid_price = 3250 - i;
            data.push_str(&format!(
                "\x01p_1_GAP{i}\x02{ask_price}\x01p_1_GAV{i}\x02{}\x01p_1_GBP{i}\x02{bid_price}\x01p_1_GBV{i}\x02{}",
                (i * 100),
                (i * 200),
            ));
        }
        let fields = parse_event_frame(&data);
        let depth = fields_to_depth(&fields).expect("板情報が返るべき");
        assert_eq!(depth.asks.len(), 10);
        assert_eq!(depth.bids.len(), 10);
        assert_eq!(depth.asks[0].price, 3251.0);
        assert_eq!(depth.asks[0].qty, 100.0);
        assert_eq!(depth.bids[0].price, 3249.0);
        assert_eq!(depth.bids[0].qty, 200.0);
    }

    #[test]
    fn fields_to_depth_partial_missing_levels() {
        let data = "\x01p_cmd\x02FD\x01p_1_GAP1\x023251\x01p_1_GAV1\x02100\x01p_1_GBP1\x023249\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("部分的な板情報でも返るべき");
        assert_eq!(depth.asks.len(), 1);
        assert_eq!(depth.bids.len(), 1);
    }

    #[test]
    fn fields_to_depth_returns_none_for_no_depth_data() {
        // p_cmd=KP なので板情報は None
        let data = "\x01p_cmd\x02KP\x01p_1_DPP\x023250\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_depth(&fields).is_none());
    }

    #[test]
    fn fields_to_depth_star_values_skipped() {
        let data = "\x01p_cmd\x02FD\x01p_1_GAP1\x02*\x01p_1_GAV1\x02100\x01p_1_GBP1\x023249\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("買気配のみでも返るべき");
        assert_eq!(depth.asks.len(), 0);
        assert_eq!(depth.bids.len(), 1);
    }

    #[test]
    fn fields_to_trade_basic() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023250\x01p_1_DV\x02500\x01p_1_DYSS\x021";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields).expect("Trade が返るべき");
        assert_eq!(trade.price.to_f32(), 3250.0);
        assert_eq!(trade.qty.to_f32_lossy(), 500.0);
        assert!(trade.is_sell);
    }

    #[test]
    fn fields_to_trade_buy_side() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023250\x01p_1_DV\x02300\x01p_1_DYSS\x023";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields).expect("Trade が返るべき");
        assert!(!trade.is_sell);
    }

    #[test]
    fn fields_to_trade_star_price_returns_none() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x02*\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none());
    }

    #[test]
    fn fields_to_trade_missing_qty_returns_none() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023250";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none());
    }

    #[test]
    fn fields_to_trade_returns_none_for_fd_cmd() {
        // p_cmd=FD なので Trade は None
        let data = "\x01p_cmd\x02FD\x01p_1_DPP\x023250\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none());
    }

    #[test]
    fn build_event_params_format() {
        let params = build_event_params("6501", "00");
        assert!(params.contains("p_issue_code=6501"));
        assert!(params.contains("p_mkt_code=00"));
        assert!(params.contains("p_evt_cmd=ST,KP,FD"));
        assert!(params.contains("p_board_no=1000"));
        assert!(params.contains("p_eno=0"));
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: EVENT I/F パーサー堅牢性
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn parse_event_frame_etx_sub_delimiter_preserved_in_value() {
        // ETX (\x03) は値のサブ区切り。値文字列にそのまま含まれる
        let data = "\x01p_1_QAS\x020101\x03extra";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "p_1_QAS");
        assert_eq!(fields[0].1, "0101\x03extra");
    }

    #[test]
    fn parse_event_frame_multiple_stx_uses_first_split_only() {
        // STX が値の中にもある場合、最初の STX でのみ分割（splitn(2)）
        let data = "\x01p_1_DPP\x023250\x02extra";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "p_1_DPP");
        assert_eq!(fields[0].1, "3250\x02extra");
    }

    #[test]
    fn parse_event_frame_real_69_field_fd_frame() {
        // 計画書セクション2.6 記載の実データフィールド69件を模擬
        let mut data = String::new();
        let field_names = [
            "p_no",
            "p_date",
            "p_cmd",
            "p_1_AV",
            "p_1_BV",
            "p_1_DHF",
            "p_1_DHP",
            "p_1_DHP:T",
            "p_1_DJ",
            "p_1_DLF",
            "p_1_DLP",
            "p_1_DLP:T",
            "p_1_DOP",
            "p_1_DOP:T",
            "p_1_DPG",
            "p_1_DPP",
            "p_1_DPP:T",
            "p_1_DV",
            "p_1_DYRP",
            "p_1_DYWP",
        ];
        for (i, name) in field_names.iter().enumerate() {
            data.push_str(&format!("\x01{}\x02val{}", name, i));
        }
        // GAP1..10, GAV1..10, GBP1..10, GBV1..10 = 40 fields
        for i in 1..=10 {
            data.push_str(&format!("\x01p_1_GAP{}\x02{}", i, 3319 + i));
            data.push_str(&format!("\x01p_1_GAV{}\x02{}", i, 10000 + i * 100));
            data.push_str(&format!("\x01p_1_GBP{}\x02{}", i, 3318 - i));
            data.push_str(&format!("\x01p_1_GBV{}\x02{}", i, 6300 + i * 50));
        }
        // 残りフィールド
        for name in &[
            "p_1_LISS", "p_1_PRP", "p_1_QAP", "p_1_QAS", "p_1_QBP", "p_1_QBS", "p_1_QOV",
            "p_1_QUV", "p_1_VWAP",
        ] {
            data.push_str(&format!("\x01{}\x021234", name));
        }
        let fields = parse_event_frame(&data);
        // 20 + 40 + 9 = 69
        assert_eq!(fields.len(), 69, "69フィールドすべてパースされるべき");
    }

    #[test]
    fn parse_event_frame_colon_in_field_name() {
        // 実データでは p_1_DPP:T のようなコロン付きフィールド名がある
        let data = "\x01p_1_DPP:T\x0215:00:00";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "p_1_DPP:T");
        assert_eq!(fields[0].1, "15:00:00");
    }

    #[test]
    fn parse_event_frame_consecutive_soh_skips_empty_records() {
        let data = "\x01\x01p_1_DPP\x023250\x01\x01";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("p_1_DPP", "3250"));
    }

    #[test]
    fn parse_event_frame_value_is_empty_string() {
        let data = "\x01p_1_DPP\x02";
        let fields = parse_event_frame(data);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("p_1_DPP", ""));
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: fields_to_depth 堅牢性
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn fields_to_depth_non_contiguous_levels() {
        // GAP1 あり、GAP2 欠損、GAP3 あり → 2件のみ
        let data = "\x01p_cmd\x02FD\
                     \x01p_1_GAP1\x023320\x01p_1_GAV1\x02100\
                     \x01p_1_GAP3\x023322\x01p_1_GAV3\x02300\
                     \x01p_1_GBP1\x023318\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("板情報が返るべき");
        assert_eq!(depth.asks.len(), 2, "GAP1+GAP3 の2件のみ");
        assert_eq!(depth.bids.len(), 1);
        assert_eq!(depth.asks[0].price, 3320.0);
        assert_eq!(depth.asks[1].price, 3322.0);
    }

    #[test]
    fn fields_to_depth_zero_qty_is_valid() {
        // 数量0は「板が消えた」意味で有効な値
        let data = "\x01p_cmd\x02FD\
                     \x01p_1_GAP1\x023320\x01p_1_GAV1\x020\
                     \x01p_1_GBP1\x023318\x01p_1_GBV1\x020";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("0数量でも板情報は返るべき");
        assert_eq!(depth.asks[0].qty, 0.0);
        assert_eq!(depth.bids[0].qty, 0.0);
    }

    #[test]
    fn fields_to_depth_large_over_under_quantities() {
        // OVER/UNDER（QOV/QUV）は板集計量。数百万の値でもパースエラーにならない
        let data = "\x01p_cmd\x02FD\
                     \x01p_1_GAP1\x023319\x01p_1_GAV1\x0210000\
                     \x01p_1_GBP1\x023318\x01p_1_GBV1\x026300\
                     \x01p_1_QOV\x024218600\x01p_1_QUV\x022520200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("大きな値でも板情報が返るべき");
        assert_eq!(depth.asks.len(), 1);
        assert_eq!(depth.bids.len(), 1);
    }

    #[test]
    fn fields_to_depth_returns_none_when_no_cmd_field() {
        // p_cmd フィールド自体が無い場合
        let data = "\x01p_1_GAP1\x023320\x01p_1_GAV1\x02100";
        let fields = parse_event_frame(data);
        assert!(fields_to_depth(&fields).is_none(), "p_cmd なしでは None");
    }

    #[test]
    fn fields_to_depth_returns_none_for_st_cmd() {
        let data = "\x01p_cmd\x02ST\x01p_1_GAP1\x023320\x01p_1_GAV1\x02100";
        let fields = parse_event_frame(data);
        assert!(
            fields_to_depth(&fields).is_none(),
            "ST コマンドでは板情報を返さない"
        );
    }

    #[test]
    fn fields_to_depth_empty_value_skipped() {
        let data = "\x01p_cmd\x02FD\
                     \x01p_1_GAP1\x02\x01p_1_GAV1\x02100\
                     \x01p_1_GBP1\x023318\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("買気配のみ返るべき");
        assert_eq!(depth.asks.len(), 0, "空の価格はスキップ");
        assert_eq!(depth.bids.len(), 1);
    }

    #[test]
    fn fields_to_depth_price_without_matching_qty_skipped() {
        // GAP1 はあるが GAV1 が無い → ペアにならないのでスキップ
        let data = "\x01p_cmd\x02FD\x01p_1_GAP1\x023320\x01p_1_GBP1\x023318\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("買気配のみ返るべき");
        assert_eq!(depth.asks.len(), 0, "数量ペアなし売気配はスキップ");
        assert_eq!(depth.bids.len(), 1);
    }

    #[test]
    fn fields_to_depth_preserves_index_order() {
        // asks は GAP1→GAP10 の順序で格納されることを確認
        let mut data = String::from("\x01p_cmd\x02FD");
        for i in 1..=5 {
            let p = 3320 + i;
            data.push_str(&format!("\x01p_1_GAP{i}\x02{p}\x01p_1_GAV{i}\x02100"));
        }
        let fields = parse_event_frame(&data);
        let depth = fields_to_depth(&fields).unwrap();
        for i in 0..4 {
            assert!(
                depth.asks[i].price < depth.asks[i + 1].price,
                "asks はインデックス順（価格昇順）であるべき: idx={i}"
            );
        }
    }

    #[test]
    fn fields_to_depth_invalid_number_skipped() {
        let data = "\x01p_cmd\x02FD\
                     \x01p_1_GAP1\x02abc\x01p_1_GAV1\x02100\
                     \x01p_1_GBP1\x023318\x01p_1_GBV1\x02200";
        let fields = parse_event_frame(data);
        let depth = fields_to_depth(&fields).expect("有効な買気配のみ返るべき");
        assert_eq!(depth.asks.len(), 0, "数値でない価格はスキップ");
        assert_eq!(depth.bids.len(), 1);
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: fields_to_trade 堅牢性
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn fields_to_trade_missing_dyss_defaults_to_buy() {
        // DYSS フィールドなし → is_sell = false（買いとみなす）
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023250\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields).expect("DYSS なしでも Trade が返るべき");
        assert!(!trade.is_sell, "DYSS なしはデフォルト買い");
    }

    #[test]
    fn fields_to_trade_returns_none_for_kp_cmd() {
        let data = "\x01p_cmd\x02KP\x01p_1_DPP\x023250\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(
            fields_to_trade(&fields).is_none(),
            "KP コマンドでは Trade を返さない"
        );
    }

    #[test]
    fn fields_to_trade_returns_none_when_no_cmd() {
        let data = "\x01p_1_DPP\x023250\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none(), "p_cmd なしでは None");
    }

    #[test]
    fn fields_to_trade_empty_price_returns_none() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x02\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none(), "空価格は None");
    }

    #[test]
    fn fields_to_trade_empty_qty_returns_none() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023250\x01p_1_DV\x02";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none(), "空数量は None");
    }

    #[test]
    fn fields_to_trade_invalid_price_returns_none() {
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x02abc\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        assert!(fields_to_trade(&fields).is_none(), "非数値価格は None");
    }

    #[test]
    fn fields_to_trade_suffix_xdpp_does_not_match_dpp() {
        // p_1_XDPP は ends_with("_DPP") = false（末尾は "XDPP"）
        // よって _DPP フィールドが見つからず Trade は None
        let data = "\x01p_cmd\x02ST\x01p_1_XDPP\x029999\x01p_1_DV\x02500";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields);
        assert!(
            trade.is_none(),
            "_XDPP は _DPP 末尾マッチにヒットしない（安全）"
        );
    }

    #[test]
    fn fields_to_trade_large_quantity() {
        // 出来高が数千万の場合でもパースできる
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x023319\x01p_1_DV\x0216930900\x01p_1_DYSS\x021";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields).expect("大きな数量でも Trade が返るべき");
        assert_eq!(trade.qty.to_f32_lossy(), 16930900.0);
    }

    #[test]
    fn fields_to_trade_fractional_price() {
        // 小数点価格（ETF など）
        let data = "\x01p_cmd\x02ST\x01p_1_DPP\x021234.5\x01p_1_DV\x02100";
        let fields = parse_event_frame(data);
        let trade = fields_to_trade(&fields).expect("小数価格でも Trade が返るべき");
        assert!((trade.price.to_f32() - 1234.5).abs() < 0.1);
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: build_event_params パラメータ順序
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn build_event_params_preserves_mandatory_order() {
        // 公式サンプル準拠: p_rid が先頭、p_issue_code が最後（順番変更不可）
        let params = build_event_params("7203", "00");
        let parts: Vec<&str> = params.split('&').collect();
        assert!(
            parts[0].starts_with("p_rid="),
            "先頭は p_rid であるべき: {:?}",
            parts[0]
        );
        assert!(
            parts.last().unwrap().starts_with("p_issue_code="),
            "最後は p_issue_code であるべき: {:?}",
            parts.last()
        );
        // 全7パラメータ
        assert_eq!(parts.len(), 7, "パラメータは7個であるべき");
    }

    #[test]
    fn build_event_params_different_market_code() {
        let params = build_event_params("7203", "01");
        assert!(params.contains("p_mkt_code=01"));
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: master_record_to_ticker_info
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn master_record_to_ticker_info_basic_kabu_record() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstKabu".to_string(),
            issue_code: "7203".to_string(),
            issue_name: "トヨタ自動車".to_string(),
            issue_name_short: "トヨタ".to_string(),
            issue_name_kana: "トヨタジドウシャ".to_string(),
            issue_name_english: "TOYOTA MOTOR".to_string(),
            primary_market: "00".to_string(),
            sector_code: "3050".to_string(),
            sector_name: "輸送用機器".to_string(),
        };
        let (ticker, info) = master_record_to_ticker_info(&record).expect("変換できるべき");
        let (symbol, _) = ticker.to_full_symbol_and_type();
        assert_eq!(symbol, "7203");
        assert_eq!(info.ticker, ticker);
    }

    #[test]
    fn master_record_to_ticker_info_non_kabu_returns_none() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstFuture".to_string(),
            issue_code: "1234".to_string(),
            issue_name: String::new(),
            issue_name_short: String::new(),
            issue_name_kana: String::new(),
            issue_name_english: "SOME FUTURE".to_string(),
            primary_market: String::new(),
            sector_code: String::new(),
            sector_name: String::new(),
        };
        assert!(
            master_record_to_ticker_info(&record).is_none(),
            "非 CLMIssueMstKabu は None"
        );
    }

    #[test]
    fn master_record_to_ticker_info_empty_issue_code_returns_none() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstKabu".to_string(),
            issue_code: String::new(),
            issue_name: String::new(),
            issue_name_short: String::new(),
            issue_name_kana: String::new(),
            issue_name_english: "SOME STOCK".to_string(),
            primary_market: String::new(),
            sector_code: String::new(),
            sector_name: String::new(),
        };
        assert!(
            master_record_to_ticker_info(&record).is_none(),
            "空 issue_code は None"
        );
    }

    #[test]
    fn master_record_to_ticker_info_empty_english_name_uses_no_display() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstKabu".to_string(),
            issue_code: "9999".to_string(),
            issue_name: "テスト銘柄".to_string(),
            issue_name_short: String::new(),
            issue_name_kana: String::new(),
            issue_name_english: String::new(), // 英語名なし
            primary_market: String::new(),
            sector_code: String::new(),
            sector_name: String::new(),
        };
        let result = master_record_to_ticker_info(&record);
        assert!(result.is_some(), "英語名なしでも変換可能であるべき");
    }

    #[test]
    fn master_record_to_ticker_info_long_english_name_truncated() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstKabu".to_string(),
            issue_code: "8001".to_string(),
            issue_name: String::new(),
            issue_name_short: String::new(),
            issue_name_kana: String::new(),
            issue_name_english: "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890".to_string(), // 36文字 > 28
            primary_market: String::new(),
            sector_code: String::new(),
            sector_name: String::new(),
        };
        // 28文字に切り捨てられてもパニックしない
        let result = master_record_to_ticker_info(&record);
        assert!(result.is_some(), "長い名前は切り捨てて変換可能であるべき");
    }

    #[test]
    fn master_record_to_ticker_info_non_ascii_english_name_falls_back() {
        let record = MasterRecord {
            clm_id: "CLMIssueMstKabu".to_string(),
            issue_code: "8002".to_string(),
            issue_name: String::new(),
            issue_name_short: String::new(),
            issue_name_kana: String::new(),
            issue_name_english: "日本語名".to_string(), // 非ASCII
            primary_market: String::new(),
            sector_code: String::new(),
            sector_name: String::new(),
        };
        // 非ASCII の display_symbol は None にフォールバック → パニックしない
        let result = master_record_to_ticker_info(&record);
        assert!(
            result.is_some(),
            "非ASCII英語名でもパニックせず変換可能であるべき"
        );
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: Event URL 管理
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn set_and_get_event_http_url() {
        set_event_http_url("https://event.test/streaming/".to_string());
        let url = get_event_http_url().expect("設定した URL が取得できるべき");
        assert_eq!(url, "https://event.test/streaming/");
    }

    #[test]
    fn set_and_get_event_ws_url() {
        set_event_ws_url("wss://ws.test/event/".to_string());
        let url = get_event_ws_url().expect("設定した URL が取得できるべき");
        assert_eq!(url, "wss://ws.test/event/");
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: 実データ統合テスト（FD/ST/KP ディスパッチ）
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn real_data_fd_frame_produces_depth_and_no_trade() {
        // TOYOTA (7203) 実データ相当のFDフレーム
        let mut data = String::from("\x01p_no\x0212345\x01p_date\x0220260410\x01p_cmd\x02FD");
        // 売気配10本（GAP/GAV）
        for i in 1..=10 {
            data.push_str(&format!("\x01p_1_GAP{}\x02{}", i, 3319 + i));
            data.push_str(&format!("\x01p_1_GAV{}\x02{}", i, 10000 + i * 930));
        }
        // 買気配10本（GBP/GBV）
        for i in 1..=10 {
            data.push_str(&format!("\x01p_1_GBP{}\x02{}", i, 3318 - i));
            data.push_str(&format!("\x01p_1_GBV{}\x02{}", i, 6300 + i * 400));
        }
        // FDフレーム内の終値・出来高（Tradeとして誤パースされてはならない）
        data.push_str("\x01p_1_DPP\x023319\x01p_1_DV\x0216930900");

        let fields = parse_event_frame(&data);

        // 板情報が正しく取得できること
        let depth = fields_to_depth(&fields).expect("FDフレームから板情報が返るべき");
        assert_eq!(depth.asks.len(), 10, "売気配10本");
        assert_eq!(depth.bids.len(), 10, "買気配10本");
        assert_eq!(depth.asks[0].price, 3320.0, "最良売気配は GAP1");
        assert_eq!(depth.bids[0].price, 3317.0, "最良買気配は GBP1");

        // FDフレームからTradeが生成されないこと（クラッシュ防止の回帰テスト）
        assert!(
            fields_to_trade(&fields).is_none(),
            "FDフレームの DPP/DV から Trade が生成されてはならない"
        );
    }

    #[test]
    fn real_data_st_frame_produces_trade_and_no_depth() {
        let data = "\x01p_no\x0212346\x01p_date\x0220260410\x01p_cmd\x02ST\
                     \x01p_1_DPP\x023319\x01p_1_DV\x02500\x01p_1_DPP:T\x0209:00:01\x01p_1_DYSS\x021";
        let fields = parse_event_frame(data);

        let trade = fields_to_trade(&fields).expect("STフレームから Trade が返るべき");
        assert_eq!(trade.price.to_f32(), 3319.0);
        assert_eq!(trade.qty.to_f32_lossy(), 500.0);
        assert!(trade.is_sell);

        assert!(
            fields_to_depth(&fields).is_none(),
            "STフレームから板情報が返ってはならない"
        );
    }

    #[test]
    fn real_data_kp_frame_produces_neither_depth_nor_trade() {
        let data = "\x01p_no\x0212347\x01p_date\x0220260410\x01p_cmd\x02KP\
                     \x01p_1_DPP\x023319\x01p_1_DV\x0216930900";
        let fields = parse_event_frame(data);

        assert!(
            fields_to_depth(&fields).is_none(),
            "KPフレームから板情報は返らない"
        );
        assert!(
            fields_to_trade(&fields).is_none(),
            "KPフレームから Trade は返らない"
        );
    }

    #[test]
    fn mixed_frame_sequence_dispatches_correctly() {
        // 連続する FD → ST → KP フレームを順に処理
        let fd_data = "\x01p_cmd\x02FD\x01p_1_GAP1\x023320\x01p_1_GAV1\x02100\x01p_1_GBP1\x023318\x01p_1_GBV1\x02200";
        let st_data = "\x01p_cmd\x02ST\x01p_1_DPP\x023319\x01p_1_DV\x02300\x01p_1_DYSS\x021";
        let kp_data = "\x01p_cmd\x02KP\x01p_1_DPP\x023319\x01p_1_DV\x0216930900";

        let frames = [fd_data, st_data, kp_data];
        let mut depth_count = 0;
        let mut trade_count = 0;

        for frame in &frames {
            let fields = parse_event_frame(frame);
            if fields_to_depth(&fields).is_some() {
                depth_count += 1;
            }
            if fields_to_trade(&fields).is_some() {
                trade_count += 1;
            }
        }

        assert_eq!(depth_count, 1, "FDフレームのみ板情報を生成");
        assert_eq!(trade_count, 1, "STフレームのみ Trade を生成");
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: date_str_to_epoch_ms エッジケース
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn date_str_to_epoch_ms_valid_date() {
        let ms = date_str_to_epoch_ms("20240101").expect("有効な日付");
        // 2024-01-01 00:00:00 JST = 2023-12-31 15:00:00 UTC = 1704034800000ms
        assert_eq!(ms, 1704034800000);
    }

    #[test]
    fn date_str_to_epoch_ms_invalid_length() {
        assert!(date_str_to_epoch_ms("2024010").is_none(), "7文字は無効");
        assert!(date_str_to_epoch_ms("202401012").is_none(), "9文字は無効");
        assert!(date_str_to_epoch_ms("").is_none(), "空文字列は無効");
    }

    #[test]
    fn date_str_to_epoch_ms_invalid_month() {
        assert!(date_str_to_epoch_ms("20241301").is_none(), "13月は無効");
    }

    #[test]
    fn date_str_to_epoch_ms_invalid_day() {
        assert!(date_str_to_epoch_ms("20240230").is_none(), "2月30日は無効");
    }

    #[test]
    fn date_str_to_epoch_ms_non_numeric() {
        assert!(date_str_to_epoch_ms("abcdefgh").is_none(), "非数値は無効");
    }

    #[test]
    fn date_str_to_epoch_ms_leap_year() {
        // 2024年はうるう年: 2月29日は有効
        assert!(
            date_str_to_epoch_ms("20240229").is_some(),
            "うるう年の2月29日は有効"
        );
        // 2023年は非うるう年: 2月29日は無効
        assert!(
            date_str_to_epoch_ms("20230229").is_none(),
            "非うるう年の2月29日は無効"
        );
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: daily_record_to_kline エッジケース
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn daily_record_returns_none_when_all_fields_asterisk() {
        let record = DailyHistoryRecord {
            date: "20240101".to_string(),
            open: "*".to_string(),
            high: "*".to_string(),
            low: "*".to_string(),
            close: "*".to_string(),
            volume: "*".to_string(),
            open_adj: String::new(),
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };
        assert!(daily_record_to_kline(&record, false).is_none());
    }

    #[test]
    fn daily_record_returns_none_when_adjusted_fields_empty_and_use_adjusted() {
        let record = DailyHistoryRecord {
            date: "20200101".to_string(),
            open: "6400".to_string(),
            high: "6560".to_string(),
            low: "6300".to_string(),
            close: "6500".to_string(),
            volume: "750000".to_string(),
            open_adj: String::new(), // 空
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };
        assert!(
            daily_record_to_kline(&record, true).is_none(),
            "調整値が空で use_adjusted=true なら None"
        );
    }

    #[test]
    fn daily_record_invalid_date_returns_none() {
        let record = DailyHistoryRecord {
            date: "invalid!".to_string(),
            open: "100".to_string(),
            high: "110".to_string(),
            low: "90".to_string(),
            close: "105".to_string(),
            volume: "1000".to_string(),
            open_adj: String::new(),
            high_adj: String::new(),
            low_adj: String::new(),
            close_adj: String::new(),
            volume_adj: String::new(),
        };
        assert!(
            daily_record_to_kline(&record, false).is_none(),
            "不正な日付は None"
        );
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: ApiResponse エッジケース
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn api_response_check_passes_when_both_codes_empty() {
        let json = r#"{
            "p_errno": "",
            "p_err": "",
            "sResultCode": "",
            "sResultText": "",
            "aCLMMfdsMarketPrice": [{"sIssueCode": "6501", "pDPP": "3250"}]
        }"#;
        let resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(json).unwrap();
        let data = resp.check().unwrap();
        assert_eq!(data.records.len(), 1, "空コードは正常扱い");
    }

    #[test]
    fn api_response_p_errno_takes_precedence_over_result_code() {
        // p_errno がエラーなら sResultCode が "0" でもエラーになる
        let json = r#"{
            "p_errno": "2",
            "p_err": "セッション切断",
            "sResultCode": "0",
            "sResultText": "",
            "aCLMMfdsMarketPrice": []
        }"#;
        let resp: ApiResponse<MarketPriceResponse> = serde_json::from_str(json).unwrap();
        let result = resp.check();
        assert!(
            matches!(result, Err(TachibanaError::ApiError { ref code, .. }) if code == "2"),
            "p_errno が優先されるべき"
        );
    }

    // ══════════════════════════════════════════════════════════════════════
    // 追加テスト: LoginResponse TryFrom エッジケース
    // ══════════════════════════════════════════════════════════════════════

    #[test]
    fn session_creation_fails_on_p_errno_error() {
        let response = LoginResponse {
            clm_id: "CLMAuthLoginAck".to_string(),
            p_errno: "999".to_string(),
            p_err: "内部エラー".to_string(),
            result_code: "0".to_string(),
            url_request: String::new(),
            url_master: String::new(),
            url_price: String::new(),
            url_event: String::new(),
            url_event_ws: String::new(),
            unread_notice_flag: "0".to_string(),
            result_text: String::new(),
        };
        let result = TachibanaSession::try_from(response);
        assert!(
            matches!(result, Err(TachibanaError::LoginFailed(_))),
            "p_errno が 0 でない場合は LoginFailed が返るべき"
        );
    }

    #[test]
    fn session_creation_succeeds_with_empty_p_errno() {
        let response = LoginResponse {
            clm_id: "CLMAuthLoginAck".to_string(),
            p_errno: String::new(), // 空
            p_err: String::new(),
            result_code: "0".to_string(),
            url_request: "https://r.test/".to_string(),
            url_master: "https://m.test/".to_string(),
            url_price: "https://p.test/".to_string(),
            url_event: "https://e.test/".to_string(),
            url_event_ws: "wss://ws.test/".to_string(),
            unread_notice_flag: "0".to_string(),
            result_text: String::new(),
        };
        let session = TachibanaSession::try_from(response).expect("空 p_errno は成功すべき");
        assert_eq!(session.url_price, "https://p.test/");
    }

    // ── Cycle XX: Shift-JIS マスタストリーム解析 ─────────────────────────────

    /// ASCII のみの2件レコードが `}` で正しく分割される基本ケース
    #[test]
    fn parse_sjis_stream_records_splits_ascii_records_at_brace() {
        let data = b"abc}def}";
        let records = parse_sjis_stream_records(data);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], b"abc}");
        assert_eq!(records[1], b"def}");
    }

    /// Shift-JIS リードバイト (0x81-0x9F 範囲) の直後に来る 0x7D は
    /// トレイルバイトであり、レコード境界として扱ってはならない。
    #[test]
    fn parse_sjis_stream_records_does_not_split_on_sjis_trail_byte_0x7d() {
        // Shift-JIS 2バイト文字: リードバイト 0x81 + トレイルバイト 0x7D (= ASCII `}`)
        // このトレイル 0x7D をレコード境界と誤認するバグを再現するテスト。
        let data: &[u8] = &[b'{', b'"', b'x', b'"', b':', b'"', 0x81, 0x7d, b'"', b'}'];
        let records = parse_sjis_stream_records(data);
        assert_eq!(
            records.len(),
            1,
            "Shift-JIS トレイルバイト 0x7D をレコード境界としてはならない; {} 件に分割された",
            records.len()
        );
        assert_eq!(records[0], data);
    }

    /// リードバイト 0xE0-0xEF 範囲でも同様にトレイル 0x7D を境界扱いしない
    #[test]
    fn parse_sjis_stream_records_handles_e0_range_lead_byte() {
        let data: &[u8] = &[0xE0, 0x7d, b'}'];
        let records = parse_sjis_stream_records(data);
        assert_eq!(
            records.len(),
            1,
            "0xE0 リードバイト後の 0x7D も境界外; {} 件に分割された",
            records.len()
        );
    }

    /// Shift-JIS 文字を含む1件目と ASCII のみの2件目が正しく分割される
    #[test]
    fn parse_sjis_stream_records_two_records_with_sjis_in_first() {
        let mut data = Vec::new();
        data.extend_from_slice(&[b'A', 0x81, 0x7d, b'}']); // 1件目: Shift-JIS 0x81 0x7D を含む
        data.extend_from_slice(&[b'B', b'}']); // 2件目: ASCII のみ
        let records = parse_sjis_stream_records(&data);
        assert_eq!(records.len(), 2, "正確に2件に分割されるべき; {} 件", records.len());
        assert_eq!(records[0], &[b'A', 0x81, 0x7d, b'}']);
        assert_eq!(records[1], &[b'B', b'}']);
    }

    /// 末尾に `}` がない残余データもそのまま返す
    #[test]
    fn parse_sjis_stream_records_returns_trailing_incomplete_data() {
        let data = b"abc}incomplete";
        let records = parse_sjis_stream_records(data);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], b"abc}");
        assert_eq!(records[1], b"incomplete");
    }
}
