use std::collections::HashMap;

use exchange::adapter::tachibana::{
    CancelOrderRequest, CorrectOrderRequest, ExecutionRecord, OrderRecord,
};
use iced::{
    Alignment, Element, Theme,
    widget::{button, column, container, row, scrollable, text, text_input},
};

// ── ドメイン型 ────────────────────────────────────────────────────────────────

/// 訂正モーダルの状態
#[derive(Debug, Clone)]
struct CorrectModal {
    order_num: String,
    eig_day: String,
    /// 現在の注文内容（表示用）
    current_price: String,
    current_qty: String,
    issue_code: String,
    /// 入力フォーム（空欄 → API に "*" を送る = 変更なし）
    new_price: String,
    new_qty: String,
    second_password: String,
}

impl CorrectModal {
    fn new(order: &OrderRecord) -> Self {
        Self {
            order_num: order.order_num.clone(),
            eig_day: order.eig_day.clone(),
            current_price: order.order_price.clone(),
            current_qty: order.order_qty.clone(),
            issue_code: order.issue_code.clone(),
            new_price: String::new(),
            new_qty: String::new(),
            second_password: String::new(),
        }
    }
}

/// 取消モーダルの状態
#[derive(Debug, Clone)]
struct CancelModal {
    order_num: String,
    eig_day: String,
    issue_code: String,
    order_qty: String,
    second_password: String,
}

impl CancelModal {
    fn new(order: &OrderRecord) -> Self {
        Self {
            order_num: order.order_num.clone(),
            eig_day: order.eig_day.clone(),
            issue_code: order.issue_code.clone(),
            order_qty: order.order_qty.clone(),
            second_password: String::new(),
        }
    }
}

// ── パネル本体 ─────────────────────────────────────────────────────────────────

/// 注文照会パネル。
/// iced widget で描画する（Panel trait / canvas::Program は実装しない）。
pub struct OrderListPanel {
    /// 現在の注文一覧
    orders: Vec<OrderRecord>,
    /// 約定通知の diff 比較用（前回取得時のスナップショット）
    prev_orders: Vec<OrderRecord>,
    /// クリックで展開中の注文番号
    expanded_order: Option<String>,
    /// 約定明細キャッシュ（注文番号 → 約定リスト）
    executions: HashMap<String, Vec<ExecutionRecord>>,
    /// 訂正モーダル（None = 非表示）
    correct_modal: Option<CorrectModal>,
    /// 取消モーダル（None = 非表示）
    cancel_modal: Option<CancelModal>,
    /// 読み込み中フラグ
    loading: bool,
    /// 直前のエラーメッセージ
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    // ── 注文一覧操作 ──────────────────────────────────────────────────────────
    /// 手動リフレッシュボタン
    RefreshClicked,
    /// 注文行クリック（約定明細の展開/折りたたみ）
    RowClicked(String),
    // ── 訂正モーダル ─────────────────────────────────────────────────────────
    /// [訂正] ボタンクリック → モーダルを開く
    CorrectClicked(String), // order_num
    CorrectNewPriceChanged(String),
    CorrectNewQtyChanged(String),
    CorrectPasswordChanged(String),
    CorrectSubmitted,
    CorrectCancelled,
    // ── 取消モーダル ─────────────────────────────────────────────────────────
    /// [取消] ボタンクリック → モーダルを開く
    CancelClicked(String), // order_num
    CancelPasswordChanged(String),
    CancelSubmitted,
    CancelCancelled,
    // ── API 応答 ──────────────────────────────────────────────────────────────
    /// 注文一覧の取得結果
    OrdersUpdated(Vec<OrderRecord>),
    /// 約定明細の取得結果
    ExecutionsUpdated {
        order_num: String,
        executions: Vec<ExecutionRecord>,
    },
    /// 訂正・取消の結果（成功時は order_num, エラー時はメッセージ）
    ModifyCompleted(Result<String, String>),
    /// ポーリング（dashboard.rs の subscription からのタイマー）
    PollTick,
}

/// このパネルが発行するアクション（pane.rs が Effect に変換する）
pub enum Action {
    FetchOrders,
    FetchOrderDetail { order_num: String, eig_day: String },
    SubmitCorrect(Box<CorrectOrderRequest>),
    SubmitCancel(Box<CancelOrderRequest>),
}

impl OrderListPanel {
    pub fn new() -> Self {
        Self {
            orders: Vec::new(),
            prev_orders: Vec::new(),
            expanded_order: None,
            executions: HashMap::new(),
            correct_modal: None,
            cancel_modal: None,
            loading: false,
            last_error: None,
        }
    }

    /// 注文が "全部約定" に遷移した注文番号リストを返す（トースト通知用）。
    pub fn newly_executed(&self) -> Vec<&str> {
        self.orders
            .iter()
            .filter(|o| {
                o.status_text == "全部約定"
                    && self
                        .prev_orders
                        .iter()
                        .find(|p| p.order_num == o.order_num)
                        .map(|p| p.status_text != "全部約定")
                        .unwrap_or(false)
            })
            .map(|o| o.order_num.as_str())
            .collect()
    }

    pub fn update(&mut self, msg: Message) -> Option<Action> {
        match msg {
            Message::RefreshClicked => {
                self.loading = true;
                return Some(Action::FetchOrders);
            }
            Message::PollTick => {
                return Some(Action::FetchOrders);
            }
            Message::RowClicked(order_num) => {
                if self.expanded_order.as_deref() == Some(&order_num) {
                    self.expanded_order = None;
                } else {
                    let eig_day = self
                        .orders
                        .iter()
                        .find(|o| o.order_num == order_num)
                        .map(|o| o.eig_day.clone())
                        .unwrap_or_default();
                    self.expanded_order = Some(order_num.clone());
                    if !self.executions.contains_key(&order_num) {
                        return Some(Action::FetchOrderDetail { order_num, eig_day });
                    }
                }
            }
            // ── 訂正モーダル ─────────────────────────────────────────────────
            Message::CorrectClicked(order_num) => {
                if let Some(order) = self.orders.iter().find(|o| o.order_num == order_num) {
                    self.correct_modal = Some(CorrectModal::new(order));
                }
            }
            Message::CorrectNewPriceChanged(v) => {
                if let Some(m) = &mut self.correct_modal {
                    m.new_price = v;
                }
            }
            Message::CorrectNewQtyChanged(v) => {
                if let Some(m) = &mut self.correct_modal {
                    m.new_qty = v;
                }
            }
            Message::CorrectPasswordChanged(v) => {
                if let Some(m) = &mut self.correct_modal {
                    m.second_password = v;
                }
            }
            Message::CorrectSubmitted => {
                if let Some(m) = &self.correct_modal {
                    if m.second_password.is_empty() {
                        return None;
                    }
                    let req = CorrectOrderRequest {
                        order_number: m.order_num.clone(),
                        eig_day: m.eig_day.clone(),
                        condition: "*".to_string(),
                        // 空欄は変更なし("*")、入力があればその値
                        price: if m.new_price.is_empty() {
                            "*".to_string()
                        } else {
                            m.new_price.clone()
                        },
                        qty: if m.new_qty.is_empty() {
                            "*".to_string()
                        } else {
                            m.new_qty.clone()
                        },
                        expire_day: "*".to_string(),
                        second_password: m.second_password.clone(),
                    };
                    self.correct_modal = None;
                    return Some(Action::SubmitCorrect(Box::new(req)));
                }
            }
            Message::CorrectCancelled => {
                self.correct_modal = None;
            }
            // ── 取消モーダル ─────────────────────────────────────────────────
            Message::CancelClicked(order_num) => {
                if let Some(order) = self.orders.iter().find(|o| o.order_num == order_num) {
                    self.cancel_modal = Some(CancelModal::new(order));
                }
            }
            Message::CancelPasswordChanged(v) => {
                if let Some(m) = &mut self.cancel_modal {
                    m.second_password = v;
                }
            }
            Message::CancelSubmitted => {
                if let Some(m) = &self.cancel_modal {
                    if m.second_password.is_empty() {
                        return None;
                    }
                    let req = CancelOrderRequest {
                        order_number: m.order_num.clone(),
                        eig_day: m.eig_day.clone(),
                        second_password: m.second_password.clone(),
                    };
                    self.cancel_modal = None;
                    return Some(Action::SubmitCancel(Box::new(req)));
                }
            }
            Message::CancelCancelled => {
                self.cancel_modal = None;
            }
            // ── API 応答 ──────────────────────────────────────────────────────
            Message::OrdersUpdated(orders) => {
                self.loading = false;
                self.prev_orders = std::mem::replace(&mut self.orders, orders);
                self.last_error = None;
            }
            Message::ExecutionsUpdated {
                order_num,
                executions,
            } => {
                self.executions.insert(order_num, executions);
            }
            Message::ModifyCompleted(result) => match result {
                Ok(_) => {
                    self.last_error = None;
                    return Some(Action::FetchOrders);
                }
                Err(e) => {
                    self.last_error = Some(e);
                }
            },
        }
        None
    }

    pub fn view(&self, theme: &Theme) -> Element<'_, Message> {
        // 訂正モーダルが開いている場合
        if let Some(modal) = &self.correct_modal {
            return self.view_correct_modal(modal);
        }
        // 取消モーダルが開いている場合
        if let Some(modal) = &self.cancel_modal {
            return self.view_cancel_modal(modal);
        }
        self.view_order_list(theme)
    }

    fn view_order_list(&self, theme: &Theme) -> Element<'_, Message> {
        let header = row![
            button(text("更新").size(13))
                .on_press_maybe((!self.loading).then_some(Message::RefreshClicked)),
            if self.loading {
                text("読み込み中...").size(12)
            } else {
                text("").size(12)
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let error_row: Element<'_, Message> = if let Some(e) = &self.last_error {
            text(e.as_str()).size(12).color([0.9, 0.2, 0.2]).into()
        } else {
            text("").size(12).into()
        };

        let orders_col = if self.orders.is_empty() {
            column![text("注文なし").size(13)].spacing(4)
        } else {
            let mut col = column![].spacing(2);
            for order in &self.orders {
                col = col.push(self.view_order_row(order, theme));
                // 展開中の行には約定明細を表示
                if self.expanded_order.as_deref() == Some(&order.order_num) {
                    col = col.push(self.view_execution_detail(&order.order_num));
                }
            }
            col
        };

        container(
            column![
                header,
                error_row,
                scrollable(orders_col.padding(4)).height(iced::Length::Fill),
            ]
            .spacing(6)
            .padding(8),
        )
        .into()
    }

    fn view_order_row<'a>(&'a self, order: &'a OrderRecord, theme: &Theme) -> Element<'a, Message> {
        let expanded = self.expanded_order.as_deref() == Some(&order.order_num);
        let toggle_label = if expanded { "▲" } else { "▼" };

        let toggle_btn = button(text(toggle_label).size(11))
            .on_press(Message::RowClicked(order.order_num.clone()));

        let status_label = text(&order.status_text)
            .size(12)
            .color(crate::style::order_status_color(&order.status_text, theme));
        let info = text(format!(
            "{} {} {}株 @{}",
            order.issue_code, order.order_qty, order.executed_qty, order.order_price
        ))
        .size(12);

        let mut row_widgets = row![toggle_btn, info, status_label]
            .spacing(8)
            .align_y(Alignment::Center);

        if order.is_cancelable() {
            let cancel_btn = button(text("取消").size(11))
                .on_press(Message::CancelClicked(order.order_num.clone()));
            let correct_btn = button(text("訂正").size(11))
                .on_press(Message::CorrectClicked(order.order_num.clone()));
            row_widgets = row_widgets.push(correct_btn).push(cancel_btn);
        }

        container(row_widgets)
            .padding(2)
            .style(container::bordered_box)
            .into()
    }

    fn view_execution_detail<'a>(&'a self, order_num: &str) -> Element<'a, Message> {
        match self.executions.get(order_num) {
            None => text("約定明細を読み込み中...").size(11).into(),
            Some(execs) if execs.is_empty() => text("約定なし").size(11).into(),
            Some(execs) => {
                let rows = execs.iter().map(|e| {
                    text(format!(
                        "  {}株 @{} ({})",
                        e.exec_qty, e.exec_price, e.exec_datetime
                    ))
                    .size(11)
                    .into()
                });
                column(rows)
                    .spacing(2)
                    .padding(iced::padding::left(16))
                    .into()
            }
        }
    }

    fn view_correct_modal<'a>(&'a self, modal: &'a CorrectModal) -> Element<'a, Message> {
        let title = text(format!("注文訂正: {}", modal.order_num)).size(14);
        let current_info = text(format!(
            "{} 現在: {}円 × {}株",
            modal.issue_code, modal.current_price, modal.current_qty
        ))
        .size(12);

        let price_input = text_input("変更なし（空欄可）", &modal.new_price)
            .on_input(Message::CorrectNewPriceChanged)
            .size(13)
            .width(iced::Length::Fixed(120.0));

        let qty_input = text_input("変更なし（空欄可）", &modal.new_qty)
            .on_input(Message::CorrectNewQtyChanged)
            .size(13)
            .width(iced::Length::Fixed(120.0));

        let password_input = text_input("発注パスワード", &modal.second_password)
            .on_input(Message::CorrectPasswordChanged)
            .secure(true)
            .size(13);

        let can_submit = !modal.second_password.is_empty();
        let cancel_btn = button(text("キャンセル").size(13)).on_press(Message::CorrectCancelled);
        let submit_btn = button(text("訂正を送信").size(13))
            .on_press_maybe(can_submit.then_some(Message::CorrectSubmitted));

        container(
            column![
                title,
                current_info,
                row![text("変更後の値段: ").size(13), price_input]
                    .spacing(4)
                    .align_y(Alignment::Center),
                row![text("変更後の株数: ").size(13), qty_input]
                    .spacing(4)
                    .align_y(Alignment::Center),
                row![text("発注パスワード: ").size(13), password_input]
                    .spacing(4)
                    .align_y(Alignment::Center),
                row![cancel_btn, submit_btn].spacing(8),
            ]
            .spacing(8)
            .padding(8),
        )
        .into()
    }

    fn view_cancel_modal<'a>(&'a self, modal: &'a CancelModal) -> Element<'a, Message> {
        let title = text("注文を取り消しますか？").size(14);
        let info = text(format!(
            "注文番号: {}  {}  {}株",
            modal.order_num, modal.issue_code, modal.order_qty
        ))
        .size(12);

        let password_input = text_input("発注パスワード", &modal.second_password)
            .on_input(Message::CancelPasswordChanged)
            .secure(true)
            .size(13);

        let can_submit = !modal.second_password.is_empty();
        let back_btn = button(text("戻る").size(13)).on_press(Message::CancelCancelled);
        let submit_btn = button(text("取消を送信").size(13))
            .on_press_maybe(can_submit.then_some(Message::CancelSubmitted));

        container(
            column![
                title,
                info,
                row![text("発注パスワード: ").size(13), password_input]
                    .spacing(4)
                    .align_y(Alignment::Center),
                row![back_btn, submit_btn].spacing(8),
            ]
            .spacing(8)
            .padding(8),
        )
        .into()
    }
}

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_order(num: &str, status: &str) -> OrderRecord {
        OrderRecord {
            order_num: num.to_string(),
            issue_code: "7203".to_string(),
            order_qty: "100".to_string(),
            current_qty: "100".to_string(),
            order_price: "2500".to_string(),
            order_datetime: "20260101090000".to_string(),
            status_text: status.to_string(),
            executed_qty: "0".to_string(),
            executed_price: "0".to_string(),
            eig_day: "20260101".to_string(),
        }
    }

    #[test]
    fn refresh_clicked_emits_fetch_orders() {
        let mut panel = OrderListPanel::new();
        let action = panel.update(Message::RefreshClicked);
        assert!(matches!(action, Some(Action::FetchOrders)));
        assert!(panel.loading);
    }

    #[test]
    fn orders_updated_clears_loading_and_saves_prev() {
        let mut panel = OrderListPanel::new();
        panel.loading = true;
        let orders = vec![make_order("001", "受付中")];
        panel.update(Message::OrdersUpdated(orders.clone()));
        assert!(!panel.loading);
        assert_eq!(panel.orders.len(), 1);
        assert!(panel.prev_orders.is_empty()); // prev was empty before
    }

    #[test]
    fn orders_updated_rotates_prev_orders() {
        let mut panel = OrderListPanel::new();
        let first = vec![make_order("001", "注文中")];
        panel.update(Message::OrdersUpdated(first));
        let second = vec![make_order("001", "全部約定")];
        panel.update(Message::OrdersUpdated(second));
        // prev_orders should be the first batch
        assert_eq!(panel.prev_orders[0].status_text, "注文中");
        assert_eq!(panel.orders[0].status_text, "全部約定");
    }

    #[test]
    fn newly_executed_detects_transition_to_full_execution() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "全部約定")];
        panel.prev_orders = vec![make_order("001", "注文中")];
        let newly = panel.newly_executed();
        assert_eq!(newly, vec!["001"]);
    }

    #[test]
    fn newly_executed_returns_empty_when_already_was_executed() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "全部約定")];
        panel.prev_orders = vec![make_order("001", "全部約定")];
        assert!(panel.newly_executed().is_empty());
    }

    #[test]
    fn row_clicked_expands_and_emits_fetch_detail() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "注文中")];
        let action = panel.update(Message::RowClicked("001".to_string()));
        assert_eq!(panel.expanded_order.as_deref(), Some("001"));
        assert!(matches!(action, Some(Action::FetchOrderDetail { .. })));
    }

    #[test]
    fn row_clicked_twice_collapses() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "注文中")];
        panel.update(Message::RowClicked("001".to_string()));
        // second click collapses (executions are cached now — seed them to avoid extra fetch)
        panel.executions.insert("001".to_string(), vec![]);
        panel.update(Message::RowClicked("001".to_string()));
        // collapse
        let _action = panel.update(Message::RowClicked("001".to_string()));
        panel.update(Message::RowClicked("001".to_string()));
        assert!(panel.expanded_order.is_none());
    }

    #[test]
    fn cancel_clicked_opens_cancel_modal() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "受付中")];
        panel.update(Message::CancelClicked("001".to_string()));
        assert!(panel.cancel_modal.is_some());
        let m = panel.cancel_modal.as_ref().unwrap();
        assert_eq!(m.order_num, "001");
    }

    #[test]
    fn cancel_submitted_requires_password() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "受付中")];
        panel.update(Message::CancelClicked("001".to_string()));
        // no password → no action
        let action = panel.update(Message::CancelSubmitted);
        assert!(action.is_none());
        assert!(panel.cancel_modal.is_some()); // modal stays open
    }

    #[test]
    fn cancel_submitted_with_password_emits_action_and_closes_modal() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "受付中")];
        panel.update(Message::CancelClicked("001".to_string()));
        panel.update(Message::CancelPasswordChanged("pass".to_string()));
        let action = panel.update(Message::CancelSubmitted);
        assert!(matches!(action, Some(Action::SubmitCancel(_))));
        assert!(panel.cancel_modal.is_none());
    }

    #[test]
    fn correct_submitted_maps_empty_fields_to_asterisk() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "注文中")];
        panel.update(Message::CorrectClicked("001".to_string()));
        panel.update(Message::CorrectPasswordChanged("pass".to_string()));
        // new_price and new_qty are left empty → "*"
        let action = panel.update(Message::CorrectSubmitted);
        if let Some(Action::SubmitCorrect(req)) = action {
            assert_eq!(req.price, "*");
            assert_eq!(req.qty, "*");
            assert_eq!(req.condition, "*");
            assert_eq!(req.expire_day, "*");
        } else {
            panic!("expected SubmitCorrect action");
        }
    }

    #[test]
    fn correct_submitted_uses_entered_price_and_qty() {
        let mut panel = OrderListPanel::new();
        panel.orders = vec![make_order("001", "注文中")];
        panel.update(Message::CorrectClicked("001".to_string()));
        panel.update(Message::CorrectNewPriceChanged("2600".to_string()));
        panel.update(Message::CorrectNewQtyChanged("50".to_string()));
        panel.update(Message::CorrectPasswordChanged("pass".to_string()));
        let action = panel.update(Message::CorrectSubmitted);
        if let Some(Action::SubmitCorrect(req)) = action {
            assert_eq!(req.price, "2600");
            assert_eq!(req.qty, "50");
        } else {
            panic!("expected SubmitCorrect action");
        }
    }

    #[test]
    fn modify_completed_ok_triggers_fetch_orders() {
        let mut panel = OrderListPanel::new();
        let action = panel.update(Message::ModifyCompleted(Ok("001".to_string())));
        assert!(matches!(action, Some(Action::FetchOrders)));
        assert!(panel.last_error.is_none());
    }

    #[test]
    fn modify_completed_err_stores_error() {
        let mut panel = OrderListPanel::new();
        let action = panel.update(Message::ModifyCompleted(Err("エラー".to_string())));
        assert!(action.is_none());
        assert!(panel.last_error.is_some());
    }

    #[test]
    fn poll_tick_emits_fetch_orders() {
        let mut panel = OrderListPanel::new();
        let action = panel.update(Message::PollTick);
        assert!(matches!(action, Some(Action::FetchOrders)));
    }

    #[test]
    fn executions_updated_inserts_into_cache() {
        let mut panel = OrderListPanel::new();
        panel.update(Message::ExecutionsUpdated {
            order_num: "001".to_string(),
            executions: vec![],
        });
        assert!(panel.executions.contains_key("001"));
    }
}
