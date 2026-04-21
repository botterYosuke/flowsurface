use super::{Dashboard, pane, panel};
use crate::window;

impl Dashboard {
    pub(super) fn handle_order_new_result(
        &mut self,
        pane_id: uuid::Uuid,
        result: Result<exchange::adapter::tachibana::NewOrderResponse, String>,
        main_window: window::Id,
    ) {
        use panel::order_entry::OrderSuccess;
        let order_result = match result {
            Ok(ref resp) => {
                if !resp.eig_day.is_empty() {
                    self.eig_day = Some(resp.eig_day.clone());
                }
                Ok(OrderSuccess {
                    order_num: resp.order_number.clone(),
                    warning: if resp.warning_code == "0" || resp.warning_code.is_empty() {
                        None
                    } else {
                        Some(resp.warning_text.clone())
                    },
                })
            }
            Err(e) => Err(e),
        };
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::OrderEntry(panel) = &mut state.content
        {
            panel.update(panel::order_entry::Message::OrderCompleted(order_result));
        }
    }

    pub(super) fn handle_order_modify_result(
        &mut self,
        pane_id: uuid::Uuid,
        result: Result<String, String>,
        main_window: window::Id,
    ) {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::OrderList(panel) = &mut state.content
        {
            panel.update(panel::order_list::Message::ModifyCompleted(result));
        }
    }

    pub(super) fn handle_orders_list_result(
        &mut self,
        pane_id: uuid::Uuid,
        result: Result<Vec<exchange::adapter::tachibana::OrderRecord>, String>,
        main_window: window::Id,
    ) {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::OrderList(panel) = &mut state.content
        {
            match result {
                Ok(orders) => {
                    panel.update(panel::order_list::Message::OrdersUpdated(orders));
                }
                Err(e) => {
                    panel.update(panel::order_list::Message::ModifyCompleted(Err(e)));
                }
            }
        }
    }

    pub(super) fn handle_order_detail_result(
        &mut self,
        pane_id: uuid::Uuid,
        order_num: String,
        result: Result<Vec<exchange::adapter::tachibana::ExecutionRecord>, String>,
        main_window: window::Id,
    ) {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::OrderList(panel) = &mut state.content
        {
            match result {
                Ok(executions) => {
                    panel.update(panel::order_list::Message::ExecutionsUpdated {
                        order_num,
                        executions,
                    });
                }
                Err(e) => {
                    panel.update(panel::order_list::Message::ModifyCompleted(Err(e)));
                }
            }
        }
    }

    pub(super) fn handle_buying_power_result(
        &mut self,
        pane_id: uuid::Uuid,
        result: Result<
            (
                exchange::adapter::tachibana::BuyingPowerResponse,
                exchange::adapter::tachibana::MarginPowerResponse,
            ),
            String,
        >,
        main_window: window::Id,
    ) {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::BuyingPower(panel) = &mut state.content
        {
            match result {
                Ok((cash, margin)) => {
                    panel.update(panel::buying_power::Message::BuyingPowerUpdated {
                        cash_buying_power: cash.cash_buying_power,
                        nisa_growth_buying_power: cash.nisa_growth_buying_power,
                        shortage_flag: cash.shortage_flag,
                    });
                    panel.update(panel::buying_power::Message::MarginPowerUpdated {
                        margin_new_order_power: margin.margin_new_order_power,
                        maintenance_margin_rate: margin.maintenance_margin_rate,
                        margin_call_flag: margin.margin_call_flag,
                    });
                }
                Err(e) => {
                    panel.update(panel::buying_power::Message::FetchFailed(e));
                }
            }
        }
    }

    pub(super) fn handle_holdings_result(
        &mut self,
        pane_id: uuid::Uuid,
        result: Result<u64, String>,
        main_window: window::Id,
    ) {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id)
            && let pane::Content::OrderEntry(panel) = &mut state.content
        {
            panel.update(panel::order_entry::Message::HoldingsUpdated(result.ok()));
        }
    }

    pub(super) fn eig_day_or_today(&self) -> String {
        self.eig_day.clone().unwrap_or_else(|| {
            let now = chrono::Local::now();
            now.format("%Y%m%d").to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eig_day_or_today_returns_stored_value_when_set() {
        let dashboard = Dashboard {
            eig_day: Some("20240417".to_string()),
            ..Default::default()
        };
        assert_eq!(dashboard.eig_day_or_today(), "20240417");
    }

    #[test]
    fn eig_day_or_today_returns_today_in_yyyymmdd_format_when_not_set() {
        let dashboard = Dashboard::default();
        let result = dashboard.eig_day_or_today();
        assert_eq!(result.len(), 8, "fallback must be 8-char YYYYMMDD");
        assert!(
            result.chars().all(|c| c.is_ascii_digit()),
            "fallback must be all digits, got: {result}"
        );
    }
}
