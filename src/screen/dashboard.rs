mod effect;
mod order_handler;
pub mod pane;
mod pane_ops;
pub mod panel;
mod replay;
pub mod sidebar;
mod subscription;
pub mod tickers_table;

pub use sidebar::Sidebar;

use super::DashboardError;
use crate::{
    chart,
    connector::{
        ResolvedStream,
        fetcher::{self, FetchedData, InfoKind},
    },
    screen::dashboard::tickers_table::TickersTable,
    style,
    widget::toast::Toast,
    window::{self, Window},
};
use data::{
    UserTimezone,
    layout::{WindowSpec, pane::ContentKind},
    stream::PersistStreamKind,
};
use exchange::{
    Kline, StreamPairKind, TickerInfo, Trade,
    adapter::{StreamKind, UniqueStreams},
    depth::Depth,
};

use iced::{
    Element, Length, Subscription, Task, Theme,
    widget::{
        PaneGrid, center, container,
        pane_grid::{self, Configuration},
    },
};
use std::{collections::HashMap, time::Instant, vec};

#[derive(Debug, Clone)]
pub enum Message {
    Pane(window::Id, pane::Message),
    ChangePaneStatus(uuid::Uuid, pane::Status),
    SavePopoutSpecs(HashMap<window::Id, WindowSpec>),
    ErrorOccurred(Option<uuid::Uuid>, DashboardError),
    Notification(Toast),
    DistributeFetchedData {
        layout_id: uuid::Uuid,
        pane_id: uuid::Uuid,
        stream: StreamKind,
        data: FetchedData,
    },
    ResolveStreams(uuid::Uuid, Vec<PersistStreamKind>),
    RequestPalette,
    // ── 注文 API 応答メッセージ ───────────────────────────────────────────────
    OrderNewResult {
        pane_id: uuid::Uuid,
        result: Result<exchange::adapter::tachibana::NewOrderResponse, String>,
    },
    OrderModifyResult {
        pane_id: uuid::Uuid,
        /// 訂正・取消成功時の注文番号
        result: Result<String, String>,
    },
    OrdersListResult {
        pane_id: uuid::Uuid,
        result: Result<Vec<exchange::adapter::tachibana::OrderRecord>, String>,
    },
    OrderDetailResult {
        pane_id: uuid::Uuid,
        order_num: String,
        result: Result<Vec<exchange::adapter::tachibana::ExecutionRecord>, String>,
    },
    BuyingPowerResult {
        pane_id: uuid::Uuid,
        result: Result<
            (
                exchange::adapter::tachibana::BuyingPowerResponse,
                exchange::adapter::tachibana::MarginPowerResponse,
            ),
            String,
        >,
    },
    HoldingsResult {
        pane_id: uuid::Uuid,
        result: Result<u64, String>,
    },
    /// 仮想約定通知（REPLAYモード）。main.rs の on_tick() から届く。
    VirtualOrderFilled(crate::replay::virtual_exchange::FillEvent),
}

pub struct Dashboard {
    pub panes: pane_grid::State<pane::State>,
    pub focus: Option<(window::Id, pane_grid::Pane)>,
    pub popout: HashMap<window::Id, (pane_grid::State<pane::State>, WindowSpec)>,
    pub streams: UniqueStreams,
    layout_id: uuid::Uuid,
    /// 本日営業日 (YYYYMMDD)。初回注文成功時に NewOrderResponse.eig_day から取得する。
    eig_day: Option<String>,
    /// REPLAYモード中かどうか。main.rs から replay 状態切替時にシンクされる。
    /// update() 内で Effect ハンドラが参照できるよう、view() 引数とは別に保持する。
    pub is_replay: bool,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            panes: pane_grid::State::with_configuration(Self::default_pane_config()),
            focus: None,
            streams: UniqueStreams::default(),
            popout: HashMap::new(),
            layout_id: uuid::Uuid::new_v4(),
            eig_day: None,
            is_replay: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Notification(Toast),
    DistributeFetchedData {
        layout_id: uuid::Uuid,
        pane_id: uuid::Uuid,
        data: FetchedData,
        stream: StreamKind,
    },
    ResolveStreams {
        pane_id: uuid::Uuid,
        streams: Vec<PersistStreamKind>,
    },
    RequestPalette,
    /// リプレイ中に kline stream の basis が変わったとき、コントローラに再ロードを依頼する。
    ReloadReplayKlines {
        old_stream: Option<StreamKind>,
        new_stream: StreamKind,
    },
    /// MiniTickersList ペインから銘柄を切り替えたとき、main.rs でリプレイ同期を行うために伝搬する。
    SwitchTickersInGroup {
        ticker_info: TickerInfo,
    },
    /// REPLAYモードで仮想注文が送信されたとき、main.rs の VirtualExchangeEngine に渡す。
    SubmitVirtualOrder(crate::replay::virtual_exchange::VirtualOrder),
}

impl Dashboard {
    fn default_pane_config() -> Configuration<pane::State> {
        Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.8,
            a: Box::new(Configuration::Split {
                axis: pane_grid::Axis::Horizontal,
                ratio: 0.4,
                a: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(pane::State::default())),
                    b: Box::new(Configuration::Pane(pane::State::default())),
                }),
                b: Box::new(Configuration::Split {
                    axis: pane_grid::Axis::Vertical,
                    ratio: 0.5,
                    a: Box::new(Configuration::Pane(pane::State::default())),
                    b: Box::new(Configuration::Pane(pane::State::default())),
                }),
            }),
            b: Box::new(Configuration::Pane(pane::State::default())),
        }
    }

    pub fn from_config(
        panes: Configuration<pane::State>,
        popout_windows: Vec<(Configuration<pane::State>, WindowSpec)>,
        layout_id: uuid::Uuid,
    ) -> Self {
        let panes = pane_grid::State::with_configuration(panes);

        let mut popout = HashMap::new();

        for (pane, specs) in popout_windows {
            popout.insert(
                window::Id::unique(),
                (pane_grid::State::with_configuration(pane), specs),
            );
        }

        Self {
            panes,
            focus: None,
            streams: UniqueStreams::default(),
            popout,
            layout_id,
            eig_day: None,
            is_replay: false,
        }
    }

    pub fn load_layout(&mut self, main_window: window::Id) -> Task<Message> {
        let mut open_popouts_tasks: Vec<Task<Message>> = vec![];
        let mut new_popout = Vec::new();
        let mut keys_to_remove = Vec::new();

        for (old_window_id, (_, specs)) in &self.popout {
            keys_to_remove.push((*old_window_id, *specs));
        }

        // remove keys and open new windows
        for (old_window_id, window_spec) in keys_to_remove {
            let (window, task) = window::open(window::Settings {
                position: window::Position::Specific(window_spec.position()),
                size: window_spec.size(),
                exit_on_close_request: false,
                ..window::settings()
            });

            open_popouts_tasks.push(task.then(|_| Task::none()));

            if let Some((removed_pane, specs)) = self.popout.remove(&old_window_id) {
                new_popout.push((window, (removed_pane, specs)));
            }
        }

        // assign new windows to old panes
        for (window, (pane, specs)) in new_popout {
            self.popout.insert(window, (pane, specs));
        }

        Task::batch(open_popouts_tasks).chain(self.refresh_streams(main_window))
    }

    pub fn update(
        &mut self,
        message: Message,
        main_window: &Window,
        layout_id: &uuid::Uuid,
    ) -> (Task<Message>, Option<Event>) {
        match message {
            Message::SavePopoutSpecs(specs) => {
                for (window_id, new_spec) in specs {
                    if let Some((_, spec)) = self.popout.get_mut(&window_id) {
                        *spec = new_spec;
                    }
                }
            }
            Message::ErrorOccurred(pane_id, err) => match pane_id {
                Some(id) => {
                    if let Some(state) = self.get_mut_pane_state_by_uuid(main_window.id, id) {
                        state.status = pane::Status::Ready;
                        state.notifications.push(Toast::error(err.to_string()));
                    }
                }
                _ => {
                    return (
                        Task::done(Message::Notification(Toast::error(err.to_string()))),
                        None,
                    );
                }
            },
            Message::Pane(window, msg) => {
                return self.handle_pane_message(window, msg, main_window, layout_id);
            }
            Message::RequestPalette => return (Task::none(), Some(Event::RequestPalette)),
            Message::ChangePaneStatus(pane_id, status) => {
                if let Some(state) = self.get_mut_pane_state_by_uuid(main_window.id, pane_id) {
                    state.status = status;
                }
            }
            Message::DistributeFetchedData {
                layout_id,
                pane_id,
                data,
                stream,
            } => {
                return (
                    Task::none(),
                    Some(Event::DistributeFetchedData {
                        layout_id,
                        pane_id,
                        data,
                        stream,
                    }),
                );
            }
            Message::ResolveStreams(pane_id, streams) => {
                return (
                    Task::none(),
                    Some(Event::ResolveStreams { pane_id, streams }),
                );
            }
            Message::Notification(toast) => {
                return (Task::none(), Some(Event::Notification(toast)));
            }
            Message::OrderNewResult { pane_id, result } => {
                self.handle_order_new_result(pane_id, result, main_window.id);
            }
            Message::OrderModifyResult { pane_id, result } => {
                self.handle_order_modify_result(pane_id, result, main_window.id);
            }
            Message::OrdersListResult { pane_id, result } => {
                self.handle_orders_list_result(pane_id, result, main_window.id);
            }
            Message::OrderDetailResult {
                pane_id,
                order_num,
                result,
            } => {
                self.handle_order_detail_result(pane_id, order_num, result, main_window.id);
            }
            Message::BuyingPowerResult { pane_id, result } => {
                self.handle_buying_power_result(pane_id, result, main_window.id);
            }
            Message::HoldingsResult { pane_id, result } => {
                self.handle_holdings_result(pane_id, result, main_window.id);
            }
            Message::VirtualOrderFilled(fill) => {
                return self.handle_virtual_order_filled(fill);
            }
        }

        (Task::none(), None)
    }

    pub fn get_pane(
        &self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> Option<&pane::State> {
        if main_window == window {
            self.panes.get(pane)
        } else {
            self.popout
                .get(&window)
                .and_then(|(panes, _)| panes.get(pane))
        }
    }

    fn get_mut_pane(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        pane: pane_grid::Pane,
    ) -> Option<&mut pane::State> {
        if main_window == window {
            self.panes.get_mut(pane)
        } else {
            self.popout
                .get_mut(&window)
                .and_then(|(panes, _)| panes.get_mut(pane))
        }
    }

    fn get_mut_pane_state_by_uuid(
        &mut self,
        main_window: window::Id,
        uuid: uuid::Uuid,
    ) -> Option<&mut pane::State> {
        self.iter_all_panes_mut(main_window)
            .find(|(_, _, state)| state.unique_id() == uuid)
            .map(|(_, _, state)| state)
    }

    pub fn iter_all_panes(
        &self,
        main_window: window::Id,
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &pane::State)> {
        self.panes
            .iter()
            .map(move |(pane, state)| (main_window, *pane, state))
            .chain(self.popout.iter().flat_map(|(window_id, (panes, _))| {
                panes.iter().map(|(pane, state)| (*window_id, *pane, state))
            }))
    }

    pub fn iter_all_panes_mut(
        &mut self,
        main_window: window::Id,
    ) -> impl Iterator<Item = (window::Id, pane_grid::Pane, &mut pane::State)> {
        self.panes
            .iter_mut()
            .map(move |(pane, state)| (main_window, *pane, state))
            .chain(self.popout.iter_mut().flat_map(|(window_id, (panes, _))| {
                panes
                    .iter_mut()
                    .map(|(pane, state)| (*window_id, *pane, state))
            }))
    }

    pub fn view<'a>(
        &'a self,
        main_window: &'a Window,
        tickers_table: &'a TickersTable,
        timezone: UserTimezone,
        is_replay: bool,
        theme: &'a Theme,
    ) -> Element<'a, Message> {
        let mut pane_grid = PaneGrid::new(&self.panes, |id, pane, maximized| {
            let is_focused = self.focus == Some((main_window.id, id));
            pane.view(
                id,
                self.panes.len(),
                is_focused,
                maximized,
                main_window.id,
                main_window,
                timezone,
                tickers_table,
                is_replay,
                theme,
            )
        })
        .min_size(240)
        .on_click(pane::Message::PaneClicked)
        .spacing(6)
        .style(style::pane_grid);

        if !is_replay {
            pane_grid = pane_grid
                .on_drag(pane::Message::PaneDragged)
                .on_resize(8, pane::Message::PaneResized);
        }

        let pane_grid: Element<_> = pane_grid.into();

        pane_grid.map(move |message| Message::Pane(main_window.id, message))
    }

    pub fn view_window<'a>(
        &'a self,
        window: window::Id,
        main_window: &'a Window,
        tickers_table: &'a TickersTable,
        timezone: UserTimezone,
        theme: &'a Theme,
    ) -> Element<'a, Message> {
        if let Some((state, _)) = self.popout.get(&window) {
            let content = container(
                PaneGrid::new(state, |id, pane, _maximized| {
                    let is_focused = self.focus == Some((window, id));
                    pane.view(
                        id,
                        state.len(),
                        is_focused,
                        false,
                        window,
                        main_window,
                        timezone,
                        tickers_table,
                        false, // popout windows don't support replay
                        theme,
                    )
                })
                .on_click(pane::Message::PaneClicked),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(8);

            Element::new(content).map(move |message| Message::Pane(window, message))
        } else {
            Element::new(center("No pane found for window"))
                .map(move |message| Message::Pane(window, message))
        }
    }

    pub fn go_back(&mut self, main_window: window::Id) -> bool {
        let Some((window, pane)) = self.focus else {
            return false;
        };

        let Some(state) = self.get_mut_pane(main_window, window, pane) else {
            return false;
        };

        if state.modal.is_some() {
            state.modal = None;
            return true;
        }
        false
    }

    fn handle_error(
        &mut self,
        pane_id: Option<uuid::Uuid>,
        err: &DashboardError,
        main_window: window::Id,
    ) -> Task<Message> {
        match pane_id {
            Some(id) => {
                if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, id) {
                    state.status = pane::Status::Ready;
                    state.notifications.push(Toast::error(err.to_string()));
                }
                Task::none()
            }
            _ => Task::done(Message::Notification(Toast::error(err.to_string()))),
        }
    }

    fn init_pane(
        &mut self,
        main_window: window::Id,
        window: window::Id,
        selected_pane: pane_grid::Pane,
        ticker_info: TickerInfo,
        content_kind: ContentKind,
        skip_kline_fetch: bool,
    ) -> Task<Message> {
        if let Some(state) = self.get_mut_pane(main_window, window, selected_pane) {
            let pane_id = state.unique_id();
            let previous_ticker = state.stream_pair();

            let streams = state.set_content_and_streams(vec![ticker_info], content_kind);
            self.streams.extend(streams.iter());

            // Sync ticker change to OrderEntry panes if ticker actually changed
            let ticker_changed = previous_ticker != Some(ticker_info);
            if ticker_changed {
                let issue_code = ticker_info.ticker.symbol_and_exchange_string();
                let tick_size: f64 = f32::from(ticker_info.min_ticksize) as f64;
                let sync_task = self.sync_issue_to_order_entry(
                    main_window,
                    issue_code.clone(),
                    issue_code, // Use ticker symbol as both code and name
                    Some(tick_size),
                );
                if !skip_kline_fetch {
                    for stream in &streams {
                        if let StreamKind::Kline { .. } = stream {
                            return Task::batch(vec![
                                fetcher::kline_fetch_task(
                                    self.layout_id,
                                    pane_id,
                                    *stream,
                                    None,
                                    None,
                                )
                                .map(Message::from),
                                sync_task,
                            ]);
                        }
                    }
                }
                return sync_task;
            }

            if !skip_kline_fetch {
                for stream in &streams {
                    if let StreamKind::Kline { .. } = stream {
                        return fetcher::kline_fetch_task(
                            self.layout_id,
                            pane_id,
                            *stream,
                            None,
                            None,
                        )
                        .map(Message::from);
                    }
                }
            }
        }

        Task::none()
    }

    pub fn init_focused_pane(
        &mut self,
        main_window: window::Id,
        ticker_info: TickerInfo,
        content_kind: ContentKind,
        skip_kline_fetch: bool,
    ) -> Task<Message> {
        if self.focus.is_none()
            && self.panes.len() == 1
            && let Some((pane_id, _)) = self.panes.iter().next()
        {
            self.focus = Some((main_window, *pane_id));
        }

        if let Some((window, selected_pane)) = self.focus
            && let Some(state) = self.get_mut_pane(main_window, window, selected_pane)
        {
            let previous_ticker = state.stream_pair();
            let ticker_changed = previous_ticker.is_some() && previous_ticker != Some(ticker_info);
            if ticker_changed {
                state.link_group = None;
            }

            let streams = state.set_content_and_streams(vec![ticker_info], content_kind);

            let pane_id = state.unique_id();
            self.streams.extend(streams.iter());

            // Sync ticker change to OrderEntry panes if ticker actually changed
            if ticker_changed {
                let issue_code = ticker_info.ticker.symbol_and_exchange_string();
                let tick_size: f64 = f32::from(ticker_info.min_ticksize) as f64;
                let sync_task = self.sync_issue_to_order_entry(
                    main_window,
                    issue_code.clone(),
                    issue_code, // Use ticker symbol as both code and name
                    Some(tick_size),
                );
                if !skip_kline_fetch {
                    for stream in &streams {
                        if let StreamKind::Kline { .. } = stream {
                            return Task::batch(vec![
                                fetcher::kline_fetch_task(
                                    self.layout_id,
                                    pane_id,
                                    *stream,
                                    None,
                                    None,
                                )
                                .map(Message::from),
                                sync_task,
                            ]);
                        }
                    }
                }
                return sync_task;
            }

            if !skip_kline_fetch {
                for stream in &streams {
                    if let StreamKind::Kline { .. } = stream {
                        return fetcher::kline_fetch_task(
                            self.layout_id,
                            pane_id,
                            *stream,
                            None,
                            None,
                        )
                        .map(Message::from);
                    }
                }
            }
            return Task::none();
        }

        Task::done(Message::Notification(Toast::warn(
            "No focused pane found".to_string(),
        )))
    }

    pub fn toggle_trade_fetch(&mut self, is_enabled: bool, main_window: &Window) {
        fetcher::toggle_trade_fetch(is_enabled);

        self.iter_all_panes_mut(main_window.id)
            .for_each(|(_, _, state)| {
                if let pane::Content::Kline { chart, kind, .. } = &mut state.content
                    && matches!(kind, data::chart::KlineChartKind::Footprint { .. })
                    && let Some(c) = chart
                {
                    c.reset_request_handler();

                    if !is_enabled {
                        state.status = pane::Status::Ready;
                    }
                }
            });
    }

    pub fn distribute_fetched_data(
        &mut self,
        main_window: window::Id,
        pane_id: uuid::Uuid,
        data: FetchedData,
        stream_type: StreamKind,
    ) -> Task<Message> {
        match data {
            FetchedData::Trades { batch, until_time } => {
                let last_trade_time = batch.last().map_or(0, |trade| trade.time);

                if last_trade_time < until_time {
                    if let Err(reason) =
                        self.insert_fetched_trades(main_window, pane_id, &batch, false)
                    {
                        return self.handle_error(Some(pane_id), &reason, main_window);
                    }
                } else {
                    let filtered_batch = batch
                        .iter()
                        .filter(|trade| trade.time <= until_time)
                        .copied()
                        .collect::<Vec<_>>();

                    if let Err(reason) =
                        self.insert_fetched_trades(main_window, pane_id, &filtered_batch, true)
                    {
                        return self.handle_error(Some(pane_id), &reason, main_window);
                    }
                }
            }
            FetchedData::Klines { data, req_id } => {
                if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window, pane_id) {
                    pane_state.status = pane::Status::Ready;

                    if let StreamKind::Kline {
                        timeframe,
                        ticker_info,
                    } = stream_type
                    {
                        pane_state.insert_hist_klines(req_id, timeframe, ticker_info, &data);
                    }
                }
            }
            FetchedData::OI { data, req_id } => {
                if let Some(pane_state) = self.get_mut_pane_state_by_uuid(main_window, pane_id) {
                    pane_state.status = pane::Status::Ready;

                    if let StreamKind::Kline { .. } = stream_type {
                        pane_state.insert_hist_oi(req_id, &data);
                    }
                }
            }
        }

        Task::none()
    }

    fn insert_fetched_trades(
        &mut self,
        main_window: window::Id,
        pane_id: uuid::Uuid,
        trades: &[Trade],
        is_batches_done: bool,
    ) -> Result<(), DashboardError> {
        let pane_state = self
            .get_mut_pane_state_by_uuid(main_window, pane_id)
            .ok_or_else(|| {
                DashboardError::Unknown(
                    "No matching pane state found for fetched trades".to_string(),
                )
            })?;

        match &mut pane_state.status {
            pane::Status::Loading(InfoKind::FetchingTrades(count)) => {
                *count += trades.len();
            }
            _ => {
                pane_state.status = pane::Status::Loading(InfoKind::FetchingTrades(trades.len()));
            }
        }

        match &mut pane_state.content {
            pane::Content::Kline { chart, .. } => {
                if let Some(c) = chart {
                    c.insert_raw_trades(trades.to_owned(), is_batches_done);

                    if is_batches_done {
                        pane_state.status = pane::Status::Ready;
                    }
                    Ok(())
                } else {
                    Err(DashboardError::Unknown(
                        "fetched trades but no chart found".to_string(),
                    ))
                }
            }
            _ => Err(DashboardError::Unknown(
                "No matching chart found for fetched trades".to_string(),
            )),
        }
    }

    pub fn update_latest_klines(
        &mut self,
        stream: &StreamKind,
        kline: &Kline,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                if pane_state.matches_stream(stream) {
                    match &mut pane_state.content {
                        pane::Content::Kline { chart: Some(c), .. } => {
                            c.update_latest_kline(kline);
                        }
                        pane::Content::Comparison(Some(c)) => {
                            c.update_latest_kline(&stream.ticker_info(), kline);
                        }
                        _ => {}
                    }
                    found_match = true;
                }
            });

        if found_match {
            Task::none()
        } else {
            log::debug!("{stream:?} stream had no matching panes - dropping");
            self.refresh_streams(main_window)
        }
    }

    pub fn ingest_depth(
        &mut self,
        stream: &StreamKind,
        depth_update_t: u64,
        depth: &Depth,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut found_match = false;

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                if pane_state.matches_stream(stream) {
                    match &mut pane_state.content {
                        pane::Content::Heatmap { chart, .. } => {
                            if let Some(c) = chart {
                                c.insert_depth(depth, depth_update_t);
                            }
                        }
                        pane::Content::ShaderHeatmap { chart, .. } => {
                            if let Some(c) = chart {
                                c.insert_depth(depth, depth_update_t);
                            }
                        }
                        pane::Content::Ladder(panel) => {
                            if let Some(panel) = panel {
                                panel.insert_depth(depth, depth_update_t);
                            }
                        }
                        _ => {
                            log::error!("No chart found for the stream: {stream:?}");
                        }
                    }
                    found_match = true;
                }
            });

        if found_match {
            Task::none()
        } else {
            self.refresh_streams(main_window)
        }
    }

    pub fn ingest_trades(
        &mut self,
        stream: &StreamKind,
        buffer: &[Trade],
        update_t: u64,
        main_window: window::Id,
    ) -> Task<Message> {
        let mut found_match = false;
        let trade_ticker = stream.ticker_info();

        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, pane_state)| {
                // 完全一致 または 同一 ticker_info を持つペインにマッチ
                let matched = pane_state.matches_stream(stream)
                    || (pane_state.stream_pair() == Some(trade_ticker));
                if matched {
                    match &mut pane_state.content {
                        pane::Content::Heatmap { chart, .. } => {
                            if let Some(c) = chart {
                                c.insert_trades(buffer, update_t);
                            }
                        }
                        pane::Content::ShaderHeatmap { chart, .. } => {
                            if let Some(c) = chart {
                                c.insert_trades(buffer, update_t);
                            }
                        }
                        pane::Content::Kline { chart, .. } => {
                            if let Some(c) = chart {
                                c.insert_trades(buffer);
                            }
                        }
                        pane::Content::TimeAndSales(panel) => {
                            if let Some(p) = panel {
                                p.insert_buffer(buffer);
                            }
                        }
                        pane::Content::Ladder(panel) => {
                            if let Some(p) = panel {
                                p.insert_trades(buffer);
                            }
                        }
                        _ => {
                            log::error!("No chart found for the stream: {stream:?}");
                        }
                    }
                    found_match = true;
                }
            });

        if found_match {
            Task::none()
        } else {
            self.refresh_streams(main_window)
        }
    }

    pub fn invalidate_all_panes(&mut self, main_window: window::Id) {
        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, state)| {
                let _ = state.invalidate(Instant::now());
            });
    }

    pub fn park_for_inactive_layout(&mut self, main_window: window::Id) {
        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, state)| state.park_for_inactive_layout());
    }

    pub fn tick(&mut self, now: Instant, _main_window: window::Id) -> Task<Message> {
        let mut tasks = vec![];

        let mut tick_state = |state: &mut pane::State| match state.tick(now) {
            Some(pane::Action::Chart(action)) => match action {
                chart::Action::ErrorOccurred(err) => {
                    state.status = pane::Status::Ready;
                    state.notifications.push(Toast::error(err.to_string()));
                }
                chart::Action::RequestFetch(reqs) => {
                    let pane_id = state.unique_id();
                    let ready_streams = state
                        .streams
                        .ready_iter()
                        .map(|iter| iter.copied().collect::<Vec<_>>())
                        .unwrap_or_default();

                    let fetch_tasks = fetcher::request_fetch_many(
                        pane_id,
                        &ready_streams,
                        self.layout_id,
                        reqs.into_iter().map(|r| (r.req_id, r.fetch, r.stream)),
                        |handle| {
                            if let pane::Content::Kline { chart, .. } = &mut state.content
                                && let Some(c) = chart
                            {
                                c.set_handle(handle);
                            }
                        },
                    )
                    .map(Message::from);

                    tasks.push(fetch_tasks);
                }
                chart::Action::RequestPalette => {
                    tasks.push(Task::done(Message::RequestPalette));
                }
            },
            Some(pane::Action::Panel(_action)) => {}
            Some(pane::Action::ResolveStreams(streams)) => {
                tasks.push(Task::done(Message::ResolveStreams(
                    state.unique_id(),
                    streams,
                )));
            }
            Some(pane::Action::ResolveContent) => match state.stream_pair_kind() {
                Some(StreamPairKind::MultiSource(tickers)) => {
                    state.set_content_and_streams(tickers, state.content.kind());
                }
                Some(StreamPairKind::SingleSource(ticker)) => {
                    state.set_content_and_streams(vec![ticker], state.content.kind());
                }
                None => {}
            },
            None => {}
        };

        // tick only the maximized pane if there is any, otherwise tick all panes
        let maximized_pane = self.panes.maximized();
        for (pane_id, state) in self.panes.iter_mut() {
            if maximized_pane.is_some_and(|maximized| *pane_id != maximized) {
                continue;
            }

            tick_state(state);
        }

        for (popout_state, _) in self.popout.values_mut() {
            for (_, state) in popout_state.iter_mut() {
                tick_state(state);
            }
        }

        Task::batch(tasks)
    }

    pub fn resolve_streams(
        &mut self,
        main_window: window::Id,
        pane_id: uuid::Uuid,
        streams: Vec<StreamKind>,
    ) -> Task<Message> {
        if let Some(state) = self.get_mut_pane_state_by_uuid(main_window, pane_id) {
            state.streams = ResolvedStream::Ready(streams.clone());
        }
        self.refresh_streams(main_window)
    }

    pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
        let subs = self
            .streams
            .combined_used()
            .flat_map(|(exchange, specs)| {
                [
                    subscription::build_depth_subs(exchange, specs),
                    subscription::build_trade_subs(exchange, specs),
                    subscription::build_kline_subs(exchange, specs),
                ]
                .into_iter()
                .flatten()
            })
            .collect::<Vec<_>>();

        Subscription::batch(subs)
    }

    pub fn theme_updated(&mut self, main_window: window::Id, theme: &iced_core::Theme) {
        self.iter_all_panes_mut(main_window)
            .for_each(|(_, _, state)| {
                state.content.update_theme(theme);
            });
    }

    /// リプレイ用にペインの content をクリアし、各ペインの kline StreamKind + pane_id を返す。
    /// settings / streams はそのまま保持する。
    /// Kline ストリームを収集する（チャートはクリアしない）。
    /// Play バリデーション用。
    pub fn peek_kline_streams(&self, main_window: window::Id) -> Vec<(uuid::Uuid, StreamKind)> {
        let mut kline_targets = Vec::new();
        for (_, _, state) in self.iter_all_panes(main_window) {
            let pane_id = state.unique_id();
            if let Some(streams) = state.streams.ready_iter() {
                for stream in streams {
                    if matches!(stream, StreamKind::Kline { .. }) {
                        kline_targets.push((pane_id, *stream));
                    }
                }
            }
        }
        kline_targets
    }

    pub fn prepare_replay(&mut self, main_window: window::Id) -> Vec<(uuid::Uuid, StreamKind)> {
        let mut kline_targets = Vec::new();

        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            let pane_id = state.unique_id();

            // Collect kline streams for this pane
            if let Some(streams) = state.streams.ready_iter() {
                for stream in streams {
                    if matches!(stream, StreamKind::Kline { .. }) {
                        kline_targets.push((pane_id, *stream));
                    }
                }
            }

            // Rebuild content: clear chart data, preserve layout/indicators/kind
            state.rebuild_content_for_replay();
        }

        kline_targets
    }

    /// StepBackward 用: kline 収集をせずチャートデータのみクリアする。
    pub fn clear_chart_for_replay(&mut self, main_window: window::Id) {
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            state.rebuild_content_for_replay();
        }
    }

    /// StepBackward/StepForward seek 用: ビューポートを保持したままデータのみリセットする。
    /// `clear_chart_for_replay` と異なり KlineChart を再構築しないため、チラつきが発生しない。
    pub fn reset_charts_for_seek(&mut self, main_window: window::Id) {
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            state.reset_for_seek();
        }
    }

    /// Replay→Live 切替時にペインの content をリビルドする（replay_kline_buffer を無効化）。
    pub fn rebuild_for_live(&mut self, main_window: window::Id) {
        for (_, _, state) in self.iter_all_panes_mut(main_window) {
            state.rebuild_content_for_live();
        }
    }

    /// リプレイ用に全ペインの trades StreamKind を（重複なしで）収集する。
    pub fn collect_trade_streams(&self, main_window: window::Id) -> Vec<StreamKind> {
        let mut seen = Vec::new();
        for (_, _, state) in self.iter_all_panes(main_window) {
            if let Some(streams) = state.streams.ready_iter() {
                for stream in streams {
                    if matches!(stream, StreamKind::Trades { .. }) && !seen.contains(stream) {
                        seen.push(*stream);
                    }
                }
            }
        }
        seen
    }

    fn refresh_streams(&mut self, main_window: window::Id) -> Task<Message> {
        let all_pane_streams = self
            .iter_all_panes(main_window)
            .flat_map(|(_, _, pane_state)| pane_state.streams.ready_iter().into_iter().flatten());
        self.streams = UniqueStreams::from(all_pane_streams);

        Task::none()
    }
}

impl From<fetcher::FetchUpdate> for Message {
    fn from(update: fetcher::FetchUpdate) -> Self {
        match update {
            fetcher::FetchUpdate::Status { pane_id, status } => match status {
                fetcher::FetchTaskStatus::Loading(info) => {
                    Message::ChangePaneStatus(pane_id, pane::Status::Loading(info))
                }
                fetcher::FetchTaskStatus::Completed => {
                    Message::ChangePaneStatus(pane_id, pane::Status::Ready)
                }
            },
            fetcher::FetchUpdate::Data {
                layout_id,
                pane_id,
                stream,
                data,
            } => Message::DistributeFetchedData {
                layout_id,
                pane_id,
                stream,
                data,
            },
            fetcher::FetchUpdate::Error { pane_id, error } => {
                Message::ErrorOccurred(Some(pane_id), DashboardError::Fetch(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // compile-time: clear_chart_for_replay returns (), not Vec<...
    fn _type_check_clear_chart_for_replay_returns_unit(
        d: &mut super::Dashboard,
        id: iced::window::Id,
    ) {
        let _: () = d.clear_chart_for_replay(id);
    }

    /// MiniTickersList で銘柄切り替えを行うと SwitchTickersInGroup イベントが返ること。
    /// このイベントが main.rs に伝搬することで ReloadKlineStream が発火される（リプレイ修正）。
    #[test]
    fn mini_tickers_list_switch_emits_switch_tickers_in_group_event() {
        use crate::modal::pane::{
            Modal,
            mini_tickers_list::{Message as MiniMsg, MiniPanel, RowSelection},
        };
        use crate::window::Window;

        let main_window = Window::new(iced::window::Id::unique());
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();

        // 最初のペインを取得し、MiniTickersList モーダルを開いた状態にする
        let pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let state = dashboard.panes.get_mut(pane).unwrap();
        state.modal = Some(Modal::MiniTickersList(MiniPanel::new()));

        let ticker_info = exchange::TickerInfo::new(
            exchange::Ticker::new("BTCUSDT", exchange::adapter::Exchange::BinanceLinear),
            0.1,
            0.001,
            None,
        );

        // ペイン左上クリック → MiniTickersList → Switch 選択を模倣するメッセージ
        let msg = Message::Pane(
            main_window.id,
            pane::Message::PaneEvent(
                pane,
                pane::Event::MiniTickersListInteraction(MiniMsg::RowSelected(
                    RowSelection::Switch(ticker_info),
                )),
            ),
        );

        let (_task, event) = dashboard.update(msg, &main_window, &layout_id);

        // SwitchTickersInGroup イベントが返ること（None だとリプレイが更新されないバグ）
        assert!(
            matches!(&event, Some(Event::SwitchTickersInGroup { ticker_info: ti }) if *ti == ticker_info),
            "expected Some(Event::SwitchTickersInGroup {{ ticker_info }}) but got {:?}",
            event
        );
    }

    // --- update() Message バリアントの振る舞いテスト ---

    fn make_window() -> crate::window::Window {
        crate::window::Window::new(iced::window::Id::unique())
    }

    #[test]
    fn update_request_palette_emits_event() {
        let main_window = make_window();
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();

        let (_task, event) = dashboard.update(Message::RequestPalette, &main_window, &layout_id);

        assert!(
            matches!(event, Some(Event::RequestPalette)),
            "RequestPalette must emit Event::RequestPalette, got {event:?}"
        );
    }

    #[test]
    fn update_notification_passes_through() {
        use crate::widget::toast::Toast;
        let main_window = make_window();
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();

        let toast = Toast::info("test message".to_string());
        let (_task, event) = dashboard.update(
            Message::Notification(toast.clone()),
            &main_window,
            &layout_id,
        );

        assert!(
            matches!(event, Some(Event::Notification(_))),
            "Notification must emit Event::Notification, got {event:?}"
        );
    }

    #[test]
    fn update_resolve_streams_emits_event() {
        let main_window = make_window();
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();
        let pane_id = uuid::Uuid::new_v4();

        let (_task, event) = dashboard.update(
            Message::ResolveStreams(pane_id, vec![]),
            &main_window,
            &layout_id,
        );

        assert!(
            matches!(event, Some(Event::ResolveStreams { pane_id: pid, .. }) if pid == pane_id),
            "ResolveStreams must emit Event::ResolveStreams with matching pane_id, got {event:?}"
        );
    }

    #[test]
    fn update_change_pane_status_updates_state() {
        use crate::connector::fetcher::InfoKind;
        let main_window = make_window();
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();

        let pane = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();
        let pane_uuid = dashboard.panes.get(pane).unwrap().unique_id();

        let (_task, event) = dashboard.update(
            Message::ChangePaneStatus(pane_uuid, pane::Status::Loading(InfoKind::FetchingKlines)),
            &main_window,
            &layout_id,
        );

        assert!(event.is_none(), "ChangePaneStatus must not emit an event");
        let state = dashboard.panes.get(pane).unwrap();
        assert!(
            matches!(
                state.status,
                pane::Status::Loading(InfoKind::FetchingKlines)
            ),
            "pane status must be updated to Loading(FetchingKlines)"
        );
    }

    #[test]
    fn update_virtual_order_filled_emits_notification() {
        use crate::replay::virtual_exchange::order_book::FillEvent;
        use crate::replay::virtual_exchange::portfolio::PositionSide;
        let main_window = make_window();
        let layout_id = uuid::Uuid::new_v4();
        let mut dashboard = Dashboard::default();

        let fill = FillEvent {
            order_id: "test-order".to_string(),
            ticker: "BTCUSDT".to_string(),
            side: PositionSide::Long,
            qty: 0.001,
            fill_price: 50000.0,
            fill_time_ms: 0,
        };

        let (_task, event) =
            dashboard.update(Message::VirtualOrderFilled(fill), &main_window, &layout_id);

        assert!(
            matches!(event, Some(Event::Notification(_))),
            "VirtualOrderFilled must emit Event::Notification, got {event:?}"
        );
    }

    #[test]
    fn market_subscriptions_returns_batch_for_empty_dashboard() {
        // ストリームが一切ない Dashboard では Subscription::batch([]) が返る。
        // パニックしないことだけを確認する。
        let dashboard = Dashboard::default();
        let _sub = dashboard.market_subscriptions();
        // コンパイルが通り、パニックしなければ OK
    }

    #[test]
    fn sync_issue_to_order_entry_updates_panel() {
        // sync_issue_to_order_entry() が OrderEntry ペインを更新することを確認
        use crate::screen::dashboard::panel::order_entry::OrderEntryPanel;

        let main_window = make_window();

        // 1. Dashboard を作成
        let mut dashboard = Dashboard::default();
        let pane1 = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();

        // 2. ペイン1 を OrderEntry に設定
        if let Some(state) = dashboard.get_mut_pane(main_window.id, main_window.id, pane1) {
            state.content = pane::Content::OrderEntry(OrderEntryPanel::new());
        }

        // 3. OrderEntry の初期状態を確認（銘柄未選択）
        if let Some(state) = dashboard.get_pane(main_window.id, main_window.id, pane1) {
            if let pane::Content::OrderEntry(panel) = &state.content {
                assert_eq!(panel.issue_code, "", "OrderEntry should start empty");
            }
        }

        // 4. sync_issue_to_order_entry() を直接呼び出し
        let _task = dashboard.sync_issue_to_order_entry(
            main_window.id,
            "TachibanaSpot:7201".to_string(),
            "TachibanaSpot:7201".to_string(),
            Some(0.1),
        );

        // 5. OrderEntry の issue_code が更新されたことを確認
        if let Some(state) = dashboard.get_pane(main_window.id, main_window.id, pane1) {
            if let pane::Content::OrderEntry(panel) = &state.content {
                assert_eq!(
                    panel.issue_code, "TachibanaSpot:7201",
                    "sync_issue_to_order_entry should update issue_code immediately"
                );
            }
        }
    }

    #[test]
    fn switch_tickers_syncs_to_order_entry_panel() {
        // チャートペインで銘柄を切り替えると、同じリンクグループの OrderEntry ペインが同期される
        use crate::screen::dashboard::panel::order_entry::OrderEntryPanel;
        use data::layout::pane::{ContentKind, LinkGroup};
        use exchange::adapter::Exchange;

        let main_window = make_window();

        // 1. Dashboard を作成し、最初のペインをチャートペインに設定
        let mut dashboard = Dashboard::default();
        let pane1 = *dashboard.panes.iter().next().map(|(p, _)| p).unwrap();

        let toyota = exchange::TickerInfo::new(
            exchange::Ticker::new("7203", Exchange::Tachibana),
            0.1,
            1.0,
            None,
        );

        // ペイン1 を Kline チャートに設定（TOYOTA）
        if let Some(state) = dashboard.get_mut_pane(main_window.id, main_window.id, pane1) {
            state.set_content_and_streams(vec![toyota.clone()], ContentKind::CandlestickChart);
            state.link_group = Some(LinkGroup::A);
        }

        // 2. 2 番目のペインを分割で作成し、OrderEntry に設定
        let (_pane2, _) = dashboard
            .panes
            .split(
                iced::widget::pane_grid::Axis::Horizontal,
                pane1,
                pane::State::new(),
            )
            .expect("split should succeed");
        let pane2 = *dashboard
            .panes
            .iter()
            .find(|(p, _)| **p != pane1)
            .map(|(p, _)| p)
            .unwrap();

        if let Some(state) = dashboard.get_mut_pane(main_window.id, main_window.id, pane2) {
            state.content = pane::Content::OrderEntry(OrderEntryPanel::new());
            state.link_group = Some(LinkGroup::A); // Same link group as pane1
        }

        // 3. OrderEntry の初期状態を確認（銘柄未選択）
        if let Some(state) = dashboard.get_pane(main_window.id, main_window.id, pane2) {
            if let pane::Content::OrderEntry(panel) = &state.content {
                assert_eq!(
                    panel.issue_code, "",
                    "OrderEntry should start with empty issue_code"
                );
            }
        }

        // 4. チャートペインで NISSAN に切り替え（同じリンクグループなので両方が切り替わる）
        let nissan = exchange::TickerInfo::new(
            exchange::Ticker::new("7201", Exchange::Tachibana),
            0.1,
            1.0,
            None,
        );

        // Set focus to pane1 so init_focused_pane will be called
        dashboard.focus = Some((main_window.id, pane1));

        let _task = dashboard.switch_tickers_in_group(main_window.id, nissan.clone(), false);

        // 5. OrderEntry の issue_code が NISSAN に更新されていることを確認
        if let Some(state) = dashboard.get_pane(main_window.id, main_window.id, pane2) {
            if let pane::Content::OrderEntry(panel) = &state.content {
                assert_eq!(
                    panel.issue_code, "TachibanaSpot:7201",
                    "OrderEntry issue_code should be synced to NISSAN after chart ticker switches"
                );
            } else {
                panic!("pane2 should be OrderEntry panel");
            }
        } else {
            panic!("couldn't get pane2");
        }
    }
}
