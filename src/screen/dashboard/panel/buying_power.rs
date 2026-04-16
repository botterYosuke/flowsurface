use iced::{
    Alignment, Element, Theme,
    widget::{button, column, container, row, text},
};

// ── パネル本体 ─────────────────────────────────────────────────────────────────

/// 余力情報パネル。
/// iced widget で描画する（Panel trait / canvas::Program は実装しない）。
pub struct BuyingPowerPanel {
    /// 現物買付可能額（空文字 = 未取得）
    cash_buying_power: String,
    /// NISA成長投資可能額
    nisa_growth_buying_power: String,
    /// 不足金発生フラグ（"1" = 発生）
    shortage_flag: String,
    /// 信用新規建可能額
    margin_new_order_power: String,
    /// 委託保証金率(%)
    maintenance_margin_rate: String,
    /// 追証フラグ（"1" = 確定）
    margin_call_flag: String,
    /// 読み込み中フラグ
    loading: bool,
    /// 直前のエラーメッセージ
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// 手動リフレッシュボタン押下
    RefreshClicked,
    /// 現物余力の取得結果
    BuyingPowerUpdated {
        cash_buying_power: String,
        nisa_growth_buying_power: String,
        shortage_flag: String,
    },
    /// 信用余力の取得結果
    MarginPowerUpdated {
        margin_new_order_power: String,
        maintenance_margin_rate: String,
        margin_call_flag: String,
    },
    /// API エラー
    FetchFailed(String),
}

/// このパネルが発行するアクション（pane.rs が Effect に変換する）
pub enum Action {
    FetchBuyingPower,
}

impl BuyingPowerPanel {
    pub fn new() -> Self {
        Self {
            cash_buying_power: String::new(),
            nisa_growth_buying_power: String::new(),
            shortage_flag: String::new(),
            margin_new_order_power: String::new(),
            maintenance_margin_rate: String::new(),
            margin_call_flag: String::new(),
            loading: false,
            last_error: None,
        }
    }

    pub fn update(&mut self, msg: Message) -> Option<Action> {
        match msg {
            Message::RefreshClicked => {
                self.loading = true;
                self.last_error = None;
                return Some(Action::FetchBuyingPower);
            }
            Message::BuyingPowerUpdated {
                cash_buying_power,
                nisa_growth_buying_power,
                shortage_flag,
            } => {
                self.cash_buying_power = cash_buying_power;
                self.nisa_growth_buying_power = nisa_growth_buying_power;
                self.shortage_flag = shortage_flag;
                self.loading = false;
            }
            Message::MarginPowerUpdated {
                margin_new_order_power,
                maintenance_margin_rate,
                margin_call_flag,
            } => {
                self.margin_new_order_power = margin_new_order_power;
                self.maintenance_margin_rate = maintenance_margin_rate;
                self.margin_call_flag = margin_call_flag;
                self.loading = false;
            }
            Message::FetchFailed(e) => {
                self.last_error = Some(e);
                self.loading = false;
            }
        }
        None
    }

    pub fn view(&self, theme: &Theme) -> Element<'_, Message> {
        let refresh_btn = button(text("更新").size(13))
            .on_press_maybe((!self.loading).then_some(Message::RefreshClicked));

        let loading_label = if self.loading {
            text("読み込み中...").size(12)
        } else {
            text("").size(12)
        };

        let error_row: Element<'_, Message> = if let Some(e) = &self.last_error {
            text(e.as_str()).size(12).color([0.9, 0.2, 0.2]).into()
        } else {
            text("").size(12).into()
        };

        // 現物口座
        let cash_section = column![
            text("現物口座").size(13),
            labeled_value("現物株買付可能額:", &self.cash_buying_power, "円"),
            labeled_value("NISA成長投資可能額:", &self.nisa_growth_buying_power, "円"),
        ]
        .spacing(4);

        // 信用口座
        let margin_call_label: Element<'_, Message> = if self.margin_call_flag == "1" {
            text("⚠ 追証確定")
                .size(12)
                .color(crate::style::margin_call_color(theme))
                .into()
        } else {
            text("追証: なし").size(12).into()
        };

        let margin_section = column![
            text("信用口座").size(13),
            labeled_value("信用新規建可能額:", &self.margin_new_order_power, "円"),
            labeled_value("委託保証金率:", &self.maintenance_margin_rate, "%"),
            margin_call_label,
        ]
        .spacing(4);

        container(
            column![
                row![refresh_btn, loading_label]
                    .spacing(8)
                    .align_y(Alignment::Center),
                error_row,
                cash_section,
                margin_section,
            ]
            .spacing(10)
            .padding(8),
        )
        .into()
    }
}

/// `ラベル: 値 単位` の行を生成するヘルパー。
fn labeled_value<'a>(label: &'a str, value: &str, unit: &str) -> Element<'a, Message> {
    let display = if value.is_empty() {
        "---".to_string()
    } else {
        format!("{value}{unit}")
    };
    row![
        text(label).size(12).width(iced::Length::Fixed(160.0)),
        text(display).size(12),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_clicked_sets_loading_and_emits_action() {
        let mut panel = BuyingPowerPanel::new();
        let action = panel.update(Message::RefreshClicked);
        assert!(panel.loading);
        assert!(matches!(action, Some(Action::FetchBuyingPower)));
    }

    #[test]
    fn buying_power_updated_clears_loading() {
        let mut panel = BuyingPowerPanel::new();
        panel.loading = true;
        panel.update(Message::BuyingPowerUpdated {
            cash_buying_power: "1000000".to_string(),
            nisa_growth_buying_power: "500000".to_string(),
            shortage_flag: "0".to_string(),
        });
        assert!(!panel.loading);
        assert_eq!(panel.cash_buying_power, "1000000");
    }

    #[test]
    fn margin_power_updated_clears_loading() {
        let mut panel = BuyingPowerPanel::new();
        panel.loading = true;
        panel.update(Message::MarginPowerUpdated {
            margin_new_order_power: "2000000".to_string(),
            maintenance_margin_rate: "30".to_string(),
            margin_call_flag: "0".to_string(),
        });
        assert!(!panel.loading);
        assert_eq!(panel.maintenance_margin_rate, "30");
    }

    #[test]
    fn fetch_failed_stores_error_and_clears_loading() {
        let mut panel = BuyingPowerPanel::new();
        panel.loading = true;
        panel.update(Message::FetchFailed("接続エラー".to_string()));
        assert!(!panel.loading);
        assert_eq!(panel.last_error.as_deref(), Some("接続エラー"));
    }

    #[test]
    fn margin_call_flag_one_means_margin_call() {
        let mut panel = BuyingPowerPanel::new();
        panel.update(Message::MarginPowerUpdated {
            margin_new_order_power: "0".to_string(),
            maintenance_margin_rate: "15".to_string(),
            margin_call_flag: "1".to_string(),
        });
        assert_eq!(panel.margin_call_flag, "1");
    }

    #[test]
    fn refresh_clears_previous_error() {
        let mut panel = BuyingPowerPanel::new();
        panel.last_error = Some("前のエラー".to_string());
        panel.update(Message::RefreshClicked);
        assert!(panel.last_error.is_none());
    }
}
