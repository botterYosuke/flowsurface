use iced::Task;

use super::{Dashboard, Event, Message, pane};
use crate::window;

impl Dashboard {
    pub(super) fn handle_virtual_order_filled(
        &self,
        fill: crate::replay::virtual_exchange::FillEvent,
    ) -> (Task<Message>, Option<Event>) {
        let side_str = match fill.side {
            crate::replay::virtual_exchange::PositionSide::Long => "買い",
            crate::replay::virtual_exchange::PositionSide::Short => "売り",
        };
        let msg = format!(
            "[仮想] 約定: {} {} {:.4} @ {:.2}",
            fill.ticker, side_str, fill.qty, fill.fill_price
        );
        (
            Task::none(),
            Some(Event::Notification(crate::widget::toast::Toast::info(msg))),
        )
    }

    pub fn sync_virtual_mode(&mut self, main_window: window::Id) {
        let virtual_mode = self.is_replay;
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            state.is_virtual_mode = virtual_mode;
            if let pane::Content::OrderEntry(panel) = &mut state.content {
                panel.is_virtual = virtual_mode;
            }
        }
    }

    pub fn ingest_replay_klines(
        &mut self,
        stream: &exchange::adapter::StreamKind,
        klines: &[exchange::Kline],
        main_window: window::Id,
    ) {
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            let has_stream = state
                .streams
                .ready_iter()
                .is_some_and(|mut iter| iter.any(|s| s == stream));
            if has_stream {
                state.ingest_replay_klines(klines);
            }
        }
    }
}
