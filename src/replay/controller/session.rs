use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::super::{
    ReplayLoadEvent, ReplayMessage, ReplayMode, ReplaySession, ReplayUserMessage, clock::StepClock,
    loader, min_timeframe_ms, store::EventStore, store::LoadedData,
};
use super::ReplayController;

impl ReplayController {
    pub fn handle_user_message(
        &mut self,
        msg: ReplayUserMessage,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        match msg {
            ReplayUserMessage::ToggleMode => {
                let was_replay = self.state.is_replay();
                self.state.toggle_mode();
                if was_replay && !self.state.is_replay() {
                    dashboard.rebuild_for_live(main_window_id);
                } else if !was_replay && self.state.is_replay() {
                    dashboard.clear_chart_for_replay(main_window_id);
                }
                (Task::none(), None)
            }

            ReplayUserMessage::StartTimeChanged(s) => {
                self.state.range_input.start = s;
                (Task::none(), None)
            }

            ReplayUserMessage::EndTimeChanged(s) => {
                self.state.range_input.end = s;
                (Task::none(), None)
            }
        }
    }

    pub fn initialize_session(
        &mut self,
        start: &str,
        end: &str,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Result<Task<ReplayMessage>, String> {
        use std::collections::HashSet;

        let (start_ms, end_ms) =
            super::super::parse_replay_range(start, end).map_err(|err| err.to_string())?;
        let kline_targets = dashboard.prepare_replay(main_window_id);
        if kline_targets.is_empty() {
            return Err("no ready kline streams; configure at least one chart first".to_string());
        }

        let active_streams: HashSet<_> = kline_targets
            .into_iter()
            .map(|(_, stream)| stream)
            .collect();
        let step_size_ms = min_timeframe_ms(&active_streams);
        let clock = StepClock::new(start_ms, end_ms, step_size_ms);

        self.state.mode = ReplayMode::Replay;
        self.state.range_input.start = start.to_string();
        self.state.range_input.end = end.to_string();
        self.state.session = ReplaySession::Loading {
            clock,
            pending_count: active_streams.len(),
            store: EventStore::new(),
            active_streams: active_streams.clone(),
        };

        let tasks = active_streams
            .into_iter()
            .map(|stream| {
                let stream_step_ms = stream
                    .as_kline_stream()
                    .map(|(_, tf)| tf.to_milliseconds())
                    .unwrap_or(step_size_ms);
                let range = super::super::compute_load_range(start_ms, end_ms, stream_step_ms);
                Task::perform(loader::load_klines(stream, range), |result| match result {
                    Ok(r) => ReplayMessage::Load(ReplayLoadEvent::KlinesLoadCompleted(
                        r.stream, r.range, r.klines,
                    )),
                    Err(e) => ReplayMessage::Load(ReplayLoadEvent::DataLoadFailed(e)),
                })
            })
            .collect::<Vec<_>>();

        Ok(Task::batch(tasks))
    }

    pub fn handle_load_event(
        &mut self,
        event: ReplayLoadEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Option<Toast> {
        match event {
            ReplayLoadEvent::KlinesLoadCompleted(stream, range, klines) => {
                let should_activate = if let ReplaySession::Loading {
                    pending_count,
                    store,
                    ..
                } = &mut self.state.session
                {
                    store.ingest_loaded(
                        stream,
                        range,
                        LoadedData {
                            klines: klines.clone(),
                            trades: vec![],
                        },
                    );
                    *pending_count = pending_count.saturating_sub(1);
                    *pending_count == 0
                } else {
                    false
                };

                if should_activate {
                    let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
                    if let ReplaySession::Loading {
                        clock,
                        store,
                        active_streams,
                        ..
                    } = old
                    {
                        self.state.session = ReplaySession::Active {
                            clock,
                            store,
                            active_streams,
                        };
                    }
                }

                let start_ms = match &self.state.session {
                    ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                        clock.full_range().start
                    }
                    ReplaySession::Idle => 0,
                };
                let history_klines = super::super::pre_start_history(&klines, start_ms);
                if !history_klines.is_empty() {
                    dashboard.ingest_replay_klines(&stream, &history_klines, main_window_id);
                }
                None
            }

            ReplayLoadEvent::DataLoadFailed(err) => {
                self.reset_session();
                Some(Toast::error(format!("Replay data load failed: {err}")))
            }
        }
    }

    fn reset_session(&mut self) {
        self.state.session = ReplaySession::Idle;
    }
}
