use crate::replay::{ReplayMessage, ReplaySystemEvent};
use crate::{Flowsurface, Message};
use iced::Task;

impl Flowsurface {
    pub(crate) fn pane_api_set_ticker(
        &mut self,
        pane_id: uuid::Uuid,
        ticker_str: &str,
    ) -> (u16, String, Task<Message>) {
        let ticker = match Self::parse_ser_ticker(ticker_str) {
            Ok(t) => t,
            Err(err) => {
                return (
                    400,
                    format!(r#"{{"error":"{}"}}"#, err.replace('"', "'")),
                    Task::none(),
                );
            }
        };

        let ticker_info = self.resolve_ticker_info(&ticker);
        let Some(ticker_info) = ticker_info else {
            return (
                404,
                format!(
                    r#"{{"error":"ticker info not loaded yet: {ticker_str} (wait for metadata fetch)"}}"#
                ),
                Task::none(),
            );
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                404,
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let main_window_id = self.main_window.id;

        let is_replay = self.replay.is_replay();
        let old_kline_stream_for_replay = if is_replay {
            self.active_dashboard().and_then(|d| {
                d.iter_all_panes(main_window_id)
                    .find(|(_, p, _)| *p == pg_pane)
                    .and_then(|(_, _, state)| {
                        state
                            .streams
                            .ready_iter()
                            .and_then(|mut it| {
                                it.find(|s| {
                                    matches!(s, exchange::adapter::StreamKind::Kline { .. })
                                })
                            })
                            .copied()
                    })
            })
        } else {
            None
        };

        let Some(dashboard) = self.active_dashboard_mut() else {
            return (
                500,
                r#"{"error":"no active dashboard"}"#.to_string(),
                Task::none(),
            );
        };
        // init_focused_pane は dashboard.focus を使って対象ペインを決定するため、
        // focus を一時的に pg_pane に変更し、呼び出し後に元の値へ戻す。
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        let kind = dashboard
            .iter_all_panes(main_window_id)
            .find(|(_, p, _)| *p == pg_pane)
            .map(|(_, _, state)| state.content.kind())
            .map(|k| match k {
                data::layout::pane::ContentKind::Starter => {
                    data::layout::pane::ContentKind::CandlestickChart
                }
                other => other,
            })
            .unwrap_or(data::layout::pane::ContentKind::CandlestickChart);

        let replay_task = if let Some(old) = old_kline_stream_for_replay
            && let Some((_, tf)) = old.as_kline_stream()
        {
            let new_stream = exchange::adapter::StreamKind::Kline {
                ticker_info,
                timeframe: tf,
            };
            Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::ReloadKlineStream {
                    old_stream: Some(old),
                    new_stream,
                },
            )))
        } else {
            Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::SyncReplayBuffers,
            )))
        };

        let init_task = dashboard
            .init_focused_pane(main_window_id, ticker_info, kind, is_replay)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            });
        let task = Task::batch([init_task, replay_task]);

        if let Some(d) = self.active_dashboard_mut() {
            d.focus = prev_focus;
        }

        let ok = serde_json::json!({
            "ok": true,
            "action": "set-ticker",
            "pane_id": pane_id.to_string(),
            "ticker": ticker_str,
        });
        (200, ok.to_string(), task)
    }

    pub(crate) fn pane_api_set_timeframe(
        &mut self,
        pane_id: uuid::Uuid,
        tf_str: &str,
    ) -> (u16, String, Task<Message>) {
        let tf = match Self::parse_timeframe(tf_str) {
            Some(tf) => tf,
            None => {
                return (
                    400,
                    format!(r#"{{"error":"invalid timeframe: {tf_str}"}}"#),
                    Task::none(),
                );
            }
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                404,
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let main_window_id = self.main_window.id;
        let (ticker_info, kind, old_kline_stream_for_replay) = {
            let Some(dashboard) = self.active_dashboard() else {
                return (
                    500,
                    r#"{"error":"no active dashboard"}"#.to_string(),
                    Task::none(),
                );
            };
            let Some((_, _, state)) = dashboard
                .iter_all_panes(main_window_id)
                .find(|(_, p, _)| *p == pg_pane)
            else {
                return (
                    404,
                    format!(r#"{{"error":"pane not found in dashboard: {pane_id}"}}"#),
                    Task::none(),
                );
            };
            let Some(ti) = state.stream_pair() else {
                return (
                    400,
                    format!(
                        r#"{{"error":"pane has no active ticker to rebase timeframe: {pane_id}"}}"#
                    ),
                    Task::none(),
                );
            };
            let old_kline = if self.replay.is_replay() {
                state
                    .streams
                    .ready_iter()
                    .and_then(|mut it| {
                        it.find(|s| matches!(s, exchange::adapter::StreamKind::Kline { .. }))
                    })
                    .copied()
            } else {
                None
            };
            (ti, state.content.kind(), old_kline)
        };

        let is_replay = self.replay.is_replay();
        let Some(dashboard) = self.active_dashboard_mut() else {
            return (
                500,
                r#"{"error":"no active dashboard"}"#.to_string(),
                Task::none(),
            );
        };
        // init_focused_pane は dashboard.focus を使って対象ペインを決定するため、
        // focus を一時的に pg_pane に変更し、呼び出し後に元の値へ戻す。
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        if let Some(state) = dashboard
            .iter_all_panes_mut(main_window_id)
            .find(|(_, p, _)| *p == pg_pane)
            .map(|(_, _, s)| s)
        {
            state.settings.selected_basis = Some(data::chart::Basis::Time(tf));
        }

        let replay_task = if is_replay {
            let new_stream = exchange::adapter::StreamKind::Kline {
                ticker_info,
                timeframe: tf,
            };
            Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::ReloadKlineStream {
                    old_stream: old_kline_stream_for_replay,
                    new_stream,
                },
            )))
        } else {
            Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::SyncReplayBuffers,
            )))
        };

        let init_task = dashboard
            .init_focused_pane(main_window_id, ticker_info, kind, is_replay)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            });
        let task = Task::batch([init_task, replay_task]);

        if let Some(d) = self.active_dashboard_mut() {
            d.focus = prev_focus;
        }

        let ok = serde_json::json!({
            "ok": true,
            "action": "set-timeframe",
            "pane_id": pane_id.to_string(),
            "timeframe": tf_str,
        });
        (200, ok.to_string(), task)
    }

    pub(crate) fn pane_api_sidebar_select_ticker(
        &mut self,
        pane_id: uuid::Uuid,
        ticker_str: &str,
        kind_str: Option<&str>,
    ) -> (u16, String, Task<Message>) {
        let ticker = match Self::parse_ser_ticker(ticker_str) {
            Ok(t) => t,
            Err(err) => {
                return (
                    400,
                    format!(r#"{{"error":"{}"}}"#, err.replace('"', "'")),
                    Task::none(),
                );
            }
        };

        let ticker_info = self.resolve_ticker_info(&ticker);
        let Some(ticker_info) = ticker_info else {
            return (
                404,
                format!(
                    r#"{{"error":"ticker info not loaded yet: {ticker_str} (wait for metadata fetch)"}}"#
                ),
                Task::none(),
            );
        };

        let kind = match kind_str {
            Some(s) => match Self::parse_content_kind(s) {
                Some(k) => Some(k),
                None => {
                    return (
                        400,
                        format!(r#"{{"error":"invalid kind: {s}"}}"#),
                        Task::none(),
                    );
                }
            },
            None => None,
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                404,
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let main_window_id = self.main_window.id;

        if let Some(kind) = kind {
            let Some(dashboard) = self.active_dashboard_mut() else {
                return (
                    500,
                    r#"{"error":"no active dashboard"}"#.to_string(),
                    Task::none(),
                );
            };
            dashboard.focus = Some((window_id, pg_pane));

            let task = match dashboard.split_focused_and_init(main_window_id, ticker_info, kind) {
                Some(split_task) => {
                    let sync_task = Task::done(Message::Replay(ReplayMessage::System(
                        ReplaySystemEvent::SyncReplayBuffers,
                    )));
                    Task::batch([
                        split_task.map(move |msg| Message::Dashboard {
                            layout_id: None,
                            event: msg,
                        }),
                        sync_task,
                    ])
                }
                None => {
                    if let Some(d) = self.active_dashboard_mut() {
                        d.focus = None;
                    }
                    Task::none()
                }
            };

            let ok = serde_json::json!({
                "ok": true,
                "action": "sidebar-select-ticker",
                "pane_id": pane_id.to_string(),
                "ticker": ticker_str,
                "kind": kind_str,
            });
            return (200, ok.to_string(), task);
        }

        // kind == None → switch_tickers_in_group 経路
        let is_replay = self.replay.is_replay();
        let replay_task = self.make_kline_reload_task(ticker_info);

        let Some(dashboard) = self.active_dashboard_mut() else {
            return (
                500,
                r#"{"error":"no active dashboard"}"#.to_string(),
                Task::none(),
            );
        };
        // init_focused_pane は dashboard.focus を使って対象ペインを決定するため、
        // focus を一時的に pg_pane に変更し、呼び出し後に元の値へ戻す。
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        let task = dashboard.switch_tickers_in_group(main_window_id, ticker_info, is_replay);

        let task = Task::batch([
            task.map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            }),
            replay_task,
        ]);

        if let Some(d) = self.active_dashboard_mut() {
            d.focus = prev_focus;
        }

        let ok = serde_json::json!({
            "ok": true,
            "action": "sidebar-select-ticker",
            "pane_id": pane_id.to_string(),
            "ticker": ticker_str,
            "kind": kind_str,
        });
        (200, ok.to_string(), task)
    }
}
