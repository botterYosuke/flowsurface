use std::cmp;

use exchange::Trade;
use exchange::adapter::StreamKind;

use crate::screen::dashboard::Dashboard;

use super::{ReplayController, ReplaySession};

/// agent_step / agent_advance の進行結果。
/// `.0` は進行後の clock_ms、`.1` は active stream ごとに抽出された trade イベント。
pub type AgentStepOutcome = (u64, Vec<(StreamKind, Vec<Trade>)>);

impl ReplayController {
    /// UI 発火の advance に対する進行上限（1時間）。
    /// caller decides cap policy: ADR-0001 §5 に従い、UI の advance は cap を適用するが
    /// HTTP 経由の agent_advance は cap なしを許容するため、定数として外出し。
    pub const UI_ADVANCE_CAP_MS: u64 = 3_600_000;

    /// `AgentMessage::Step`（UI の ▶ ボタン）。1 bar 進める。
    /// session が `Active` でない場合は `None`（caller で 400 / warn 相当の扱い）。
    pub fn agent_step(
        &mut self,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Option<AgentStepOutcome> {
        let (current, step, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => {
                (clock.now_ms(), clock.step_size_ms(), clock.full_range().end)
            }
            _ => return None,
        };

        let next_ms = cmp::min(current + step, end);
        if next_ms > current {
            Some(self.step_with_dispatch(next_ms, dashboard, main_window_id))
        } else {
            Some((current, vec![]))
        }
    }

    /// `AgentMessage::Advance`（UI の ⏭ ボタン）。`cap_ms` だけ進める。
    /// session が `Active` でない場合は `None`。
    pub fn agent_advance(
        &mut self,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
        cap_ms: u64,
    ) -> Option<AgentStepOutcome> {
        let (current, end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => (clock.now_ms(), clock.full_range().end),
            _ => return None,
        };

        let next_ms = cmp::min(current + cap_ms, end);
        if next_ms > current {
            Some(self.step_with_dispatch(next_ms, dashboard, main_window_id))
        } else {
            Some((current, vec![]))
        }
    }

    /// `AgentMessage::RewindToStart`（UI の ⏮ ボタン）。clock を `range.start` に戻す。
    ///
    /// NOTE: ADR-0001 §4 Reset 不変条件のうち、本メソッドは clock 巻き戻しと
    /// chart の seek-reset + 再注入のみを担う。`SessionLifecycleEvent::Reset` 発火 /
    /// `client_order_id` map クリア / `NarrativeState::Reset` / `VirtualExchange::reset`
    /// は呼び出し側（`src/app/handlers.rs::handle_agent`）の責務で、さらに完全実装は
    /// 後続サブフェーズに分離されている（計画書 Q 参照）。
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
    ) -> AgentStepOutcome {
        use super::super::dispatcher::dispatch_tick;

        let (store, active_streams, clock) = match &mut self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => (store, active_streams, clock),
            _ => return (0, vec![]),
        };

        let result = dispatch_tick(clock, store, active_streams, target_ms);
        let current_time = result.current_time;
        for (stream, klines) in &result.kline_events {
            if !klines.is_empty() {
                dashboard.ingest_replay_klines(stream, klines, main_window_id);
            }
        }
        for (stream, trades) in &result.trade_events {
            if !trades.is_empty() {
                let _ = dashboard.ingest_trades(stream, trades, current_time, main_window_id);
            }
        }

        (current_time, result.trade_events)
    }
}
