use crate::connector::order as order_connector;
use iced::Task;

use super::{Dashboard, Message, pane};

impl Dashboard {
    pub(super) fn order_effect_task(
        effect: pane::Effect,
        is_replay: bool,
        pane_id: uuid::Uuid,
        eig_day: String,
    ) -> Task<Message> {
        match effect {
            effect @ (pane::Effect::SubmitNewOrder(_)
            | pane::Effect::SubmitCorrectOrder(_)
            | pane::Effect::SubmitCancelOrder(_)) => {
                Self::submit_effect_task(effect, is_replay, pane_id)
            }
            effect => Self::fetch_effect_task(effect, pane_id, eig_day),
        }
    }

    fn submit_effect_task(
        effect: pane::Effect,
        is_replay: bool,
        pane_id: uuid::Uuid,
    ) -> Task<Message> {
        match effect {
            effect @ (pane::Effect::SubmitNewOrder(_)
            | pane::Effect::SubmitCorrectOrder(_)
            | pane::Effect::SubmitCancelOrder(_))
                if is_replay =>
            {
                log::warn!("REPLAY中の発注はブロックされました: {:?}", effect);
                Task::none()
            }
            pane::Effect::SubmitNewOrder(req) => {
                Task::perform(order_connector::submit_new_order(req), move |result| {
                    Message::OrderNewResult { pane_id, result }
                })
            }
            pane::Effect::SubmitCorrectOrder(req) => {
                Task::perform(order_connector::submit_correct_order(req), move |result| {
                    Message::OrderModifyResult {
                        pane_id,
                        result: result.map(|r| r.order_number),
                    }
                })
            }
            pane::Effect::SubmitCancelOrder(req) => {
                Task::perform(order_connector::submit_cancel_order(req), move |result| {
                    Message::OrderModifyResult {
                        pane_id,
                        result: result.map(|r| r.order_number),
                    }
                })
            }
            // order_effect_task が SubmitNew/Correct/Cancel のみをここに渡すことを保証する
            _ => unreachable!("non-submit effect passed to submit_effect_task"),
        }
    }

    fn fetch_effect_task(
        effect: pane::Effect,
        pane_id: uuid::Uuid,
        eig_day: String,
    ) -> Task<Message> {
        match effect {
            pane::Effect::FetchOrders => {
                Task::perform(order_connector::fetch_orders(eig_day), move |result| {
                    Message::OrdersListResult { pane_id, result }
                })
            }
            pane::Effect::FetchOrderDetail {
                order_num,
                eig_day: detail_eig_day,
            } => Task::perform(
                order_connector::fetch_order_detail(order_num.clone(), detail_eig_day),
                move |result| Message::OrderDetailResult {
                    pane_id,
                    order_num,
                    result,
                },
            ),
            pane::Effect::FetchBuyingPower => {
                Task::perform(order_connector::fetch_buying_power(), move |result| {
                    Message::BuyingPowerResult { pane_id, result }
                })
            }
            pane::Effect::FetchHoldings { issue_code } => {
                Task::perform(order_connector::fetch_holdings(issue_code), move |result| {
                    Message::HoldingsResult { pane_id, result }
                })
            }
            // order_effect_task が FetchOrders/Detail/BuyingPower/Holdings のみをここに渡すことを保証する
            _ => unreachable!("non-fetch effect passed to fetch_effect_task"),
        }
    }
}
