use std::cmp;

use crate::screen::dashboard::Dashboard;

use super::{ReplayController, ReplaySession};

const UI_ADVANCE_CAP_MS: u64 = 3_600_000; // 1時間

impl ReplayController {
    /// AgentMessage::Step（UIの ▶ ボタン）
    pub fn agent_step(&mut self, dashboard: &mut Dashboard, main_window_id: iced::window::Id) {
        let (current, step, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => (clock.now_ms(), clock.step_size_ms(), clock.full_range().end),
            _ => return,
        };

        let next_ms = cmp::min(current + step, end);
        if next_ms > current {
            // seek_to はチャートを全リセットして全再注入するため、1バーの進行には重い可能性があるが、
            // 現在の seek_to 実装を利用する。将来的には差分 inject に最適化する。
            self.seek_to(next_ms, dashboard, main_window_id);
        }
    }

    /// AgentMessage::Advance（UIの ⏭ ボタン）
    pub fn agent_advance(&mut self, dashboard: &mut Dashboard, main_window_id: iced::window::Id) {
        let (current, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => (clock.now_ms(), clock.full_range().end),
            _ => return,
        };

        let next_ms = cmp::min(current + UI_ADVANCE_CAP_MS, end);
        if next_ms > current {
            self.seek_to(next_ms, dashboard, main_window_id);
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
}
