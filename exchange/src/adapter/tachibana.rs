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
        log::warn!("Shift-JIS decode produced lossy output ({} bytes)", bytes.len());
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
use std::sync::Arc;
use tokio::sync::RwLock;

/// 全マスタダウンロードの各レコードをパースするための汎用型。
/// sCLMID でレコード種別を判定し、CLMIssueMstKabu のみ抽出する。
#[derive(Debug, Deserialize, Clone)]
pub struct MasterRecord {
    #[serde(rename = "sCLMID")]
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

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        for &byte in chunk.iter() {
            buf.push(byte);
            if byte == b'}' {
                // `}` でレコード境界を判定（サンプルコード準拠）
                let (decoded, _, had_errors) = encoding_rs::SHIFT_JIS.decode(&buf);
                if had_errors {
                    log::warn!("Shift-JIS decode produced lossy output in master stream ({} bytes)", buf.len());
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

static ISSUE_MASTER_CACHE: RwLock<Option<Arc<Vec<MasterRecord>>>> = RwLock::const_new(None);

/// ログイン成功時に呼び出し、銘柄マスタをキャッシュに格納する。
pub async fn init_issue_master(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(), TachibanaError> {
    let records = fetch_all_master(client, session).await?;
    *ISSUE_MASTER_CACHE.write().await = Some(Arc::new(records));
    Ok(())
}

/// キャッシュ済みの銘柄マスタを返す。未取得なら None。
pub async fn get_cached_issue_master() -> Option<Arc<Vec<MasterRecord>>> {
    ISSUE_MASTER_CACHE.read().await.clone()
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
pub async fn cached_ticker_metadata() -> HashMap<Ticker, Option<TickerInfo>> {
    let mut out = HashMap::new();
    let cache = get_cached_issue_master().await;
    if let Some(records) = cache {
        for record in records.iter() {
            if let Some((ticker, info)) = master_record_to_ticker_info(record) {
                out.insert(ticker, Some(info));
            }
        }
    }
    out
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
        let handles: Vec<_> = (0..10)
            .map(|_| std::thread::spawn(|| next_p_no()))
            .collect();
        let values: HashSet<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(values.len(), 10, "並行呼び出しでも全 p_no がユニークであるべき");
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
}
