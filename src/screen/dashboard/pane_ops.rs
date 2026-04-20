use super::{Dashboard, Event, Message, pane};
use crate::{
    connector::{ResolvedStream, fetcher, order as order_connector},
    widget::toast::Toast,
    window::{self, Window},
};
use data::{
    layout::{WindowSpec, pane::ContentKind},
    stream::PersistStreamKind,
};
use exchange::{TickerInfo, adapter::StreamKind};
use iced::{Task, Vector, widget::pane_grid};

impl Dashboard {
    pub(super) fn handle_pane_message(
        &mut self,
        window: window::Id,
        message: pane::Message,
        main_window: &Window,
        layout_id: &uuid::Uuid,
    ) -> (Task<Message>, Option<Event>) {
        match message {
            pane::Message::PaneClicked(pane) => {
                self.focus = Some((window, pane));
            }
            pane::Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                self.panes.resize(split, ratio);
            }
            pane::Message::PaneDragged(event) => {
                if let pane_grid::DragEvent::Dropped { pane, target } = event {
                    self.panes.drop(pane, target);
                }
            }
            pane::Message::SplitPane(axis, pane) => {
                if let Some((new_pane, _)) = self.panes.split(axis, pane, pane::State::new()) {
                    self.focus = Some((window, new_pane));
                }
            }
            pane::Message::ClosePane(pane) => {
                if let Some((_, sibling)) = self.panes.close(pane) {
                    self.focus = Some((window, sibling));
                }
            }
            pane::Message::MaximizePane(pane) => {
                self.panes.maximize(pane);
            }
            pane::Message::Restore => {
                self.panes.restore();
            }
            pane::Message::ReplacePane(pane) => {
                if let Some(pane) = self.panes.get_mut(pane) {
                    *pane = pane::State::new();
                }
                return (self.refresh_streams(main_window.id), None);
            }
            pane::Message::VisualConfigChanged(pane, cfg, to_sync) => {
                self.handle_visual_config_changed(window, pane, cfg, to_sync, main_window.id);
            }
            pane::Message::SwitchLinkGroup(pane, group) => {
                return self.handle_switch_link_group(window, pane, group, main_window, layout_id);
            }
            pane::Message::Popout => return (self.popout_pane(main_window), None),
            pane::Message::Merge => return (self.merge_pane(main_window), None),
            pane::Message::PaneEvent(pane, local) => {
                return self.handle_pane_event(window, pane, local, main_window, layout_id);
            }
        }
        (Task::none(), None)
    }

    fn handle_visual_config_changed(
        &mut self,
        window: window::Id,
        pane: pane_grid::Pane,
        cfg: data::layout::pane::VisualConfig,
        to_sync: bool,
        main_window: window::Id,
    ) {
        if to_sync {
            if let Some(state) = self.get_pane(main_window, window, pane) {
                let studies_cfg = state.content.studies();
                let clusters_cfg = match &state.content {
                    pane::Content::Kline {
                        kind: data::chart::KlineChartKind::Footprint { clusters, .. },
                        ..
                    } => Some(*clusters),
                    _ => None,
                };

                self.iter_all_panes_mut(main_window)
                    .for_each(|(_, _, state)| {
                        let should_apply = Self::visual_config_should_apply(&cfg, state);

                        if should_apply {
                            state.settings.visual_config = Some(cfg.clone());
                            state.content.change_visual_config(cfg.clone());

                            if let Some(studies) = &studies_cfg {
                                state.content.update_studies(studies.clone());
                            }

                            if let Some(cluster_kind) = &clusters_cfg
                                && let pane::Content::Kline { chart, .. } = &mut state.content
                                && let Some(c) = chart
                            {
                                c.set_cluster_kind(*cluster_kind);
                            }
                        }
                    });
            }
        } else if let Some(state) = self.get_mut_pane(main_window, window, pane) {
            state.settings.visual_config = Some(cfg.clone());
            state.content.change_visual_config(cfg);
        }
    }

    fn visual_config_should_apply(
        cfg: &data::layout::pane::VisualConfig,
        state: &pane::State,
    ) -> bool {
        match state.settings.visual_config {
            Some(ref current_cfg) => {
                std::mem::discriminant(current_cfg) == std::mem::discriminant(cfg)
            }
            None => matches!(
                (cfg, &state.content),
                (
                    data::layout::pane::VisualConfig::Kline(_),
                    pane::Content::Kline { .. }
                ) | (
                    data::layout::pane::VisualConfig::Heatmap(_),
                    pane::Content::Heatmap { .. } | pane::Content::ShaderHeatmap { .. }
                ) | (
                    data::layout::pane::VisualConfig::TimeAndSales(_),
                    pane::Content::TimeAndSales(_)
                ) | (
                    data::layout::pane::VisualConfig::Comparison(_),
                    pane::Content::Comparison(_)
                )
            ),
        }
    }

    fn handle_switch_link_group(
        &mut self,
        window: window::Id,
        pane: pane_grid::Pane,
        group: Option<data::layout::pane::LinkGroup>,
        main_window: &Window,
        layout_id: &uuid::Uuid,
    ) -> (Task<Message>, Option<Event>) {
        if group.is_none() {
            if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
                state.link_group = None;
            }
            return (Task::none(), None);
        }

        let maybe_ticker_info = self
            .iter_all_panes(main_window.id)
            .filter(|(w, p, _)| !(*w == window && *p == pane))
            .find_map(|(_, _, other_state)| {
                if other_state.link_group == group {
                    other_state.stream_pair()
                } else {
                    None
                }
            });

        if let Some(state) = self.get_mut_pane(main_window.id, window, pane) {
            state.link_group = group;
            state.modal = None;

            if let Some(ticker_info) = maybe_ticker_info
                && state.stream_pair() != Some(ticker_info)
            {
                let pane_id = state.unique_id();
                let content_kind = state.content.kind();

                let streams = state.set_content_and_streams(vec![ticker_info], content_kind);
                self.streams.extend(streams.iter());

                for stream in &streams {
                    if let StreamKind::Kline { .. } = stream {
                        return (
                            fetcher::kline_fetch_task(*layout_id, pane_id, *stream, None, None)
                                .map(Message::from),
                            None,
                        );
                    }
                }
            }
        }

        (Task::none(), None)
    }

    pub(super) fn handle_pane_event(
        &mut self,
        window: window::Id,
        pane: pane_grid::Pane,
        local: pane::Event,
        main_window: &Window,
        layout_id: &uuid::Uuid,
    ) -> (Task<Message>, Option<Event>) {
        let (is_replay, eig_day) = (self.is_replay, self.eig_day_or_today());
        let Some(state) = self.get_mut_pane(main_window.id, window, pane) else {
            return (Task::none(), None);
        };
        let Some(effect) = state.update(local) else {
            return (Task::none(), None);
        };
        match effect {
            pane::Effect::RefreshStreams => (self.refresh_streams(main_window.id), None),
            pane::Effect::RequestFetch(reqs) => {
                let task = Self::handle_request_fetch(state, reqs, *layout_id);
                (task.chain(self.refresh_streams(main_window.id)), None)
            }
            pane::Effect::SwitchTickersInGroup(ticker_info) => (
                Task::none(),
                Some(Event::SwitchTickersInGroup { ticker_info }),
            ),
            pane::Effect::FocusWidget(id) => (iced::widget::operation::focus(id), None),
            pane::Effect::ReloadReplayKlines {
                old_stream,
                new_stream,
            } => (
                Task::none(),
                Some(Event::ReloadReplayKlines {
                    old_stream,
                    new_stream,
                }),
            ),
            pane::Effect::SyncIssueToOrderEntry {
                issue_code,
                issue_name,
                tick_size,
            } => (
                self.sync_issue_to_order_entry(main_window.id, issue_code, issue_name, tick_size),
                None,
            ),
            pane::Effect::SubmitVirtualOrder(vo) => {
                (Task::none(), Some(Event::SubmitVirtualOrder(vo)))
            }
            effect @ (pane::Effect::SubmitNewOrder(_)
            | pane::Effect::SubmitCorrectOrder(_)
            | pane::Effect::SubmitCancelOrder(_)
            | pane::Effect::FetchOrders
            | pane::Effect::FetchOrderDetail { .. }
            | pane::Effect::FetchBuyingPower
            | pane::Effect::FetchHoldings { .. }) => {
                let pane_id = state.unique_id();
                let task = Self::order_effect_task(effect, is_replay, pane_id, eig_day);
                (task, None)
            }
        }
    }

    fn handle_request_fetch(
        state: &mut pane::State,
        reqs: Vec<crate::connector::fetcher::FetchSpec>,
        layout_id: uuid::Uuid,
    ) -> Task<Message> {
        let pane_id = state.unique_id();
        let ready_streams = state
            .streams
            .ready_iter()
            .map(|iter| iter.copied().collect::<Vec<_>>())
            .unwrap_or_default();

        fetcher::request_fetch_many(
            pane_id,
            &ready_streams,
            layout_id,
            reqs.into_iter().map(|r| (r.req_id, r.fetch, r.stream)),
            |handle| {
                if let pane::Content::Kline { chart, .. } = &mut state.content
                    && let Some(c) = chart
                {
                    c.set_handle(handle);
                }
            },
        )
        .map(Message::from)
    }

    pub(super) fn new_pane(
        &mut self,
        axis: pane_grid::Axis,
        main_window: &Window,
        pane_state: Option<pane::State>,
    ) -> Task<Message> {
        if self
            .focus
            .filter(|(window, _)| *window == main_window.id)
            .is_some()
        {
            // If there is any focused pane on main window, split it
            return self.split_pane(axis, main_window);
        } else {
            // If there is no focused pane, split the last pane or create a new empty grid
            let pane = self.panes.iter().last().map(|(pane, _)| pane).copied();

            if let Some(pane) = pane {
                let result = self.panes.split(axis, pane, pane_state.unwrap_or_default());

                if let Some((pane, _)) = result {
                    return self.focus_pane(main_window.id, pane);
                }
            } else {
                let (state, pane) = pane_grid::State::new(pane_state.unwrap_or_default());
                self.panes = state;

                return self.focus_pane(main_window.id, pane);
            }
        }

        Task::none()
    }

    pub(super) fn focus_pane(
        &mut self,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> Task<Message> {
        if self.focus != Some((window, pane)) {
            self.focus = Some((window, pane));
        }

        Task::none()
    }

    fn split_pane(&mut self, axis: pane_grid::Axis, main_window: &Window) -> Task<Message> {
        if let Some((window, pane)) = self.focus
            && window == main_window.id
        {
            let result = self.panes.split(axis, pane, pane::State::new());

            if let Some((pane, _)) = result {
                return self.focus_pane(main_window.id, pane);
            }
        }

        Task::none()
    }

    fn popout_pane(&mut self, main_window: &Window) -> Task<Message> {
        if let Some((_, id)) = self.focus.take()
            && let Some((pane, _)) = self.panes.close(id)
        {
            let (window, task) = window::open(window::Settings {
                position: main_window
                    .position
                    .map(|point| window::Position::Specific(point + Vector::new(20.0, 20.0)))
                    .unwrap_or_default(),
                exit_on_close_request: false,
                min_size: Some(iced::Size::new(400.0, 300.0)),
                ..window::settings()
            });

            let (state, id) = pane_grid::State::new(pane);
            self.popout.insert(window, (state, WindowSpec::default()));

            return task.then(move |window| {
                Task::done(Message::Pane(window, pane::Message::PaneClicked(id)))
            });
        }

        Task::none()
    }

    fn merge_pane(&mut self, main_window: &Window) -> Task<Message> {
        if let Some((window, pane)) = self.focus.take()
            && let Some(pane_state) = self
                .popout
                .remove(&window)
                .and_then(|(mut panes, _)| panes.panes.remove(&pane))
        {
            let task = self.new_pane(pane_grid::Axis::Horizontal, main_window, Some(pane_state));

            return Task::batch(vec![window::close(window), task]);
        }

        Task::none()
    }

    pub fn all_panes_have_ready_streams(&self, main_window: window::Id) -> bool {
        self.iter_all_panes(main_window)
            .all(|(_, _, state)| match &state.streams {
                ResolvedStream::Waiting { streams, .. } => streams.is_empty(),
                ResolvedStream::Ready(_) => true,
            })
    }

    pub fn has_tachibana_stream_pane(&self, main_window: window::Id) -> bool {
        self.iter_all_panes(main_window).any(|(_, _, state)| {
            if let ResolvedStream::Waiting { streams, .. } = &state.streams {
                streams.iter().any(|s| {
                    let ticker = match s {
                        PersistStreamKind::Kline { ticker, .. } => ticker,
                        PersistStreamKind::Depth(d) => &d.ticker,
                        PersistStreamKind::Trades { ticker } => ticker,
                        PersistStreamKind::DepthAndTrades(d) => &d.ticker,
                    };
                    ticker.exchange == exchange::adapter::Exchange::Tachibana
                })
            } else {
                false
            }
        })
    }

    pub fn refresh_waiting_panes(&mut self, main_window: window::Id) {
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            if let ResolvedStream::Waiting { streams, .. } = &state.streams
                && !streams.is_empty()
            {
                state.streams.mark_resolution_due();
            }
        }
    }

    fn auto_focus_single_pane(&mut self, main_window: window::Id) {
        if self.focus.is_none()
            && self.panes.len() == 1
            && let Some((pane_id, _)) = self.panes.iter().next()
        {
            self.focus = Some((main_window, *pane_id));
        }
    }

    pub fn split_focused_and_init(
        &mut self,
        main_window: window::Id,
        ticker_info: TickerInfo,
        content_kind: ContentKind,
    ) -> Option<Task<Message>> {
        self.auto_focus_single_pane(main_window);

        let (window, focused_pane) = self.focus?;

        let (new_pane, _) = self.panes.split(
            pane_grid::Axis::Horizontal,
            focused_pane,
            pane::State::new(),
        )?;

        self.focus = Some((window, new_pane));

        let task = self.init_pane(
            main_window,
            window,
            new_pane,
            ticker_info,
            content_kind,
            false,
        );
        Some(task)
    }

    pub fn split_focused_and_init_order(
        &mut self,
        main_window: window::Id,
        content_kind: data::layout::pane::ContentKind,
    ) -> Task<Message> {
        self.auto_focus_single_pane(main_window);

        let Some((window, focused_pane)) = self.focus else {
            return Task::done(Message::Notification(Toast::warn(
                "No focused pane found".to_string(),
            )));
        };

        let Some((new_pane, _)) = self.panes.split(
            pane_grid::Axis::Horizontal,
            focused_pane,
            pane::State::new(),
        ) else {
            return Task::done(Message::Notification(Toast::warn(
                "Could not split pane".to_string(),
            )));
        };

        self.focus = Some((window, new_pane));

        if let Some(state) = self.get_mut_pane(main_window, window, new_pane) {
            state.content = pane::Content::placeholder(content_kind);
        }

        if content_kind == ContentKind::BuyingPower
            && let Some(state) = self.get_pane(main_window, window, new_pane)
        {
            let pane_id = state.unique_id();
            return Task::perform(order_connector::fetch_buying_power(), move |result| {
                Message::BuyingPowerResult { pane_id, result }
            });
        }

        Task::none()
    }

    pub fn initial_buying_power_fetch(&self, main_window: window::Id) -> Task<Message> {
        let tasks: Vec<Task<Message>> = self
            .iter_all_panes(main_window)
            .filter_map(|(_, _, state)| {
                if matches!(state.content, pane::Content::BuyingPower(_)) {
                    let pane_id = state.unique_id();
                    Some(Task::perform(
                        order_connector::fetch_buying_power(),
                        move |result| Message::BuyingPowerResult { pane_id, result },
                    ))
                } else {
                    None
                }
            })
            .collect();
        Task::batch(tasks)
    }

    pub fn initial_order_list_fetch(&self, main_window: window::Id) -> Task<Message> {
        let eig_day = self.eig_day_or_today();
        let tasks: Vec<Task<Message>> = self
            .iter_all_panes(main_window)
            .filter_map(|(_, _, state)| {
                if matches!(state.content, pane::Content::OrderList(_)) {
                    let pane_id = state.unique_id();
                    let eig_day = eig_day.clone();
                    Some(Task::perform(
                        order_connector::fetch_orders(eig_day),
                        move |result| Message::OrdersListResult { pane_id, result },
                    ))
                } else {
                    None
                }
            })
            .collect();
        Task::batch(tasks)
    }

    pub fn switch_tickers_in_group(
        &mut self,
        main_window: window::Id,
        ticker_info: TickerInfo,
        skip_kline_fetch: bool,
    ) -> Task<Message> {
        self.auto_focus_single_pane(main_window);

        let link_group = self.focus.and_then(|(window, pane)| {
            self.get_pane(main_window, window, pane)
                .and_then(|state| state.link_group)
        });

        if let Some(group) = link_group {
            let pane_infos: Vec<(window::Id, pane_grid::Pane, ContentKind)> = self
                .iter_all_panes_mut(main_window)
                .filter_map(|(window, pane, state)| {
                    if state.link_group == Some(group) {
                        Some((window, pane, state.content.kind()))
                    } else {
                        None
                    }
                })
                .collect();

            let tasks: Vec<Task<Message>> = pane_infos
                .iter()
                .map(|(window, pane, content_kind)| {
                    self.init_pane(
                        main_window,
                        *window,
                        *pane,
                        ticker_info,
                        *content_kind,
                        skip_kline_fetch,
                    )
                })
                .collect();

            Task::batch(tasks)
        } else if let Some((window, pane)) = self.focus {
            if let Some(state) = self.get_mut_pane(main_window, window, pane) {
                let content_kind = state.content.kind();
                self.init_focused_pane(main_window, ticker_info, content_kind, skip_kline_fetch)
            } else {
                Task::done(Message::Notification(Toast::warn(
                    "Couldn't get focused pane's content".to_string(),
                )))
            }
        } else {
            Task::done(Message::Notification(Toast::warn(
                "No link group or focused pane found".to_string(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::ResolvedStream;
    use iced::widget::pane_grid::Configuration;

    fn make_ticker_info() -> exchange::TickerInfo {
        exchange::TickerInfo::new(
            exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BinanceLinear),
            0.1,
            0.001,
            None,
        )
    }

    fn single_pane_dashboard() -> Dashboard {
        Dashboard::from_config(
            Configuration::Pane(pane::State::default()),
            vec![],
            uuid::Uuid::new_v4(),
        )
    }

    #[test]
    fn all_panes_have_ready_streams_true_for_default_dashboard() {
        use iced::window;
        let dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        assert!(dashboard.all_panes_have_ready_streams(main_window));
    }

    #[test]
    fn all_panes_have_ready_streams_false_when_pane_has_non_empty_waiting_streams() {
        use iced::window;
        let main_window = window::Id::unique();
        let mut dashboard = Dashboard::default();
        let pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let state = dashboard.panes.get_mut(pane).unwrap();
        state.streams = ResolvedStream::waiting(vec![PersistStreamKind::Kline {
            ticker: exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BinanceLinear),
            timeframe: exchange::Timeframe::M1,
        }]);
        assert!(!dashboard.all_panes_have_ready_streams(main_window));
    }

    #[test]
    fn refresh_waiting_panes_marks_resolution_due_for_waiting_panes() {
        use iced::window;
        use std::time::{Duration, Instant};

        let main_window = window::Id::unique();
        let mut dashboard = Dashboard::default();

        let pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let state = dashboard.panes.get_mut(pane).unwrap();
        state.streams = ResolvedStream::waiting(vec![PersistStreamKind::Kline {
            ticker: exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BinanceLinear),
            timeframe: exchange::Timeframe::M1,
        }]);
        if let ResolvedStream::Waiting { last_attempt, .. } = &mut state.streams {
            *last_attempt = Some(Instant::now() - Duration::from_millis(100));
        }

        assert!(
            state
                .streams
                .due_streams_to_resolve(Instant::now())
                .is_none()
        );

        dashboard.refresh_waiting_panes(main_window);

        let pane_state = dashboard.panes.get_mut(pane).unwrap();
        assert!(
            pane_state
                .streams
                .due_streams_to_resolve(Instant::now())
                .is_some()
        );
    }

    #[test]
    fn has_tachibana_stream_pane_returns_true_for_tachibana_ticker() {
        use exchange::adapter::Exchange;
        use iced::window;

        let main_window = window::Id::unique();
        let mut dashboard = Dashboard::default();

        let pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let state = dashboard.panes.get_mut(pane).unwrap();
        state.streams = ResolvedStream::waiting(vec![PersistStreamKind::Kline {
            ticker: exchange::Ticker::new("7203", Exchange::Tachibana),
            timeframe: exchange::Timeframe::D1,
        }]);

        assert!(dashboard.has_tachibana_stream_pane(main_window));
    }

    #[test]
    fn has_tachibana_stream_pane_returns_false_for_binance_ticker() {
        use iced::window;
        let main_window = window::Id::unique();
        let dashboard = Dashboard::default();
        assert!(!dashboard.has_tachibana_stream_pane(main_window));
    }

    #[test]
    fn split_focused_and_init_returns_none_when_no_focus_and_multiple_panes() {
        use iced::window;
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        let pane_count_before = dashboard.panes.len();
        assert!(pane_count_before > 1);
        assert!(dashboard.focus.is_none());

        let result = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );

        assert!(result.is_none());
        assert_eq!(
            dashboard.panes.len(),
            pane_count_before,
            "pane count must not change"
        );
    }

    #[test]
    fn split_focused_and_init_auto_focuses_and_splits_when_single_pane_no_focus() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        assert_eq!(dashboard.panes.len(), 1);
        assert!(dashboard.focus.is_none());

        let result = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );

        assert!(result.is_some());
        assert_eq!(dashboard.panes.len(), 2);
        assert!(dashboard.focus.is_some());
    }

    #[test]
    fn split_focused_and_init_splits_focused_pane_and_moves_focus() {
        use iced::window;
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        let first_pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let pane_count_before = dashboard.panes.len();
        dashboard.focus = Some((main_window, first_pane));

        let result = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );

        assert!(result.is_some());
        assert_eq!(dashboard.panes.len(), pane_count_before + 1);
        let (_, focused) = dashboard.focus.unwrap();
        assert_ne!(focused, first_pane, "focus should move to the new pane");
    }

    #[test]
    fn split_focused_and_init_preserves_original_pane_count_when_focus_window_differs() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let popout_window = window::Id::unique();
        let pane_id = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        dashboard.focus = Some((popout_window, pane_id));
        let pane_count_before = dashboard.panes.len();

        let result = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );

        assert!(result.is_some());
        assert_eq!(
            dashboard.panes.len(),
            pane_count_before + 1,
            "pane count must increase by 1 even when focus is on a popout window"
        );
    }

    #[test]
    fn split_focused_and_init_focus_window_matches_original_focus_window() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let popout_window = window::Id::unique();
        let pane_id = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        dashboard.focus = Some((popout_window, pane_id));

        let result = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );

        assert!(result.is_some());
        let (focus_window, _) = dashboard.focus.unwrap();
        assert_eq!(
            focus_window, popout_window,
            "focus window must remain the same after split"
        );
        assert_ne!(focus_window, main_window);
    }

    #[test]
    fn split_focused_and_init_returns_some_twice_in_succession() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        assert_eq!(dashboard.panes.len(), 1);
        assert!(dashboard.focus.is_none());

        let result1 = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );
        assert!(result1.is_some(), "1 回目は Some を返すこと");
        assert_eq!(dashboard.panes.len(), 2, "1 回目後 pane count = 2");
        assert!(
            dashboard.focus.is_some(),
            "1 回目後 focus が設定されていること"
        );

        let result2 = dashboard.split_focused_and_init(
            main_window,
            make_ticker_info(),
            data::layout::pane::ContentKind::CandlestickChart,
        );
        assert!(result2.is_some(), "2 回目も Some を返すこと");
        assert_eq!(dashboard.panes.len(), 3, "2 回目後 pane count = 3");
    }

    #[test]
    fn split_focused_and_init_order_splits_single_pane_no_focus() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        assert_eq!(dashboard.panes.len(), 1);
        assert!(dashboard.focus.is_none());

        let _task = dashboard
            .split_focused_and_init_order(main_window, data::layout::pane::ContentKind::OrderEntry);

        assert_eq!(dashboard.panes.len(), 2, "pane count must increase by 1");
        assert!(dashboard.focus.is_some(), "focus must be set after split");
    }

    #[test]
    fn split_focused_and_init_order_no_split_when_no_focus_multiple_panes() {
        use iced::window;
        let mut dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        assert!(dashboard.panes.len() > 1);
        assert!(dashboard.focus.is_none());
        let pane_count_before = dashboard.panes.len();

        let _task = dashboard
            .split_focused_and_init_order(main_window, data::layout::pane::ContentKind::OrderEntry);

        assert_eq!(
            dashboard.panes.len(),
            pane_count_before,
            "pane count must not change when there is no focus and multiple panes"
        );
    }

    #[test]
    fn split_focused_and_init_order_sets_order_content_on_new_pane() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();

        let _task = dashboard
            .split_focused_and_init_order(main_window, data::layout::pane::ContentKind::OrderEntry);

        assert_eq!(dashboard.panes.len(), 2);
        let (_, focused_pane) = dashboard.focus.unwrap();
        let state = dashboard.panes.get(focused_pane).unwrap();
        assert!(
            matches!(state.content, pane::Content::OrderEntry(_)),
            "new pane content must be OrderEntry"
        );
    }

    #[test]
    fn split_focused_and_init_order_moves_focus_to_new_pane() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let original_pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();

        let _task = dashboard.split_focused_and_init_order(
            main_window,
            data::layout::pane::ContentKind::BuyingPower,
        );

        let (_, focused_pane) = dashboard.focus.unwrap();
        assert_ne!(
            focused_pane, original_pane,
            "focus must move to the new pane"
        );
    }

    #[test]
    fn initial_order_list_fetch_does_not_crash_with_no_order_list_panes() {
        use iced::window;
        let dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        let _task = dashboard.initial_order_list_fetch(main_window);
        assert!(!dashboard.panes.is_empty());
    }

    #[test]
    fn initial_order_list_fetch_finds_order_list_pane_and_does_not_crash() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let _task = dashboard
            .split_focused_and_init_order(main_window, data::layout::pane::ContentKind::OrderList);

        let order_list_count = dashboard
            .iter_all_panes(main_window)
            .filter(|(_, _, s)| matches!(s.content, pane::Content::OrderList(_)))
            .count();
        assert_eq!(
            order_list_count, 1,
            "setup: exactly 1 OrderList pane must exist"
        );

        let pane_count_before = dashboard.panes.len();
        let _task = dashboard.initial_order_list_fetch(main_window);
        assert_eq!(
            dashboard.panes.len(),
            pane_count_before,
            "initial_order_list_fetch must not mutate pane count"
        );
    }

    #[test]
    fn initial_order_list_fetch_does_not_process_buying_power_panes() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let _task = dashboard.split_focused_and_init_order(
            main_window,
            data::layout::pane::ContentKind::BuyingPower,
        );

        let order_list_count = dashboard
            .iter_all_panes(main_window)
            .filter(|(_, _, s)| matches!(s.content, pane::Content::OrderList(_)))
            .count();
        assert_eq!(
            order_list_count, 0,
            "BuyingPower pane must not be counted as OrderList"
        );

        let _task = dashboard.initial_order_list_fetch(main_window);
        assert_eq!(dashboard.panes.len(), 2);
    }

    #[test]
    fn initial_buying_power_fetch_does_not_crash_with_no_buying_power_panes() {
        use iced::window;
        let dashboard = Dashboard::default();
        let main_window = window::Id::unique();
        let _task = dashboard.initial_buying_power_fetch(main_window);
        assert!(!dashboard.panes.is_empty());
    }

    #[test]
    fn initial_buying_power_fetch_finds_buying_power_pane_and_does_not_crash() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();
        let _task = dashboard.split_focused_and_init_order(
            main_window,
            data::layout::pane::ContentKind::BuyingPower,
        );

        let bp_count = dashboard
            .iter_all_panes(main_window)
            .filter(|(_, _, s)| matches!(s.content, pane::Content::BuyingPower(_)))
            .count();
        assert_eq!(bp_count, 1, "setup: exactly 1 BuyingPower pane must exist");

        let pane_count_before = dashboard.panes.len();
        let _task = dashboard.initial_buying_power_fetch(main_window);
        assert_eq!(
            dashboard.panes.len(),
            pane_count_before,
            "initial_buying_power_fetch must not mutate pane count"
        );
    }

    #[test]
    fn split_focused_and_init_order_sets_order_list_content_on_new_pane() {
        use iced::window;
        let mut dashboard = single_pane_dashboard();
        let main_window = window::Id::unique();

        let _task = dashboard
            .split_focused_and_init_order(main_window, data::layout::pane::ContentKind::OrderList);

        assert_eq!(dashboard.panes.len(), 2);
        let (_, focused_pane) = dashboard.focus.unwrap();
        let state = dashboard.panes.get(focused_pane).unwrap();
        assert!(
            matches!(state.content, pane::Content::OrderList(_)),
            "new pane content must be OrderList"
        );
    }
}
