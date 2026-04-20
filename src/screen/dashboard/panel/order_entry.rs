use exchange::adapter::tachibana::NewOrderRequest;
use iced::{
    Alignment, Element, Theme,
    widget::{button, column, container, pick_list, row, text, text_input},
};

// ── 定数 ─────────────────────────────────────────────────────────────────────

const DEFAULT_MARKET_CODE: &str = "00"; // 東証

// ── ドメイン型 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "買い"),
            Side::Sell => write!(f, "売り"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceType {
    Market,
    Limit,
}

impl std::fmt::Display for PriceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriceType::Market => write!(f, "成行"),
            PriceType::Limit => write!(f, "指値"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountType {
    Tokutei,
    Ippan,
    Nisa,
    NisaGrowth,
}

impl AccountType {
    pub const ALL: [AccountType; 4] = [
        AccountType::Tokutei,
        AccountType::Ippan,
        AccountType::Nisa,
        AccountType::NisaGrowth,
    ];

    /// API で使う sZyoutoekiKazeiC の値
    pub fn api_code(&self) -> &'static str {
        match self {
            AccountType::Tokutei => "1",
            AccountType::Ippan => "3",
            AccountType::Nisa => "5",
            AccountType::NisaGrowth => "6",
        }
    }
}

impl std::fmt::Display for AccountType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AccountType::Tokutei => "特定",
            AccountType::Ippan => "一般",
            AccountType::Nisa => "NISA",
            AccountType::NisaGrowth => "N成長",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CashMarginType {
    Cash,
    MarginNew6M,
    MarginClose6M,
    MarginNewGeneral,
    MarginCloseGeneral,
}

impl CashMarginType {
    pub const ALL: [CashMarginType; 5] = [
        CashMarginType::Cash,
        CashMarginType::MarginNew6M,
        CashMarginType::MarginClose6M,
        CashMarginType::MarginNewGeneral,
        CashMarginType::MarginCloseGeneral,
    ];

    /// API で使う sGenkinShinyouKubun の値
    pub fn api_code(&self) -> &'static str {
        match self {
            CashMarginType::Cash => "0",
            CashMarginType::MarginNew6M => "2",
            CashMarginType::MarginClose6M => "4",
            CashMarginType::MarginNewGeneral => "6",
            CashMarginType::MarginCloseGeneral => "8",
        }
    }
}

impl std::fmt::Display for CashMarginType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CashMarginType::Cash => "現物",
            CashMarginType::MarginNew6M => "信新(制度)",
            CashMarginType::MarginClose6M => "信返(制度)",
            CashMarginType::MarginNewGeneral => "信新(一般)",
            CashMarginType::MarginCloseGeneral => "信返(一般)",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpireDay {
    Today,
    Specified(String),
}

impl std::fmt::Display for ExpireDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExpireDay::Today => write!(f, "当日"),
            ExpireDay::Specified(d) => write!(f, "{d}"),
        }
    }
}

// ── 注文結果型 ────────────────────────────────────────────────────────────────

/// 注文が受け付けられたことを示す成功値。
/// 警告ありでも受付済みなら Ok に包む。
#[derive(Debug, Clone)]
pub struct OrderSuccess {
    pub order_num: String,
    /// 警告コード/テキストがあれば Some、なければ None
    pub warning: Option<String>,
}

pub type OrderResult = Result<OrderSuccess, String>;

// ── パネル状態 ─────────────────────────────────────────────────────────────────

pub struct OrderEntryPanel {
    // 銘柄情報
    pub issue_code: String,

    // 入力フォーム
    pub side: Side,
    pub account_type: AccountType,
    pub qty: String,
    pub price_type: PriceType,
    pub limit_price: String,
    pub tick_size: Option<f64>,
    pub cash_margin: CashMarginType,
    pub expire_day: ExpireDay,

    // 保有株数（売り選択時に表示）
    pub holdings: Option<u64>,

    // 認証
    pub second_password: String,

    // UI 状態
    pub confirm_modal: bool,
    pub loading: bool,
    pub last_result: Option<OrderResult>,
    /// true = REPLAYモード（仮想注文）。pane.rs の is_virtual_mode に連動する。
    pub is_virtual: bool,
    /// 銘柄選択モーダル
    pub modal: Option<crate::modal::pane::mini_tickers_list::MiniPanel>,
}

#[derive(Debug, Clone)]
pub enum Message {
    SideChanged(Side),
    AccountTypeChanged(AccountType),
    QtyChanged(String),
    /// 「全数量」ボタン: holdings を qty にセット
    FillFromHoldings,
    PriceTypeChanged(PriceType),
    LimitPriceChanged(String),
    /// 「▲」ボタン: limit_price を呼値単位で +1
    PriceIncrementTick,
    /// 「▼」ボタン: limit_price を呼値単位で -1
    PriceDecrementTick,
    CashMarginChanged(CashMarginType),
    ExpireDayChanged(ExpireDay),
    SecondPasswordChanged(String),
    /// 保有株数の取得結果
    HoldingsUpdated(Option<u64>),
    /// 確認モーダルを開く
    ConfirmClicked,
    /// 確認モーダルを閉じる
    ConfirmCancelled,
    /// 実際に発注 Effect を発行
    Submitted,
    /// API 応答を受け取り UI を更新
    OrderCompleted(OrderResult),
    /// 銘柄選択モーダルを開く
    OpenTickerSearch,
    /// MiniTickersList からのメッセージを委譲
    MiniTickers(crate::modal::pane::mini_tickers_list::Message),
}

#[derive(Debug)]
pub enum Action {
    Submit(Box<NewOrderRequest>),
    FetchHoldings { issue_code: String },
}

// ── 実装 ──────────────────────────────────────────────────────────────────────

impl Default for OrderEntryPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderEntryPanel {
    pub fn new() -> Self {
        Self {
            issue_code: String::new(),
            side: Side::Buy,
            account_type: AccountType::Tokutei,
            qty: String::new(),
            price_type: PriceType::Market,
            limit_price: String::new(),
            tick_size: None,
            cash_margin: CashMarginType::Cash,
            expire_day: ExpireDay::Today,
            holdings: None,
            second_password: String::new(),
            confirm_modal: false,
            loading: false,
            last_result: None,
            is_virtual: false,
            modal: None,
        }
    }

    // ── ロジック ───────────────────────────────────────────────────────────────

    /// 呼値単位で指値を +1 する。
    /// `limit_price` が空欄のとき best_ask（買い）または best_bid（売り）を初期値にする。
    /// `tick_size` が None の場合は何もしない。
    pub fn increment_price(&mut self) {
        let Some(tick) = self.tick_size else {
            return;
        };
        let current: f64 = self.limit_price.parse().unwrap_or(0.0);
        let new_price = current + tick;
        self.limit_price = format_price(new_price, tick);
    }

    /// 呼値単位で指値を -1 する。
    pub fn decrement_price(&mut self) {
        let Some(tick) = self.tick_size else {
            return;
        };
        let current: f64 = self.limit_price.parse().unwrap_or(0.0);
        let new_price = (current - tick).max(0.0);
        self.limit_price = format_price(new_price, tick);
    }

    /// 現在の入力状態から `NewOrderRequest` を構築する。
    /// 不正な入力があれば `Err` を返す。
    pub fn build_request(&self) -> Result<NewOrderRequest, String> {
        if self.issue_code.is_empty() {
            return Err("銘柄コードが未設定です".to_string());
        }
        if self.qty.is_empty() {
            return Err("数量を入力してください".to_string());
        }
        // 仮想モードではパスワード不要
        if !self.is_virtual && self.second_password.is_empty() {
            return Err("発注パスワードを入力してください".to_string());
        }

        let price = match self.price_type {
            PriceType::Market => "0".to_string(),
            PriceType::Limit => {
                if self.limit_price.is_empty() {
                    return Err("指値を入力してください".to_string());
                }
                self.limit_price.clone()
            }
        };

        let expire = match &self.expire_day {
            ExpireDay::Today => "0".to_string(),
            ExpireDay::Specified(d) => d.clone(),
        };

        Ok(NewOrderRequest {
            account_type: self.account_type.api_code().to_string(),
            issue_code: self.issue_code.clone(),
            market_code: DEFAULT_MARKET_CODE.to_string(),
            side: match self.side {
                Side::Buy => "3".to_string(),
                Side::Sell => "1".to_string(),
            },
            condition: "0".to_string(), // 指定なし（逆指値は将来フェーズ）
            price,
            qty: self.qty.clone(),
            cash_margin: self.cash_margin.api_code().to_string(),
            expire_day: expire,
            second_password: self.second_password.clone(),
        })
    }

    pub fn update(&mut self, msg: Message) -> Option<Action> {
        match msg {
            Message::SideChanged(side) => {
                let prev_side = self.side;
                self.side = side;
                // 売りに切り替えたとき保有株数を取得
                if side == Side::Sell && prev_side != Side::Sell && !self.issue_code.is_empty() {
                    return Some(Action::FetchHoldings {
                        issue_code: self.issue_code.clone(),
                    });
                }
            }
            Message::AccountTypeChanged(t) => self.account_type = t,
            Message::QtyChanged(s) => self.qty = s,
            Message::FillFromHoldings => {
                if let Some(h) = self.holdings {
                    self.qty = h.to_string();
                }
            }
            Message::PriceTypeChanged(pt) => self.price_type = pt,
            Message::LimitPriceChanged(s) => self.limit_price = s,
            Message::PriceIncrementTick => {
                if self.price_type == PriceType::Limit {
                    self.increment_price();
                }
            }
            Message::PriceDecrementTick => {
                if self.price_type == PriceType::Limit {
                    self.decrement_price();
                }
            }
            Message::CashMarginChanged(cm) => self.cash_margin = cm,
            Message::ExpireDayChanged(e) => self.expire_day = e,
            Message::SecondPasswordChanged(s) => self.second_password = s,
            Message::HoldingsUpdated(h) => self.holdings = h,
            Message::ConfirmClicked => {
                self.confirm_modal = true;
            }
            Message::ConfirmCancelled => {
                self.confirm_modal = false;
            }
            Message::Submitted => match self.build_request() {
                Ok(req) => {
                    self.confirm_modal = false;
                    self.loading = true;
                    return Some(Action::Submit(Box::new(req)));
                }
                Err(e) => {
                    self.last_result = Some(Err(e));
                }
            },
            Message::OrderCompleted(result) => {
                self.loading = false;
                // 成功時のみパスワードをクリア。エラー時は保持し、再発注時に
                // 再入力不要にする（UX 優先）。パスワードは Debug で REDACTED 表示済み。
                if result.is_ok() {
                    self.second_password.clear();
                }
                self.last_result = Some(result);
            }
            Message::OpenTickerSearch => {
                self.modal = Some(crate::modal::pane::mini_tickers_list::MiniPanel::new());
            }
            Message::MiniTickers(msg) => {
                if let Some(modal) = &mut self.modal {
                    match modal.update(msg) {
                        Some(crate::modal::pane::mini_tickers_list::Action::RowSelected(
                            crate::modal::pane::mini_tickers_list::RowSelection::Switch(
                                ticker_info,
                            ),
                        )) => {
                            let issue_code = ticker_info.ticker.symbol_and_exchange_string();
                            let tick_size = f64::from(f32::from(ticker_info.min_ticksize));
                            let issue_changed = self.issue_code != issue_code;
                            self.issue_code = issue_code.clone();
                            self.tick_size = Some(tick_size);
                            self.modal = None;
                            if issue_changed {
                                self.holdings = None;
                                if self.side == Side::Sell && !issue_code.is_empty() {
                                    return Some(Action::FetchHoldings { issue_code });
                                }
                            }
                        }
                        Some(crate::modal::pane::mini_tickers_list::Action::RowSelected(_)) => {
                            self.modal = None;
                        }
                        None => {}
                    }
                }
            }
        }
        None
    }

    // ── View ──────────────────────────────────────────────────────────────────

    pub fn view(&self, theme: &Theme, is_replay: bool) -> Element<'_, Message> {
        let virtual_mode = self.is_virtual || is_replay;

        let issue_label = if self.issue_code.is_empty() {
            "銘柄未選択".to_string()
        } else {
            self.issue_code.clone()
        };
        let is_modal_active = self.modal.is_some();
        let ticker_btn = button(text(issue_label).size(13))
            .on_press(Message::OpenTickerSearch)
            .style(move |theme, status| {
                crate::style::button::modifier(theme, status, !is_modal_active)
            });

        let mut col = column![
            self.view_side_tabs(),
            ticker_btn,
            self.view_account_row(),
            self.view_qty_row(),
            self.view_price_row(),
            self.view_expire_row(),
        ]
        .spacing(8);

        if !virtual_mode {
            let password_input = text_input("発注パスワード", &self.second_password)
                .on_input(Message::SecondPasswordChanged)
                .secure(true)
                .size(13);
            col = col.push(
                row![text("パスワード: ").size(13), password_input]
                    .align_y(Alignment::Center)
                    .spacing(4),
            );
        }

        col = col
            .push(self.view_result_row())
            .push(self.view_action_area(theme, virtual_mode));

        let body = container(col.padding(8));

        if virtual_mode {
            let banner_text = if self.is_virtual {
                "⏪ 仮想注文モード"
            } else {
                "⏪ REPLAYモード中 — 注文は無効です"
            };
            column![
                container(text(banner_text).size(12))
                    .style(|theme: &Theme| {
                        let palette = theme.extended_palette();
                        iced::widget::container::Style {
                            background: Some(palette.primary.weak.color.into()),
                            text_color: Some(palette.primary.weak.text),
                            ..Default::default()
                        }
                    })
                    .padding([4, 8])
                    .width(iced::Length::Fill),
                body,
            ]
            .into()
        } else {
            body.into()
        }
    }

    fn view_side_tabs(&self) -> Element<'_, Message> {
        let buy_btn = button(text("買い").size(13))
            .on_press(Message::SideChanged(Side::Buy))
            .style(if self.side == Side::Buy {
                iced::widget::button::primary
            } else {
                iced::widget::button::secondary
            });
        let sell_btn = button(text("売り").size(13))
            .on_press(Message::SideChanged(Side::Sell))
            .style(if self.side == Side::Sell {
                iced::widget::button::primary
            } else {
                iced::widget::button::secondary
            });
        row![buy_btn, sell_btn].spacing(4).into()
    }

    fn view_account_row(&self) -> Element<'_, Message> {
        let account_picker = pick_list(
            &AccountType::ALL[..],
            Some(self.account_type),
            Message::AccountTypeChanged,
        )
        .text_size(13);

        let cash_margin_picker = pick_list(
            &CashMarginType::ALL[..],
            Some(self.cash_margin),
            Message::CashMarginChanged,
        )
        .text_size(13);

        column![
            row![text("口座: ").size(13), account_picker]
                .align_y(Alignment::Center)
                .spacing(4),
            row![text("現物/信用: ").size(13), cash_margin_picker]
                .align_y(Alignment::Center)
                .spacing(4)
        ]
        .spacing(8)
        .into()
    }

    fn view_qty_row(&self) -> Element<'_, Message> {
        let qty_input = text_input("株数", &self.qty)
            .on_input(Message::QtyChanged)
            .size(13)
            .width(iced::Length::Fixed(100.0));

        if self.side == Side::Sell {
            let all_btn = button(text("全数量").size(12))
                .on_press_maybe(self.holdings.map(|_| Message::FillFromHoldings));
            let holding_info = self
                .holdings
                .map(|h| text(format!("(保有: {h}株)")).size(11))
                .unwrap_or_else(|| text("(保有: --株)").size(11));
            row![text("数量: ").size(13), qty_input, all_btn, holding_info]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        } else {
            row![text("数量: ").size(13), qty_input]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        }
    }

    fn view_price_row(&self) -> Element<'_, Message> {
        let price_type_picker = pick_list(
            [PriceType::Market, PriceType::Limit],
            Some(self.price_type),
            Message::PriceTypeChanged,
        )
        .text_size(13);

        if self.price_type == PriceType::Limit {
            let dec_btn = button(text("▼").size(12)).on_press(Message::PriceDecrementTick);
            let price_input = text_input("指値", &self.limit_price)
                .on_input(Message::LimitPriceChanged)
                .size(13)
                .width(iced::Length::Fixed(80.0));
            let inc_btn = button(text("▲").size(12)).on_press(Message::PriceIncrementTick);
            row![
                text("価格: ").size(13),
                price_type_picker,
                dec_btn,
                price_input,
                inc_btn
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
        } else {
            row![text("価格: ").size(13), price_type_picker]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        }
    }

    fn view_expire_row(&self) -> Element<'_, Message> {
        let expire_options = [ExpireDay::Today];
        let expire_picker = pick_list(
            expire_options,
            Some(self.expire_day.clone()),
            Message::ExpireDayChanged,
        )
        .text_size(13);
        row![text("期日: ").size(13), expire_picker]
            .align_y(Alignment::Center)
            .spacing(4)
            .into()
    }

    fn view_result_row(&self) -> Element<'_, Message> {
        match &self.last_result {
            Some(Ok(ok)) => {
                let msg = if let Some(warn) = &ok.warning {
                    format!("受付: {} (警告: {})", ok.order_num, warn)
                } else {
                    format!("注文受付: {}", ok.order_num)
                };
                text(msg).size(12).color([0.2, 0.8, 0.2]).into()
            }
            Some(Err(e)) => text(e.as_str()).size(12).color([0.9, 0.2, 0.2]).into(),
            None => text("").size(12).into(),
        }
    }

    fn view_action_area(&self, theme: &Theme, virtual_mode: bool) -> Element<'_, Message> {
        if self.confirm_modal {
            let side_str = self.side.to_string();
            let price_str = match self.price_type {
                PriceType::Market => "成行".to_string(),
                PriceType::Limit => format!("{}円", self.limit_price),
            };
            let info = row![
                text(format!("{} ", self.issue_code)).size(12),
                text(side_str.clone())
                    .size(12)
                    .color(crate::style::side_color(&side_str, theme)),
                text(format!(" {}株 {}", self.qty, price_str)).size(12),
            ];
            let cancel_btn =
                button(text("キャンセル").size(13)).on_press(Message::ConfirmCancelled);
            let submit_btn = button(text("注文を発注する").size(13)).on_press(Message::Submitted);
            column![
                text("注文確認").size(14),
                info,
                row![cancel_btn, submit_btn].spacing(8),
            ]
            .spacing(8)
            .into()
        } else {
            let password_ok = virtual_mode || !self.second_password.is_empty();
            let confirm_enabled = !self.loading
                && !self.issue_code.is_empty()
                && !self.qty.is_empty()
                && password_ok
                && (self.price_type == PriceType::Market || !self.limit_price.is_empty());

            let label = if self.loading {
                "送信中..."
            } else if virtual_mode {
                "仮想注文確認"
            } else {
                "注文確認"
            };

            button(text(label).size(13))
                .on_press_maybe(confirm_enabled.then_some(Message::ConfirmClicked))
                .into()
        }
    }
}

// ── ヘルパー ──────────────────────────────────────────────────────────────────

/// 価格を呼値単位に合わせた桁数でフォーマットする。
fn format_price(price: f64, tick: f64) -> String {
    // 呼値の小数桁数を求める
    let decimals = if tick < 1.0 {
        let s = format!("{tick}");
        s.find('.').map(|dot| s.len() - dot - 1).unwrap_or(0)
    } else {
        0
    };
    format!("{:.prec$}", price, prec = decimals)
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_panel() -> OrderEntryPanel {
        let mut p = OrderEntryPanel::new();
        p.issue_code = "7203".to_string();
        p.qty = "100".to_string();
        p.second_password = "secret".to_string();
        p
    }

    // ── Cycle 1: build_request ─────────────────────────────────────────────

    #[test]
    fn build_request_market_buy_returns_correct_fields() {
        let panel = make_panel();
        let req = panel.build_request().unwrap();
        assert_eq!(req.issue_code, "7203");
        assert_eq!(req.side, "3"); // 買い
        assert_eq!(req.price, "0"); // 成行
        assert_eq!(req.qty, "100");
        assert_eq!(req.account_type, "1"); // 特定
        assert_eq!(req.cash_margin, "0"); // 現物
        assert_eq!(req.expire_day, "0"); // 当日
    }

    #[test]
    fn build_request_limit_sell_returns_correct_price() {
        let mut panel = make_panel();
        panel.side = Side::Sell;
        panel.price_type = PriceType::Limit;
        panel.limit_price = "2500".to_string();
        let req = panel.build_request().unwrap();
        assert_eq!(req.side, "1"); // 売り
        assert_eq!(req.price, "2500");
    }

    #[test]
    fn build_request_fails_when_issue_code_empty() {
        let mut panel = make_panel();
        panel.issue_code.clear();
        assert!(panel.build_request().is_err());
    }

    #[test]
    fn build_request_fails_when_qty_empty() {
        let mut panel = make_panel();
        panel.qty.clear();
        assert!(panel.build_request().is_err());
    }

    #[test]
    fn build_request_fails_when_password_empty() {
        let mut panel = make_panel();
        panel.second_password.clear();
        assert!(panel.build_request().is_err());
    }

    #[test]
    fn build_request_fails_when_limit_price_empty_for_limit_order() {
        let mut panel = make_panel();
        panel.price_type = PriceType::Limit;
        panel.limit_price.clear();
        assert!(panel.build_request().is_err());
    }

    // ── Cycle 2: price step ────────────────────────────────────────────────

    #[test]
    fn increment_price_adds_tick_size() {
        let mut panel = make_panel();
        panel.price_type = PriceType::Limit;
        panel.tick_size = Some(1.0);
        panel.limit_price = "2500".to_string();
        panel.increment_price();
        assert_eq!(panel.limit_price, "2501");
    }

    #[test]
    fn decrement_price_subtracts_tick_size() {
        let mut panel = make_panel();
        panel.price_type = PriceType::Limit;
        panel.tick_size = Some(1.0);
        panel.limit_price = "2500".to_string();
        panel.decrement_price();
        assert_eq!(panel.limit_price, "2499");
    }

    #[test]
    fn increment_price_does_nothing_when_tick_size_none() {
        let mut panel = make_panel();
        panel.price_type = PriceType::Limit;
        panel.tick_size = None;
        panel.limit_price = "2500".to_string();
        panel.increment_price();
        assert_eq!(panel.limit_price, "2500"); // 変わらない
    }

    #[test]
    fn decrement_price_does_not_go_below_zero() {
        let mut panel = make_panel();
        panel.tick_size = Some(1.0);
        panel.limit_price = "0".to_string();
        panel.decrement_price();
        let price: f64 = panel.limit_price.parse().unwrap();
        assert!(price >= 0.0);
    }

    // ── Cycle 3: Message::FillFromHoldings ────────────────────────────────

    #[test]
    fn fill_from_holdings_sets_qty_to_holdings() {
        let mut panel = make_panel();
        panel.holdings = Some(200);
        panel.qty = "0".to_string();
        panel.update(Message::FillFromHoldings);
        assert_eq!(panel.qty, "200");
    }

    #[test]
    fn fill_from_holdings_does_nothing_when_holdings_none() {
        let mut panel = make_panel();
        panel.holdings = None;
        panel.qty = "100".to_string();
        panel.update(Message::FillFromHoldings);
        assert_eq!(panel.qty, "100"); // 変わらない
    }

    // ── Cycle 4: confirm flow ─────────────────────────────────────────────

    #[test]
    fn confirm_clicked_opens_modal() {
        let mut panel = make_panel();
        panel.update(Message::ConfirmClicked);
        assert!(panel.confirm_modal);
    }

    #[test]
    fn confirm_cancelled_closes_modal() {
        let mut panel = make_panel();
        panel.confirm_modal = true;
        panel.update(Message::ConfirmCancelled);
        assert!(!panel.confirm_modal);
    }

    #[test]
    fn submitted_returns_submit_action() {
        let mut panel = make_panel();
        panel.confirm_modal = true;
        let action = panel.update(Message::Submitted);
        assert!(matches!(action, Some(Action::Submit(_))));
        assert!(!panel.confirm_modal);
        assert!(panel.loading);
    }

    // ── Cycle 5: SideChanged → FetchHoldings ──────────────────────────────

    #[test]
    fn switching_to_sell_emits_fetch_holdings() {
        let mut panel = make_panel();
        let action = panel.update(Message::SideChanged(Side::Sell));
        assert!(matches!(action, Some(Action::FetchHoldings { .. })));
    }

    #[test]
    fn switching_to_buy_does_not_emit_fetch_holdings() {
        let mut panel = make_panel();
        panel.side = Side::Sell;
        let action = panel.update(Message::SideChanged(Side::Buy));
        assert!(action.is_none());
    }

    // ── Cycle 6: OrderCompleted clears password ────────────────────────────

    #[test]
    fn order_completed_ok_clears_second_password() {
        let mut panel = make_panel();
        panel.second_password = "mypassword".to_string();
        panel.update(Message::OrderCompleted(Ok(OrderSuccess {
            order_num: "12345678".to_string(),
            warning: None,
        })));
        assert!(panel.second_password.is_empty());
    }

    #[test]
    fn order_completed_err_keeps_password() {
        let mut panel = make_panel();
        panel.second_password = "mypassword".to_string();
        panel.update(Message::OrderCompleted(Err("エラー".to_string())));
        assert!(!panel.second_password.is_empty());
    }

    // ── Cycle 7c: CashMarginChanged — 現物⇔信用切り替え（5-2） ─────────────
    // 5-2: 現物⇔信用切り替え時の cash_margin フィールド更新

    #[test]
    fn cash_margin_changed_updates_field() {
        let mut panel = make_panel();
        assert_eq!(panel.cash_margin, CashMarginType::Cash);
        panel.update(Message::CashMarginChanged(CashMarginType::MarginNew6M));
        assert_eq!(panel.cash_margin, CashMarginType::MarginNew6M);
        panel.update(Message::CashMarginChanged(CashMarginType::MarginNewGeneral));
        assert_eq!(panel.cash_margin, CashMarginType::MarginNewGeneral);
    }

    #[test]
    fn cash_margin_changed_to_close_updates_field() {
        let mut panel = make_panel();
        panel.update(Message::CashMarginChanged(CashMarginType::MarginClose6M));
        assert_eq!(panel.cash_margin, CashMarginType::MarginClose6M);
        panel.update(Message::CashMarginChanged(
            CashMarginType::MarginCloseGeneral,
        ));
        assert_eq!(panel.cash_margin, CashMarginType::MarginCloseGeneral);
    }

    // ── Cycle 7d: OrderCompleted Err — エラー表示（5-4） ────────────────────
    // 5-4: 注文エラー時に last_result が Err になること（エラーメッセージ格納確認）

    #[test]
    fn order_completed_err_stores_error_in_last_result() {
        let mut panel = make_panel();
        panel.update(Message::OrderCompleted(Err(
            "code=11304, 第二暗証番号が誤っています".to_string(),
        )));
        assert!(
            matches!(&panel.last_result, Some(Err(e)) if e.contains("11304")),
            "エラー結果が last_result に格納されるべき: {:?}",
            panel.last_result
        );
    }

    #[test]
    fn order_completed_err_does_not_clear_loading() {
        let mut panel = make_panel();
        panel.loading = true;
        panel.update(Message::OrderCompleted(Err("エラー".to_string())));
        assert!(
            !panel.loading,
            "OrderCompleted 後は loading がクリアされるべき"
        );
    }

    // ── Cycle 7: AccountType / CashMarginType api_code ─────────────────────

    #[test]
    fn account_type_api_codes_are_correct() {
        assert_eq!(AccountType::Tokutei.api_code(), "1");
        assert_eq!(AccountType::Ippan.api_code(), "3");
        assert_eq!(AccountType::Nisa.api_code(), "5");
        assert_eq!(AccountType::NisaGrowth.api_code(), "6");
    }

    #[test]
    fn cash_margin_type_api_codes_are_correct() {
        assert_eq!(CashMarginType::Cash.api_code(), "0");
        assert_eq!(CashMarginType::MarginNew6M.api_code(), "2");
        assert_eq!(CashMarginType::MarginClose6M.api_code(), "4");
        assert_eq!(CashMarginType::MarginNewGeneral.api_code(), "6");
        assert_eq!(CashMarginType::MarginCloseGeneral.api_code(), "8");
    }

    // ── Cycle 8: view refactoring ───────────────────────────────────────────

    #[test]
    fn view_side_tabs_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_side_tabs();
    }

    #[test]
    fn view_account_row_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_account_row();
    }

    #[test]
    fn view_qty_row_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_qty_row();
    }

    #[test]
    fn view_price_row_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_price_row();
    }

    #[test]
    fn view_expire_row_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_expire_row();
    }

    #[test]
    fn view_result_row_returns_element() {
        let panel = make_panel();
        let _elem = panel.view_result_row();
    }

    #[test]
    fn view_action_area_returns_element() {
        let panel = make_panel();
        let theme = iced::Theme::Dark;
        let _elem = panel.view_action_area(&theme, false);
    }

    // ── Cycle 9: 銘柄手動選択（OpenTickerSearch / MiniTickers） ──────────────

    fn make_ticker_info(symbol: &str) -> exchange::TickerInfo {
        let ticker = exchange::Ticker::new(symbol, exchange::adapter::Exchange::BinanceSpot);
        exchange::TickerInfo::new(ticker, 1.0, 1.0, None)
    }

    #[test]
    fn open_ticker_search_sets_modal() {
        let mut panel = make_panel();
        assert!(panel.modal.is_none());
        panel.update(Message::OpenTickerSearch);
        assert!(
            panel.modal.is_some(),
            "OpenTickerSearch でモーダルが開くべき"
        );
    }

    #[test]
    fn mini_tickers_switch_updates_issue_code() {
        let mut panel = make_panel();
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti),
            ),
        ));
        assert!(
            !panel.issue_code.is_empty(),
            "銘柄選択後に issue_code が設定されるべき"
        );
        assert!(
            panel.tick_size.is_some(),
            "銘柄選択後に tick_size が設定されるべき"
        );
    }

    #[test]
    fn mini_tickers_switch_closes_modal() {
        let mut panel = make_panel();
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti),
            ),
        ));
        assert!(panel.modal.is_none(), "銘柄選択後にモーダルが閉じるべき");
    }

    #[test]
    fn mini_tickers_switch_resets_holdings_on_new_ticker() {
        let mut panel = make_panel(); // issue_code = "7203"
        panel.holdings = Some(500);
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti),
            ),
        ));
        assert!(
            panel.holdings.is_none(),
            "銘柄変更で holdings がリセットされるべき"
        );
    }

    #[test]
    fn mini_tickers_switch_sell_mode_emits_fetch_holdings() {
        let mut panel = make_panel();
        panel.side = Side::Sell;
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        let action = panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti),
            ),
        ));
        assert!(
            matches!(action, Some(Action::FetchHoldings { .. })),
            "売りモード＋銘柄変更で FetchHoldings が発行されるべき: {action:?}"
        );
    }

    #[test]
    fn mini_tickers_switch_buy_mode_no_fetch_holdings() {
        let mut panel = make_panel();
        panel.side = Side::Buy;
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        let action = panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti),
            ),
        ));
        assert!(
            action.is_none(),
            "買いモードでは FetchHoldings を発行しない"
        );
    }

    #[test]
    fn mini_tickers_add_row_selection_is_ignored() {
        let mut panel = make_panel();
        panel.update(Message::OpenTickerSearch);
        let ti = make_ticker_info("BTCUSDT");
        let action = panel.update(Message::MiniTickers(
            crate::modal::pane::mini_tickers_list::Message::RowSelected(
                crate::modal::pane::mini_tickers_list::RowSelection::Add(ti),
            ),
        ));
        assert!(action.is_none(), "Add 選択は無視されるべき");
        assert!(
            panel.issue_code == "7203",
            "Add 選択で issue_code は変わらないべき"
        );
    }
}
