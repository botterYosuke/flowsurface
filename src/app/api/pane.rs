use crate::replay_api::PaneCommand;
use crate::screen::dashboard;
use crate::widget::toast::Status;
use crate::window;
use crate::{Flowsurface, Message};
use iced::{Task, widget::pane_grid};

use super::helpers::extract_pane_ticker_timeframe;

impl Flowsurface {
    pub(crate) fn handle_pane_api(&mut self, cmd: PaneCommand) -> (u16, String, Task<Message>) {
        match cmd {
            PaneCommand::ListPanes => {
                let json = self.build_pane_list_json();
                (200, json, Task::none())
            }
            PaneCommand::Split { pane_id, axis } => self.pane_api_split(pane_id, &axis),
            PaneCommand::Close { pane_id } => self.pane_api_close(pane_id),
            PaneCommand::SetTicker { pane_id, ticker } => {
                self.pane_api_set_ticker(pane_id, &ticker)
            }
            PaneCommand::SetTimeframe { pane_id, timeframe } => {
                self.pane_api_set_timeframe(pane_id, &timeframe)
            }
            PaneCommand::SidebarSelectTicker {
                pane_id,
                ticker,
                kind,
            } => self.pane_api_sidebar_select_ticker(pane_id, &ticker, kind.as_deref()),
            PaneCommand::ListNotifications => {
                let json = self.build_notification_list_json();
                (200, json, Task::none())
            }
            PaneCommand::GetChartSnapshot { pane_id } => {
                let json = self.build_chart_snapshot_json(pane_id);
                (200, json, Task::none())
            }
            PaneCommand::OpenOrderPane { kind } => self.pane_api_open_order_pane(&kind),
        }
    }

    pub(crate) fn find_pane_handle(
        &self,
        pane_id: uuid::Uuid,
    ) -> Option<(window::Id, pane_grid::Pane)> {
        let main_window_id = self.main_window.id;
        self.active_dashboard()?
            .iter_all_panes(main_window_id)
            .find(|(_, _, state)| state.unique_id() == pane_id)
            .map(|(win, pg, _)| (win, pg))
    }

    pub(crate) fn pane_api_open_order_pane(
        &mut self,
        kind_str: &str,
    ) -> (u16, String, Task<Message>) {
        let kind = match Self::parse_content_kind(kind_str) {
            Some(k) => k,
            None => {
                return (
                    400,
                    format!(r#"{{"error":"invalid kind: {kind_str}"}}"#),
                    Task::none(),
                );
            }
        };
        let main_window_id = self.main_window.id;
        let Some(d) = self.active_dashboard_mut() else {
            return (
                500,
                r#"{"error":"no active dashboard"}"#.to_string(),
                Task::none(),
            );
        };
        let task = d
            .split_focused_and_init_order(main_window_id, kind)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            });
        let ok = serde_json::json!({
            "ok": true,
            "action": "open-order-pane",
            "kind": kind_str,
        });
        (200, ok.to_string(), task)
    }

    pub(crate) fn pane_api_split(
        &mut self,
        pane_id: uuid::Uuid,
        axis_str: &str,
    ) -> (u16, String, Task<Message>) {
        let axis = match axis_str {
            "Vertical" | "vertical" => pane_grid::Axis::Vertical,
            "Horizontal" | "horizontal" => pane_grid::Axis::Horizontal,
            _ => {
                return (
                    400,
                    format!(
                        r#"{{"error":"invalid axis: {axis_str} (expected Vertical or Horizontal)"}}"#
                    ),
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

        let task = self.handle_dashboard_message(
            None,
            dashboard::Message::Pane(
                window_id,
                dashboard::pane::Message::SplitPane(axis, pg_pane),
            ),
        );
        let ok = serde_json::json!({"ok": true, "action": "split", "pane_id": pane_id.to_string()});
        (200, ok.to_string(), task)
    }

    pub(crate) fn pane_api_close(&mut self, pane_id: uuid::Uuid) -> (u16, String, Task<Message>) {
        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                404,
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let task = self.handle_dashboard_message(
            None,
            dashboard::Message::Pane(window_id, dashboard::pane::Message::ClosePane(pg_pane)),
        );
        let ok = serde_json::json!({"ok": true, "action": "close", "pane_id": pane_id.to_string()});
        (200, ok.to_string(), task)
    }

    pub(crate) fn build_notification_list_json(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .notifications
            .toasts()
            .iter()
            .map(|t| {
                let level = match t.status() {
                    Status::Danger => "error",
                    Status::Warning => "warning",
                    Status::Success => "success",
                    Status::Primary => "info",
                    Status::Secondary => "info",
                };
                serde_json::json!({
                    "title": t.title(),
                    "body": t.body(),
                    "level": level,
                })
            })
            .collect();
        let body = serde_json::json!({ "notifications": items });
        serde_json::to_string(&body)
            .unwrap_or_else(|_| r#"{"error":"failed to serialize notifications"}"#.to_string())
    }

    pub(crate) fn build_pane_list_json(&self) -> String {
        let main_window_id = self.main_window.id;
        let Some(dashboard) = self.active_dashboard() else {
            return r#"{"panes":[],"trade_buffer_streams":[]}"#.to_string();
        };
        let trade_buffer_streams: Vec<String> = self.replay.active_stream_debug_labels();

        let panes: Vec<serde_json::Value> = dashboard
            .iter_all_panes(main_window_id)
            .map(|(window_id, _pg_pane, state)| {
                let kind = state.content.kind().to_string();
                let (ticker, timeframe) = extract_pane_ticker_timeframe(&state.streams);
                let streams_ready =
                    matches!(&state.streams, crate::connector::ResolvedStream::Ready(_));
                serde_json::json!({
                    "id": state.unique_id().to_string(),
                    "window_id": format!("{window_id:?}"),
                    "type": kind,
                    "ticker": ticker,
                    "timeframe": timeframe,
                    "link_group": state.link_group.map(|g| format!("{g:?}")),
                    "streams_ready": streams_ready,
                })
            })
            .collect();

        let body = serde_json::json!({
            "panes": panes,
            "trade_buffer_streams": trade_buffer_streams,
        });
        serde_json::to_string(&body)
            .unwrap_or_else(|_| r#"{"error":"failed to serialize pane list"}"#.to_string())
    }

    pub(crate) fn build_chart_snapshot_json(&self, pane_id: uuid::Uuid) -> String {
        use crate::screen::dashboard::pane::Content;

        let main_window_id = self.main_window.id;
        let Some((_, _, state)) = self.active_dashboard().and_then(|d| {
            d.iter_all_panes(main_window_id)
                .find(|(_, _, s)| s.unique_id() == pane_id)
        }) else {
            return format!(r#"{{"error":"pane not found: {pane_id}"}}"#);
        };

        let kind = state.content.kind().to_string();

        let (bar_count, oldest_ts, newest_ts) = match &state.content {
            Content::Kline { chart: Some(c), .. } => (
                Some(c.bar_count()),
                c.oldest_timestamp(),
                c.newest_timestamp(),
            ),
            _ => (None, None, None),
        };

        let body = serde_json::json!({
            "pane_id": pane_id.to_string(),
            "type": kind,
            "bar_count": bar_count,
            "oldest_ts": oldest_ts,
            "newest_ts": newest_ts,
        });
        serde_json::to_string(&body)
            .unwrap_or_else(|_| r#"{"error":"failed to serialize chart snapshot"}"#.to_string())
    }
}
