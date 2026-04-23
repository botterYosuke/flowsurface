use std::cmp;

use crate::screen::dashboard::Dashboard;

use super::{ReplayController, ReplaySession};

impl ReplayController {

    /// AgentMessage::Step（UIの ▶ ボタン）
    pub fn agent_step(
        &mut self,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (u64, Vec<(exchange::adapter::StreamKind, Vec<exchange::Trade>)>) {
        let (current, step, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => (clock.now_ms(), clock.step_size_ms(), clock.full_range().end),
            _ => return (0, vec![]),
        };

        let next_ms = cmp::min(current + step, end);
        if next_ms > current {
            self.step_with_dispatch(next_ms, dashboard, main_window_id)
        } else {
            (current, vec![])
        }
    }

    /// AgentMessage::Advance（UIの ⏭ ボタン）
    pub fn agent_advance(
        &mut self,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
        cap_ms: u64,
    ) -> (u64, Vec<(exchange::adapter::StreamKind, Vec<exchange::Trade>)>) {
        let (current, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => (clock.now_ms(), clock.full_range().end),
            _ => return (0, vec![]),
        };

        let next_ms = cmp::min(current + cap_ms, end);
        if next_ms > current {
            self.step_with_dispatch(next_ms, dashboard, main_window_id)
        } else {
            (current, vec![])
        }
    }

    /// AgentMessage::RewindToStart（UIの ⏮ ボタン）
    pub fn agent_rewind(&mut self, dashboard: &mut Dashboard, main_window_id: iced::window::Id) {
        let start = match &self.state.session {
            ReplaySession::Active { clock, .. } | ReplaySession::Loading { clock, .. } => {
                clock.full_range().start
            }
            ReplaySession::Idle => return,
        };

        self.seek_to(start, dashboard, main_window_id);
    }

    fn step_with_dispatch(
        &mut self,
        target_ms: u64,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (u64, Vec<(exchange::adapter::StreamKind, Vec<exchange::Trade>)>) {
        use super::super::dispatcher::dispatch_tick;

        let (store, active_streams, clock) = match &mut self.state.session {
            ReplaySession::Active { clock, store, active_streams } => {
                (store, active_streams, clock)
            }
            _ => return (0, vec![]),
        };

        let result = dispatch_tick(clock, store, active_streams, target_ms);
        let current_time = result.current_time;
        for (stream, klines) in result.kline_events.into_iter() {
            let klines: Vec<exchange::Kline> = klines;
            if !klines.is_empty() {
                dashboard.ingest_replay_klines(&stream, &klines, main_window_id);
            }
        }
        for (stream, trades) in &result.trade_events {
            let trades: &Vec<exchange::Trade> = trades;
            if !trades.is_empty() {
                let _ = dashboard.ingest_trades(stream, trades, current_time, main_window_id);
            }
        }

        (current_time, result.trade_events)
    }
}
