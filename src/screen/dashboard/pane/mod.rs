mod content;
mod controls;
mod effect;

pub use content::Content;
pub use effect::Effect;

use controls::{basis_modifier, link_group_modal, ticksize_modifier};

use crate::{
    chart::{self, comparison::ComparisonChart, kline::KlineChart},
    connector::{ResolvedStream, fetcher::InfoKind},
    modal::{
        self, ModifierKind,
        pane::{
            Modal,
            mini_tickers_list::MiniPanel,
            settings::{
                comparison_cfg_view, heatmap_cfg_view, heatmap_shader_cfg_view, kline_cfg_view,
            },
            stack_modal,
        },
    },
    screen::dashboard::{
        panel::{self, ladder::Ladder, timeandsales::TimeAndSales},
        tickers_table::TickersTable,
    },
    style::{self, Icon, icon_text},
    widget::{
        self, button_with_tooltip, chart::heatmap::HeatmapShader, column_drag, link_group_button,
        toast::Toast,
    },
    window::{self, Window},
};
use data::{
    UserTimezone,
    chart::{
        Basis,
        heatmap::HeatmapStudy,
        indicator::{HeatmapIndicator, UiIndicator},
    },
    layout::pane::{ContentKind, LinkGroup, PaneSetup, Settings, VisualConfig},
    stream::PersistStreamKind,
};
use exchange::{
    Kline, OpenInterest, StreamPairKind, TickMultiplier, TickerInfo, Timeframe,
    adapter::{MarketKind, StreamKind, StreamTicksize},
};
use iced::{
    Alignment, Element, Length, Renderer, Theme, padding,
    widget::{button, center, column, container, pane_grid, pick_list, row, text, tooltip},
};
use std::time::Instant;

#[derive(Debug, Default, Clone, PartialEq)]
pub enum Status {
    #[default]
    Ready,
    Loading(InfoKind),
    Stale(String),
}

pub enum Action {
    Chart(chart::Action),
    Panel(panel::Action),
    ResolveStreams(Vec<PersistStreamKind>),
    ResolveContent,
}

#[derive(Debug, Clone)]
pub enum Message {
    PaneClicked(pane_grid::Pane),
    PaneResized(pane_grid::ResizeEvent),
    PaneDragged(pane_grid::DragEvent),
    ClosePane(pane_grid::Pane),
    SplitPane(pane_grid::Axis, pane_grid::Pane),
    MaximizePane(pane_grid::Pane),
    Restore,
    ReplacePane(pane_grid::Pane),
    Popout,
    Merge,
    SwitchLinkGroup(pane_grid::Pane, Option<LinkGroup>),
    VisualConfigChanged(pane_grid::Pane, VisualConfig, bool),
    PaneEvent(pane_grid::Pane, Event),
}

#[derive(Debug, Clone)]
pub enum Event {
    ShowModal(Modal),
    HideModal,
    ContentSelected(ContentKind),
    ChartInteraction(super::chart::Message),
    PanelInteraction(super::panel::Message),
    ToggleIndicator(UiIndicator),
    DeleteNotification(usize),
    ReorderIndicator(column_drag::DragEvent),
    ClusterKindSelected(data::chart::kline::ClusterKind),
    ClusterScalingSelected(data::chart::kline::ClusterScaling),
    StudyConfigurator(modal::pane::settings::study::StudyMessage),
    StreamModifierChanged(modal::stream::Message),
    ComparisonChartInteraction(super::chart::comparison::Message),
    HeatmapShaderInteraction(crate::widget::chart::heatmap::Message),
    MiniTickersListInteraction(modal::pane::mini_tickers_list::Message),
}

pub struct State {
    id: uuid::Uuid,
    pub modal: Option<Modal>,
    pub content: Content,
    pub settings: Settings,
    pub notifications: Vec<Toast>,
    pub streams: ResolvedStream,
    pub status: Status,
    pub link_group: Option<LinkGroup>,
    /// true = REPLAYモード（仮想注文）。dashboard.rs から is_replay に連動して設定する。
    pub is_virtual_mode: bool,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_config(
        content: Content,
        streams: Vec<PersistStreamKind>,
        settings: Settings,
        link_group: Option<LinkGroup>,
    ) -> Self {
        Self {
            content,
            settings,
            streams: ResolvedStream::waiting(streams),
            link_group,
            ..Default::default()
        }
    }

    pub fn stream_pair(&self) -> Option<TickerInfo> {
        self.streams.find_ready_map(|stream| match stream {
            StreamKind::Kline { ticker_info, .. } => Some(*ticker_info),
            StreamKind::Depth { ticker_info, .. } => Some(*ticker_info),
            StreamKind::Trades { ticker_info, .. } => Some(*ticker_info),
        })
    }

    pub fn stream_pair_kind(&self) -> Option<StreamPairKind> {
        let ready_streams = self.streams.ready_iter()?;
        let mut unique = vec![];

        for stream in ready_streams {
            let ticker = stream.ticker_info();
            if !unique.contains(&ticker) {
                unique.push(ticker);
            }
        }

        match unique.len() {
            0 => None,
            1 => Some(StreamPairKind::SingleSource(unique[0])),
            _ => Some(StreamPairKind::MultiSource(unique)),
        }
    }

    pub fn set_content_and_streams(
        &mut self,
        tickers: Vec<TickerInfo>,
        kind: ContentKind,
    ) -> Vec<StreamKind> {
        if self.content.kind() != kind {
            self.settings.selected_basis = None;
            self.settings.tick_multiply = None;
        }

        let Some(&base_ticker) = tickers.first() else {
            log::warn!("set_content_and_streams: empty tickers — skipping");
            return vec![];
        };
        let prev_base_ticker = self.stream_pair();

        let derived_plan = PaneSetup::new(
            kind,
            base_ticker,
            prev_base_ticker,
            self.settings.selected_basis,
            self.settings.tick_multiply,
        );

        self.settings.selected_basis = derived_plan.basis;
        self.settings.tick_multiply = derived_plan.tick_multiplier;

        let (content, streams) = {
            let kline_stream = |ti: TickerInfo, tf: Timeframe| StreamKind::Kline {
                ticker_info: ti,
                timeframe: tf,
            };
            let depth_stream = |derived_plan: &PaneSetup| StreamKind::Depth {
                ticker_info: derived_plan.ticker_info,
                depth_aggr: derived_plan.depth_aggr,
                push_freq: derived_plan.push_freq,
            };
            let trades_stream = |derived_plan: &PaneSetup| StreamKind::Trades {
                ticker_info: derived_plan.ticker_info,
            };

            match kind {
                ContentKind::HeatmapChart => {
                    let content = Content::new_heatmap(
                        &self.content,
                        derived_plan.ticker_info,
                        &self.settings,
                        derived_plan.price_step,
                    );

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::FootprintChart => {
                    let content = Content::new_kline(
                        kind,
                        &self.content,
                        derived_plan.ticker_info,
                        &self.settings,
                        derived_plan.price_step,
                    );

                    let streams = by_basis_default(
                        derived_plan.basis,
                        Timeframe::M5,
                        |tf| {
                            vec![
                                trades_stream(&derived_plan),
                                kline_stream(derived_plan.ticker_info, tf),
                            ]
                        },
                        || vec![trades_stream(&derived_plan)],
                    );

                    (content, streams)
                }
                ContentKind::CandlestickChart => {
                    let content = Content::new_kline(
                        kind,
                        &self.content,
                        derived_plan.ticker_info,
                        &self.settings,
                        base_ticker.min_ticksize.into(),
                    );

                    let time_basis_stream = |tf| vec![kline_stream(derived_plan.ticker_info, tf)];
                    let tick_basis_stream = || {
                        let depth_aggr = derived_plan
                            .ticker_info
                            .exchange()
                            .stream_ticksize(None, TickMultiplier(50));
                        let temp = PaneSetup {
                            depth_aggr,
                            ..derived_plan
                        };
                        vec![trades_stream(&temp)]
                    };

                    let streams = by_basis_default(
                        derived_plan.basis,
                        Timeframe::M15,
                        time_basis_stream,
                        tick_basis_stream,
                    );

                    (content, streams)
                }
                ContentKind::TimeAndSales => {
                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.time_and_sales());
                    let content = Content::TimeAndSales(Some(TimeAndSales::new(
                        config,
                        derived_plan.ticker_info,
                    )));

                    let temp = PaneSetup {
                        push_freq: exchange::PushFrequency::ServerDefault,
                        ..derived_plan
                    };

                    let streams = vec![trades_stream(&temp)];

                    (content, streams)
                }
                ContentKind::Ladder => {
                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.ladder());
                    let content = Content::Ladder(Some(Ladder::new(
                        config,
                        derived_plan.ticker_info,
                        derived_plan.price_step,
                    )));

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::ComparisonChart => {
                    let config = self
                        .settings
                        .visual_config
                        .clone()
                        .and_then(|cfg| cfg.comparison());

                    let timeframe = {
                        let supports = |tf| {
                            tickers
                                .iter()
                                .all(|ti| ti.exchange().supports_kline_timeframe(tf))
                        };

                        if let Some(tf) = derived_plan.basis.and_then(|basis| match basis {
                            Basis::Time(tf) => Some(tf),
                            Basis::Tick(_) => None,
                        }) && supports(tf)
                        {
                            tf
                        } else {
                            let fallback = Timeframe::M15;
                            if supports(fallback) {
                                fallback
                            } else {
                                Timeframe::KLINE
                                    .iter()
                                    .copied()
                                    .find(|tf| supports(*tf))
                                    .unwrap_or(fallback)
                            }
                        }
                    };

                    let basis = Basis::Time(timeframe);
                    self.settings.selected_basis = Some(basis);
                    let content =
                        Content::Comparison(Some(ComparisonChart::new(basis, &tickers, config)));

                    let streams = tickers
                        .iter()
                        .copied()
                        .map(|ti| kline_stream(ti, timeframe))
                        .collect();

                    (content, streams)
                }
                ContentKind::ShaderHeatmap => {
                    let basis = derived_plan
                        .basis
                        .unwrap_or(Basis::default_heatmap_time(Some(derived_plan.ticker_info)));

                    let (studies, indicators) = if let Content::ShaderHeatmap {
                        chart,
                        indicators,
                        studies,
                    } = &self.content
                    {
                        (
                            chart
                                .as_ref()
                                .map_or(studies.clone(), |c| c.studies.clone()),
                            indicators.clone(),
                        )
                    } else {
                        (
                            vec![HeatmapStudy::VolumeProfile(
                                data::chart::heatmap::ProfileKind::default(),
                            )],
                            vec![HeatmapIndicator::Volume],
                        )
                    };

                    let content = Content::ShaderHeatmap {
                        chart: Some(Box::new(HeatmapShader::new(
                            basis,
                            derived_plan.price_step,
                            base_ticker,
                            studies.clone(),
                            indicators.clone(),
                        ))),
                        studies,
                        indicators,
                    };

                    let streams = vec![depth_stream(&derived_plan), trades_stream(&derived_plan)];

                    (content, streams)
                }
                ContentKind::Starter
                | ContentKind::OrderEntry
                | ContentKind::OrderList
                | ContentKind::BuyingPower => {
                    log::warn!(
                        "set_content_and_streams: unexpected kind {:?} — skipping",
                        kind
                    );
                    return vec![];
                }
            }
        };

        self.content = content;
        self.streams = ResolvedStream::Ready(streams.clone());

        streams
    }

    pub fn insert_hist_oi(&mut self, req_id: Option<uuid::Uuid>, oi: &[OpenInterest]) {
        match &mut self.content {
            Content::Kline { chart, .. } => {
                let Some(chart) = chart else {
                    log::warn!("insert_hist_oi: Kline chart not initialized, skipping");
                    return;
                };
                chart.insert_open_interest(req_id, oi);
            }
            _ => {
                log::warn!(
                    "insert_hist_oi: unexpected content kind {:?}",
                    self.content.kind()
                );
            }
        }
    }

    /// リプレイ開始時にチャートデータをクリアし、settings/streams/layout/indicators は保持する。
    pub fn rebuild_content_for_replay(&mut self) {
        self.rebuild_content(true);
    }

    pub fn rebuild_content_for_live(&mut self) {
        self.rebuild_content(false);
    }

    fn rebuild_content(&mut self, replay_mode: bool) {
        // ticker_info を先に取得してからコンテンツを変更する（借用競合を回避）
        let ticker_info = self.stream_pair();

        match &mut self.content {
            Content::Kline {
                chart,
                indicators,
                layout,
                kind,
            } => {
                if let (Some(c), Some(ti)) = (chart.as_ref(), ticker_info) {
                    let step = c.tick_size();
                    let saved_layout = c.chart_layout();
                    let saved_kind = c.kind().clone();
                    let basis = c.basis();
                    let saved_indicators = indicators.clone();

                    let mut new_chart = KlineChart::new(
                        saved_layout.clone(),
                        basis,
                        step,
                        &[],
                        vec![],
                        &saved_indicators,
                        ti,
                        &saved_kind,
                    );
                    new_chart.set_replay_mode(replay_mode);
                    *chart = Some(new_chart);
                    *layout = saved_layout;
                    *kind = saved_kind;
                }
            }
            Content::TimeAndSales(panel) => {
                if let (Some(p), Some(ti)) = (panel.as_ref(), ticker_info) {
                    let config = p.config;
                    *panel = Some(TimeAndSales::new(Some(config), ti));
                }
            }
            _ => {
                // Heatmap, ShaderHeatmap, Ladder: Phase 3 で「Depth unavailable」を表示
            }
        }
    }

    pub fn insert_hist_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        match &mut self.content {
            Content::Kline {
                chart, indicators, ..
            } => {
                let Some(chart) = chart else {
                    log::warn!("insert_hist_klines: Kline chart not initialized, skipping");
                    return;
                };

                if let Some(id) = req_id {
                    if chart.basis() != Basis::Time(timeframe) {
                        log::warn!(
                            "Ignoring stale kline fetch for timeframe {:?}; chart basis = {:?}",
                            timeframe,
                            chart.basis()
                        );
                        return;
                    }
                    chart.insert_hist_klines(id, klines);
                } else {
                    let (raw_trades, tick_size) = (chart.raw_trades(), chart.tick_size());
                    let layout = chart.chart_layout();

                    *chart = KlineChart::new(
                        layout,
                        Basis::Time(timeframe),
                        tick_size,
                        klines,
                        raw_trades,
                        indicators,
                        ticker_info,
                        chart.kind(),
                    );
                }
            }
            Content::Comparison(chart) => {
                let Some(chart) = chart else {
                    log::warn!("insert_hist_klines: Comparison chart not initialized, skipping");
                    return;
                };

                if let Some(id) = req_id {
                    if chart.timeframe != timeframe {
                        log::warn!(
                            "Ignoring stale kline fetch for timeframe {:?}; chart timeframe = {:?}",
                            timeframe,
                            chart.timeframe
                        );
                        return;
                    }
                    chart.insert_history(id, ticker_info, klines);
                } else {
                    *chart = ComparisonChart::new(
                        Basis::Time(timeframe),
                        &[ticker_info],
                        Some(chart.serializable_config()),
                    );
                }
            }
            _ => {
                log::warn!(
                    "insert_hist_klines: unexpected content kind {:?}",
                    self.content.kind()
                );
            }
        }
    }

    fn has_stream(&self) -> bool {
        match &self.streams {
            ResolvedStream::Ready(streams) => !streams.is_empty(),
            ResolvedStream::Waiting { streams, .. } => !streams.is_empty(),
        }
    }

    pub fn view<'a>(
        &'a self,
        id: pane_grid::Pane,
        panes: usize,
        is_focused: bool,
        maximized: bool,
        window: window::Id,
        main_window: &'a Window,
        timezone: UserTimezone,
        tickers_table: &'a TickersTable,
        is_replay: bool,
        theme: &'a Theme,
    ) -> pane_grid::Content<'a, Message, Theme, Renderer> {
        let is_ticker_modal_active = matches!(self.modal, Some(Modal::MiniTickersList(_)));
        let mini_tickers_btn = |content: Element<'a, Message>| {
            button(content)
                .on_press(Message::PaneEvent(
                    id,
                    Event::ShowModal(Modal::MiniTickersList(MiniPanel::new())),
                ))
                .style(move |theme, status| {
                    style::button::modifier(theme, status, !is_ticker_modal_active)
                })
                .height(widget::PANE_CONTROL_BTN_HEIGHT)
        };

        let mut top_left_buttons = if Content::Starter == self.content {
            row![]
        } else {
            row![link_group_button(id, self.link_group, |id| {
                Message::PaneEvent(id, Event::ShowModal(Modal::LinkGroup))
            })]
        };

        if let Some(kind) = self.stream_pair_kind() {
            let (base_ti, extra) = match kind {
                StreamPairKind::MultiSource(list) => (list[0], list.len().saturating_sub(1)),
                StreamPairKind::SingleSource(ti) => (ti, 0),
            };

            let exchange_icon = icon_text(style::venue_icon(base_ti.ticker.exchange.venue()), 14);
            let mut label = {
                let symbol = base_ti.ticker.display_symbol_and_type().0;
                match base_ti.ticker.market_type() {
                    MarketKind::Spot => symbol,
                    MarketKind::LinearPerps | MarketKind::InversePerps => symbol + " PERP",
                }
            };
            if extra > 0 {
                label = format!("{label} +{extra}");
            }

            let content = row![
                exchange_icon.align_y(Alignment::Center).line_height(1.4),
                text(label)
                    .size(14)
                    .align_y(Alignment::Center)
                    .line_height(1.4)
            ]
            .align_y(Alignment::Center)
            .spacing(4);

            top_left_buttons = top_left_buttons.push(mini_tickers_btn(content.into()));
        } else if !matches!(
            self.content,
            Content::Starter
                | Content::OrderEntry(_)
                | Content::OrderList(_)
                | Content::BuyingPower(_)
        ) && !self.has_stream()
        {
            let content = row![
                text("Choose a ticker")
                    .size(13)
                    .align_y(Alignment::Center)
                    .line_height(1.4)
            ]
            .align_y(Alignment::Center);

            top_left_buttons = top_left_buttons.push(mini_tickers_btn(content.into()));
        }

        match &self.content {
            Content::OrderEntry(_) => {
                top_left_buttons = top_left_buttons.push(
                    text("注文入力")
                        .size(13)
                        .align_y(Alignment::Center)
                        .line_height(1.4),
                );
            }
            Content::OrderList(_) => {
                top_left_buttons = top_left_buttons.push(
                    text("注文一覧")
                        .size(13)
                        .align_y(Alignment::Center)
                        .line_height(1.4),
                );
            }
            Content::BuyingPower(_) => {
                top_left_buttons = top_left_buttons.push(
                    text("買付余力")
                        .size(13)
                        .align_y(Alignment::Center)
                        .line_height(1.4),
                );
            }
            _ => {}
        }

        let modifier: Option<modal::stream::Modifier> = self.modal.as_ref().and_then(|m| {
            if let Modal::StreamModifier(modifier) = m {
                Some(*modifier)
            } else {
                None
            }
        });

        let compact_controls = if self.modal == Some(Modal::Controls) {
            Some(
                container(self.view_controls(id, panes, maximized, window != main_window.id))
                    .style(style::chart_modal)
                    .into(),
            )
        } else {
            None
        };

        let uninitialized_base = |kind: ContentKind| -> Element<'a, Message> {
            if self.has_stream() {
                center(text("Loading…").size(16)).into()
            } else {
                let content = column![
                    text(kind.to_string()).size(16),
                    text("No ticker selected").size(14)
                ]
                .spacing(8)
                .align_x(Alignment::Center);

                center(content).into()
            }
        };

        let body = match &self.content {
            Content::Starter => {
                let content_picklist =
                    pick_list(ContentKind::ALL, Some(ContentKind::Starter), move |kind| {
                        Message::PaneEvent(id, Event::ContentSelected(kind))
                    });

                let base: Element<_> = widget::toast::Manager::new(
                    center(
                        column![
                            text("Choose a view to get started").size(16),
                            content_picklist
                        ]
                        .align_x(Alignment::Center)
                        .spacing(12),
                    ),
                    &self.notifications,
                    Alignment::End,
                    move |msg| Message::PaneEvent(id, Event::DeleteNotification(msg)),
                )
                .into();

                self.compose_stack_view(
                    base,
                    id,
                    None,
                    compact_controls,
                    || column![].into(),
                    None,
                    tickers_table,
                )
            }
            Content::Comparison(chart) => {
                if let Some(c) = chart {
                    let selected_basis = Basis::Time(c.timeframe);
                    let kind = ModifierKind::Comparison(selected_basis);

                    let modifiers =
                        row![basis_modifier(id, selected_basis, modifier, kind),].spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    let base = c.view(timezone).map(move |message| {
                        Message::PaneEvent(id, Event::ComparisonChartInteraction(message))
                    });

                    let settings_modal = || comparison_cfg_view(id, c);

                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        settings_modal,
                        Some(c.selected_tickers()),
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::ComparisonChart);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::TimeAndSales(panel) => {
                if let Some(panel) = panel {
                    let base = panel::view(panel, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::PanelInteraction(message))
                    });

                    let settings_modal =
                        || modal::pane::settings::timesales_cfg_view(panel.config, id);

                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        settings_modal,
                        None,
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::TimeAndSales);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::Ladder(panel) => {
                if let Some(panel) = panel {
                    let basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(self.stream_pair()));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(1));

                    let stream_pair = self.stream_pair();

                    let price_step = stream_pair
                        .map(|ti| {
                            tick_multiply.unscale_step_or_min_tick(panel.step, ti.min_ticksize)
                        })
                        .unwrap_or_else(|| tick_multiply.unscale_step(panel.step));

                    let exchange = stream_pair.map(|ti| ti.ticker.exchange);
                    let min_ticksize = stream_pair.map(|ti| ti.min_ticksize);

                    let modifiers = ticksize_modifier(
                        id,
                        price_step,
                        min_ticksize,
                        tick_multiply,
                        modifier,
                        ModifierKind::Orderbook(basis, tick_multiply),
                        exchange,
                    );

                    top_left_buttons = top_left_buttons.push(modifiers);

                    let base = panel::view(panel, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::PanelInteraction(message))
                    });

                    let settings_modal =
                        || modal::pane::settings::ladder_cfg_view(panel.config, id);

                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        settings_modal,
                        None,
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::Ladder);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::Heatmap {
                chart, indicators, ..
            } => {
                if let Some(chart) = chart {
                    let ticker_info = self.stream_pair();
                    let exchange = ticker_info.as_ref().map(|info| info.ticker.exchange);

                    let basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(ticker_info));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(5));

                    let kind = ModifierKind::Heatmap(basis, tick_multiply);
                    let price_step = ticker_info
                        .map(|ti| {
                            tick_multiply
                                .unscale_step_or_min_tick(chart.tick_size(), ti.min_ticksize)
                        })
                        .unwrap_or_else(|| tick_multiply.unscale_step(chart.tick_size()));
                    let min_ticksize = ticker_info.map(|ti| ti.min_ticksize);

                    let modifiers = row![
                        basis_modifier(id, basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            price_step,
                            min_ticksize,
                            tick_multiply,
                            modifier,
                            kind,
                            exchange
                        ),
                    ]
                    .spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    let base = chart::view(chart, indicators, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::ChartInteraction(message))
                    });
                    let settings_modal = || {
                        heatmap_cfg_view(
                            chart.visual_config(),
                            id,
                            chart.study_configurator(),
                            &chart.studies,
                            basis,
                        )
                    };

                    let indicator_modal = if self.modal == Some(Modal::Indicators) {
                        Some(modal::indicators::view(
                            id,
                            self,
                            indicators,
                            self.stream_pair().map(|i| i.ticker.market_type()),
                        ))
                    } else {
                        None
                    };

                    self.compose_stack_view(
                        base,
                        id,
                        indicator_modal,
                        compact_controls,
                        settings_modal,
                        None,
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::HeatmapChart);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::Kline {
                chart,
                indicators,
                kind: chart_kind,
                ..
            } => {
                if let Some(chart) = chart {
                    match chart_kind {
                        data::chart::KlineChartKind::Footprint { .. } => {
                            let basis = chart.basis();
                            let tick_multiply =
                                self.settings.tick_multiply.unwrap_or(TickMultiplier(10));

                            let kind = ModifierKind::Footprint(basis, tick_multiply);
                            let stream_pair = self.stream_pair();
                            let price_step = stream_pair
                                .map(|ti| {
                                    tick_multiply.unscale_step_or_min_tick(
                                        chart.tick_size(),
                                        ti.min_ticksize,
                                    )
                                })
                                .unwrap_or_else(|| tick_multiply.unscale_step(chart.tick_size()));

                            let exchange = stream_pair.as_ref().map(|info| info.ticker.exchange);
                            let min_ticksize = stream_pair.map(|ti| ti.min_ticksize);

                            let modifiers = row![
                                basis_modifier(id, basis, modifier, kind),
                                ticksize_modifier(
                                    id,
                                    price_step,
                                    min_ticksize,
                                    tick_multiply,
                                    modifier,
                                    kind,
                                    exchange
                                ),
                            ]
                            .spacing(4);

                            top_left_buttons = top_left_buttons.push(modifiers);
                        }
                        data::chart::KlineChartKind::Candles => {
                            let selected_basis = chart.basis();
                            let kind = ModifierKind::Candlestick(selected_basis);

                            let modifiers =
                                row![basis_modifier(id, selected_basis, modifier, kind),]
                                    .spacing(4);

                            top_left_buttons = top_left_buttons.push(modifiers);
                        }
                    }

                    let base = chart::view(chart, indicators, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::ChartInteraction(message))
                    });
                    let settings_modal = || {
                        kline_cfg_view(
                            chart.study_configurator(),
                            data::chart::kline::Config {},
                            chart_kind,
                            id,
                            chart.basis(),
                        )
                    };

                    let indicator_modal = if self.modal == Some(Modal::Indicators) {
                        Some(modal::indicators::view(
                            id,
                            self,
                            indicators,
                            self.stream_pair().map(|i| i.ticker.market_type()),
                        ))
                    } else {
                        None
                    };

                    self.compose_stack_view(
                        base,
                        id,
                        indicator_modal,
                        compact_controls,
                        settings_modal,
                        None,
                        tickers_table,
                    )
                } else {
                    let content_kind = match chart_kind {
                        data::chart::KlineChartKind::Candles => ContentKind::CandlestickChart,
                        data::chart::KlineChartKind::Footprint { .. } => {
                            ContentKind::FootprintChart
                        }
                    };
                    let base = uninitialized_base(content_kind);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::ShaderHeatmap {
                chart, indicators, ..
            } => {
                if let Some(chart) = chart {
                    let base = HeatmapShader::view(chart, timezone).map(move |message| {
                        Message::PaneEvent(id, Event::HeatmapShaderInteraction(message))
                    });

                    let ticker_info = self.stream_pair();
                    let exchange = ticker_info.as_ref().map(|info| info.ticker.exchange);

                    let basis = self
                        .settings
                        .selected_basis
                        .unwrap_or(Basis::default_heatmap_time(ticker_info));
                    let tick_multiply = self.settings.tick_multiply.unwrap_or(TickMultiplier(5));

                    let kind = ModifierKind::Heatmap(basis, tick_multiply);

                    let price_step = ticker_info
                        .map(|ti| {
                            tick_multiply
                                .unscale_step_or_min_tick(chart.tick_size(), ti.min_ticksize)
                        })
                        .unwrap_or_else(|| tick_multiply.unscale_step(chart.tick_size()));
                    let min_ticksize = ticker_info.map(|ti| ti.min_ticksize);

                    let settings_modal = || {
                        heatmap_shader_cfg_view(
                            chart.visual_config(),
                            id,
                            chart.study_configurator(),
                            &chart.studies,
                            basis,
                        )
                    };

                    let indicator_modal = if self.modal == Some(Modal::Indicators) {
                        Some(modal::indicators::view(
                            id,
                            self,
                            indicators,
                            self.stream_pair().map(|i| i.ticker.market_type()),
                        ))
                    } else {
                        None
                    };

                    let modifiers = row![
                        basis_modifier(id, basis, modifier, kind),
                        ticksize_modifier(
                            id,
                            price_step,
                            min_ticksize,
                            tick_multiply,
                            modifier,
                            kind,
                            exchange
                        ),
                    ]
                    .spacing(4);

                    top_left_buttons = top_left_buttons.push(modifiers);

                    self.compose_stack_view(
                        base,
                        id,
                        indicator_modal,
                        compact_controls,
                        settings_modal,
                        None,
                        tickers_table,
                    )
                } else {
                    let base = uninitialized_base(ContentKind::ShaderHeatmap);
                    self.compose_stack_view(
                        base,
                        id,
                        None,
                        compact_controls,
                        || column![].into(),
                        None,
                        tickers_table,
                    )
                }
            }
            Content::OrderEntry(panel) => {
                let base = panel.view(theme, is_replay).map(move |msg| {
                    Message::PaneEvent(id, Event::PanelInteraction(panel::Message::OrderEntry(msg)))
                });
                self.compose_stack_view(
                    base,
                    id,
                    None,
                    compact_controls,
                    || column![].into(),
                    None,
                    tickers_table,
                )
            }
            Content::OrderList(panel) => {
                let base = panel.view(theme).map(move |msg| {
                    Message::PaneEvent(id, Event::PanelInteraction(panel::Message::OrderList(msg)))
                });
                self.compose_stack_view(
                    base,
                    id,
                    None,
                    compact_controls,
                    || column![].into(),
                    None,
                    tickers_table,
                )
            }
            Content::BuyingPower(panel) => {
                let base = panel.view(theme).map(move |msg| {
                    Message::PaneEvent(
                        id,
                        Event::PanelInteraction(panel::Message::BuyingPower(msg)),
                    )
                });
                self.compose_stack_view(
                    base,
                    id,
                    None,
                    compact_controls,
                    || column![].into(),
                    None,
                    tickers_table,
                )
            }
        };

        match &self.status {
            Status::Loading(InfoKind::FetchingKlines) => {
                top_left_buttons = top_left_buttons.push(text("Fetching Klines..."));
            }
            Status::Loading(InfoKind::FetchingTrades(count)) => {
                top_left_buttons =
                    top_left_buttons.push(text(format!("Fetching Trades... {count} fetched")));
            }
            Status::Loading(InfoKind::FetchingOI) => {
                top_left_buttons = top_left_buttons.push(text("Fetching Open Interest..."));
            }
            Status::Stale(msg) => {
                top_left_buttons = top_left_buttons.push(text(msg));
            }
            Status::Ready => {}
        }

        // リプレイ中の Depth 系ペインに注意テキストを表示
        if is_replay
            && matches!(
                self.content,
                Content::Heatmap { .. } | Content::ShaderHeatmap { .. } | Content::Ladder(_)
            )
        {
            top_left_buttons = top_left_buttons.push(text("Replay: Depth unavailable").size(11));
        }

        let content = pane_grid::Content::new(body)
            .style(move |theme| style::pane_background(theme, is_focused));

        let top_right_buttons = {
            let compact_control = container(
                button(text("...").size(13).align_y(Alignment::End))
                    .on_press(Message::PaneEvent(id, Event::ShowModal(Modal::Controls)))
                    .style(move |theme, status| {
                        style::button::transparent(
                            theme,
                            status,
                            self.modal == Some(Modal::Controls)
                                || self.modal == Some(Modal::Settings),
                        )
                    }),
            )
            .align_y(Alignment::Center)
            .padding(4);

            if self.modal == Some(Modal::Controls) {
                pane_grid::Controls::new(compact_control)
            } else {
                pane_grid::Controls::dynamic(
                    self.view_controls(id, panes, maximized, window != main_window.id),
                    compact_control,
                )
            }
        };

        let title_bar = pane_grid::TitleBar::new(
            top_left_buttons
                .padding(padding::left(4))
                .align_y(Alignment::Center)
                .spacing(8)
                .height(Length::Fixed(32.0)),
        )
        .controls(top_right_buttons)
        .style(style::pane_title_bar);

        content.title_bar(if self.modal.is_none() {
            title_bar
        } else {
            title_bar.always_show_controls()
        })
    }

    pub fn update(&mut self, msg: Event) -> Option<Effect> {
        match msg {
            Event::ShowModal(requested_modal) => {
                return self.show_modal_with_focus(requested_modal);
            }
            Event::HideModal => {
                self.modal = None;
            }
            Event::ContentSelected(kind) => {
                self.content = Content::placeholder(kind);

                if !matches!(
                    kind,
                    ContentKind::Starter
                        | ContentKind::OrderEntry
                        | ContentKind::OrderList
                        | ContentKind::BuyingPower
                ) {
                    self.streams = ResolvedStream::waiting(vec![]);
                    let modal = Modal::MiniTickersList(MiniPanel::new());

                    if let Some(effect) = self.show_modal_with_focus(modal) {
                        return Some(effect);
                    }
                }

                if kind == ContentKind::BuyingPower {
                    return Some(Effect::FetchBuyingPower);
                }
            }
            Event::ChartInteraction(msg) => match &mut self.content {
                Content::Heatmap { chart: Some(c), .. } => {
                    super::chart::update(c, &msg);
                }
                Content::Kline { chart: Some(c), .. } => {
                    super::chart::update(c, &msg);
                }
                _ => {}
            },
            Event::PanelInteraction(msg) => match (&mut self.content, msg) {
                (Content::Ladder(Some(p)), msg) => super::panel::update(p, msg),
                (Content::TimeAndSales(Some(p)), msg) => super::panel::update(p, msg),
                (Content::OrderEntry(panel), panel::Message::OrderEntry(msg)) => {
                    let is_virtual = self.is_virtual_mode;
                    if let Some(action) = panel.update(msg) {
                        return match action {
                            panel::order_entry::Action::Submit(req) => {
                                if is_virtual {
                                    virtual_order_from_new_order_request(&req)
                                        .map(Effect::SubmitVirtualOrder)
                                } else {
                                    Some(Effect::SubmitNewOrder(*req))
                                }
                            }
                            panel::order_entry::Action::FetchHoldings { issue_code } => {
                                Some(Effect::FetchHoldings { issue_code })
                            }
                        };
                    }
                }
                (Content::OrderList(panel), panel::Message::OrderList(msg)) => {
                    if let Some(action) = panel.update(msg) {
                        return match action {
                            panel::order_list::Action::FetchOrders => Some(Effect::FetchOrders),
                            panel::order_list::Action::FetchOrderDetail { order_num, eig_day } => {
                                Some(Effect::FetchOrderDetail { order_num, eig_day })
                            }
                            panel::order_list::Action::SubmitCorrect(req) => {
                                Some(Effect::SubmitCorrectOrder(*req))
                            }
                            panel::order_list::Action::SubmitCancel(req) => {
                                Some(Effect::SubmitCancelOrder(*req))
                            }
                        };
                    }
                }
                (Content::BuyingPower(panel), panel::Message::BuyingPower(msg)) => {
                    if let Some(action) = panel.update(msg) {
                        return match action {
                            panel::buying_power::Action::FetchBuyingPower => {
                                Some(Effect::FetchBuyingPower)
                            }
                        };
                    }
                }
                _ => {}
            },
            Event::ToggleIndicator(ind) => {
                self.content.toggle_indicator(ind);
            }
            Event::DeleteNotification(idx) => {
                if idx < self.notifications.len() {
                    self.notifications.remove(idx);
                }
            }
            Event::ReorderIndicator(e) => {
                self.content.reorder_indicators(&e);
            }
            Event::ClusterKindSelected(kind) => {
                if let Content::Kline {
                    chart, kind: cur, ..
                } = &mut self.content
                    && let Some(c) = chart
                {
                    c.set_cluster_kind(kind);
                    *cur = c.kind.clone();
                }
            }
            Event::ClusterScalingSelected(scaling) => {
                if let Content::Kline { chart, kind, .. } = &mut self.content
                    && let Some(c) = chart
                {
                    c.set_cluster_scaling(scaling);
                    *kind = c.kind.clone();
                }
            }
            Event::StudyConfigurator(study_msg) => match study_msg {
                modal::pane::settings::study::StudyMessage::Footprint(m) => {
                    if let Content::Kline { chart, kind, .. } = &mut self.content
                        && let Some(c) = chart
                    {
                        c.update_study_configurator(m);
                        *kind = c.kind.clone();
                    }
                }
                modal::pane::settings::study::StudyMessage::Heatmap(m) => {
                    if let Content::Heatmap { chart, studies, .. } = &mut self.content
                        && let Some(c) = chart
                    {
                        c.update_study_configurator(m);
                        *studies = c.studies.clone();
                    } else if let Content::ShaderHeatmap { chart, studies, .. } = &mut self.content
                        && let Some(c) = chart
                    {
                        c.update_study_configurator(m);
                        *studies = c.studies.clone();
                    }
                }
            },
            Event::StreamModifierChanged(message) => {
                if let Some(Modal::StreamModifier(mut modifier)) = self.modal.take() {
                    let mut effect: Option<Effect> = None;

                    if let Some(action) = modifier.update(message) {
                        match action {
                            modal::stream::Action::TabSelected(tab) => {
                                modifier.tab = tab;
                            }
                            modal::stream::Action::TicksizeSelected(tm) => {
                                modifier.update_kind_with_multiplier(tm);
                                self.settings.tick_multiply = Some(tm);

                                if let Some(ticker) = self.stream_pair() {
                                    match &mut self.content {
                                        Content::Kline { chart: Some(c), .. } => {
                                            c.change_tick_size(
                                                tm.multiply_with_min_tick_step(ticker),
                                            );
                                            c.reset_request_handler();
                                        }
                                        Content::Heatmap { chart: Some(c), .. } => {
                                            c.change_tick_size(
                                                tm.multiply_with_min_tick_step(ticker),
                                            );
                                        }
                                        Content::Ladder(Some(p)) => {
                                            p.set_tick_size(tm.multiply_with_min_tick_step(ticker));
                                        }
                                        Content::ShaderHeatmap {
                                            chart: Some(c),
                                            indicators,
                                            studies,
                                            ..
                                        } => {
                                            **c = HeatmapShader::new(
                                                c.basis,
                                                tm.multiply_with_min_tick_step(ticker),
                                                c.ticker_info,
                                                studies.clone(),
                                                indicators.clone(),
                                            );
                                        }
                                        _ => {}
                                    }
                                }

                                let is_client = self
                                    .stream_pair()
                                    .map(|ti| ti.exchange().is_depth_client_aggr())
                                    .unwrap_or(false);

                                if let Some(mut it) = self.streams.ready_iter_mut() {
                                    for s in &mut it {
                                        if let StreamKind::Depth { depth_aggr, .. } = s {
                                            *depth_aggr = if is_client {
                                                StreamTicksize::Client
                                            } else {
                                                StreamTicksize::ServerSide(tm)
                                            };
                                        }
                                    }
                                }
                                if !is_client {
                                    effect = Some(Effect::RefreshStreams);
                                }
                            }
                            modal::stream::Action::BasisSelected(new_basis) => {
                                modifier.update_kind_with_basis(new_basis);
                                self.settings.selected_basis = Some(new_basis);

                                let base_ticker = self.stream_pair();

                                match &mut self.content {
                                    Content::Heatmap { chart: Some(c), .. } => {
                                        c.set_basis(new_basis);

                                        if let Some(stream_type) =
                                            self.streams.ready_iter_mut().and_then(|mut it| {
                                                it.find(|s| matches!(s, StreamKind::Depth { .. }))
                                            })
                                            && let StreamKind::Depth {
                                                push_freq,
                                                ticker_info,
                                                ..
                                            } = stream_type
                                            && ticker_info.exchange().is_custom_push_freq()
                                        {
                                            match new_basis {
                                                Basis::Time(tf) => {
                                                    *push_freq = exchange::PushFrequency::Custom(tf)
                                                }
                                                Basis::Tick(_) => {
                                                    *push_freq =
                                                        exchange::PushFrequency::ServerDefault
                                                }
                                            }
                                        }

                                        effect = Some(Effect::RefreshStreams);
                                    }
                                    Content::ShaderHeatmap {
                                        chart: Some(c),
                                        indicators,
                                        ..
                                    } => {
                                        **c = HeatmapShader::new(
                                            new_basis,
                                            c.tick_size(),
                                            c.ticker_info,
                                            c.studies.clone(),
                                            indicators.clone(),
                                        );

                                        if let Some(stream_type) =
                                            self.streams.ready_iter_mut().and_then(|mut it| {
                                                it.find(|s| matches!(s, StreamKind::Depth { .. }))
                                            })
                                            && let StreamKind::Depth {
                                                push_freq,
                                                ticker_info,
                                                ..
                                            } = stream_type
                                            && ticker_info.exchange().is_custom_push_freq()
                                        {
                                            match new_basis {
                                                Basis::Time(tf) => {
                                                    *push_freq = exchange::PushFrequency::Custom(tf)
                                                }
                                                Basis::Tick(_) => {
                                                    *push_freq =
                                                        exchange::PushFrequency::ServerDefault
                                                }
                                            }
                                        }

                                        effect = Some(Effect::RefreshStreams);
                                    }
                                    Content::Kline { chart: Some(c), .. } => {
                                        if let Some(base_ticker) = base_ticker {
                                            match new_basis {
                                                Basis::Time(tf) => {
                                                    let kline_stream = StreamKind::Kline {
                                                        ticker_info: base_ticker,
                                                        timeframe: tf,
                                                    };
                                                    let mut streams = vec![kline_stream];

                                                    if matches!(
                                                        c.kind,
                                                        data::chart::KlineChartKind::Footprint { .. }
                                                    ) {
                                                        let depth_aggr = if base_ticker
                                                            .exchange()
                                                            .is_depth_client_aggr()
                                                        {
                                                            StreamTicksize::Client
                                                        } else {
                                                            StreamTicksize::ServerSide(
                                                                self.settings
                                                                    .tick_multiply
                                                                    .unwrap_or(TickMultiplier(1)),
                                                            )
                                                        };
                                                        streams.push(StreamKind::Depth {
                                                            ticker_info: base_ticker,
                                                            depth_aggr,
                                                            push_freq: exchange::PushFrequency::ServerDefault,
                                                        });
                                                        streams.push(StreamKind::Trades {
                                                            ticker_info: base_ticker,
                                                        });
                                                    }

                                                    // リプレイ中は旧 kline stream を保存してから更新する
                                                    let old_kline_stream = self
                                                        .streams
                                                        .ready_iter()
                                                        .and_then(|mut it| {
                                                            it.find(|s| {
                                                                matches!(
                                                                    s,
                                                                    StreamKind::Kline { .. }
                                                                )
                                                            })
                                                        })
                                                        .copied();

                                                    self.streams = ResolvedStream::Ready(streams);
                                                    let action = c.set_basis(new_basis);

                                                    if c.is_replay_mode() {
                                                        // リプレイ中: コントローラに再ロードを依頼
                                                        effect = Some(Effect::ReloadReplayKlines {
                                                            old_stream: old_kline_stream,
                                                            new_stream: kline_stream,
                                                        });
                                                    } else if let Some(
                                                        chart::Action::RequestFetch(fetch),
                                                    ) = action
                                                    {
                                                        effect = Some(Effect::RequestFetch(fetch));
                                                    }
                                                }
                                                Basis::Tick(_) => {
                                                    let depth_aggr = if base_ticker
                                                        .exchange()
                                                        .is_depth_client_aggr()
                                                    {
                                                        StreamTicksize::Client
                                                    } else {
                                                        StreamTicksize::ServerSide(
                                                            self.settings
                                                                .tick_multiply
                                                                .unwrap_or(TickMultiplier(1)),
                                                        )
                                                    };

                                                    self.streams = ResolvedStream::Ready(vec![
                                                        StreamKind::Depth {
                                                            ticker_info: base_ticker,
                                                            depth_aggr,
                                                            push_freq: exchange::PushFrequency::ServerDefault,
                                                        },
                                                        StreamKind::Trades {
                                                            ticker_info: base_ticker,
                                                        },
                                                    ]);
                                                    c.set_basis(new_basis);
                                                    effect = Some(Effect::RefreshStreams);
                                                }
                                            }
                                        }
                                    }
                                    Content::Comparison(Some(c)) => {
                                        if let Basis::Time(tf) = new_basis {
                                            let streams: Vec<StreamKind> = c
                                                .selected_tickers()
                                                .iter()
                                                .copied()
                                                .map(|ti| StreamKind::Kline {
                                                    ticker_info: ti,
                                                    timeframe: tf,
                                                })
                                                .collect();

                                            self.streams = ResolvedStream::Ready(streams);
                                            let action = c.set_basis(new_basis);

                                            if let Some(chart::Action::RequestFetch(fetch)) = action
                                            {
                                                effect = Some(Effect::RequestFetch(fetch));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    self.modal = Some(Modal::StreamModifier(modifier));

                    if let Some(e) = effect {
                        return Some(e);
                    }
                }
            }
            Event::ComparisonChartInteraction(message) => {
                if let Content::Comparison(chart_opt) = &mut self.content
                    && let Some(chart) = chart_opt
                    && let Some(action) = chart.update(message)
                {
                    match action {
                        super::chart::comparison::Action::SeriesColorChanged(t, color) => {
                            chart.set_series_color(t, color);
                        }
                        super::chart::comparison::Action::SeriesNameChanged(t, name) => {
                            chart.set_series_name(t, name);
                        }
                        super::chart::comparison::Action::OpenSeriesEditor => {
                            self.modal = Some(Modal::Settings);
                        }
                        super::chart::comparison::Action::RemoveSeries(ti) => {
                            let rebuilt = chart.remove_ticker(&ti);
                            self.streams = ResolvedStream::Ready(rebuilt);

                            return Some(Effect::RefreshStreams);
                        }
                    }
                }
            }
            Event::HeatmapShaderInteraction(message) => {
                if let Content::ShaderHeatmap { chart: Some(c), .. } = &mut self.content {
                    c.update(message);
                }
            }
            Event::MiniTickersListInteraction(message) => {
                if let Some(Modal::MiniTickersList(ref mut mini_panel)) = self.modal
                    && let Some(action) = mini_panel.update(message)
                {
                    let crate::modal::pane::mini_tickers_list::Action::RowSelected(sel) = action;
                    match sel {
                        crate::modal::pane::mini_tickers_list::RowSelection::Add(ti) => {
                            if let Content::Comparison(chart) = &mut self.content
                                && let Some(c) = chart
                            {
                                let rebuilt = c.add_ticker(&ti);
                                self.streams = ResolvedStream::Ready(rebuilt);
                                return Some(Effect::RefreshStreams);
                            }
                        }
                        crate::modal::pane::mini_tickers_list::RowSelection::Remove(ti) => {
                            if let Content::Comparison(chart) = &mut self.content
                                && let Some(c) = chart
                            {
                                let rebuilt = c.remove_ticker(&ti);
                                self.streams = ResolvedStream::Ready(rebuilt);
                                return Some(Effect::RefreshStreams);
                            }
                        }
                        crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti) => {
                            return Some(Effect::SwitchTickersInGroup(ti));
                        }
                    }
                }
            }
        }
        None
    }

    fn view_controls(
        &'_ self,
        pane: pane_grid::Pane,
        total_panes: usize,
        is_maximized: bool,
        is_popout: bool,
    ) -> Element<'_, Message> {
        let modal_btn_style = |modal: Modal| {
            let is_active = self.modal == Some(modal);
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let control_btn_style = |is_active: bool| {
            move |theme: &Theme, status: button::Status| {
                style::button::transparent(theme, status, is_active)
            }
        };

        let treat_as_starter =
            matches!(&self.content, Content::Starter) || !self.content.initialized();

        let tooltip_pos = tooltip::Position::Bottom;
        let mut buttons = row![];

        let show_modal = |modal: Modal| Message::PaneEvent(pane, Event::ShowModal(modal));

        if !treat_as_starter {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Cog, 12),
                show_modal(Modal::Settings),
                None,
                tooltip_pos,
                modal_btn_style(Modal::Settings),
            ));
        }
        if !treat_as_starter
            && matches!(
                &self.content,
                Content::Heatmap { .. } | Content::Kline { .. } | Content::ShaderHeatmap { .. }
            )
        {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::ChartOutline, 12),
                show_modal(Modal::Indicators),
                Some("Indicators"),
                tooltip_pos,
                modal_btn_style(Modal::Indicators),
            ));
        }

        if is_popout {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Popout, 12),
                Message::Merge,
                Some("Merge"),
                tooltip_pos,
                control_btn_style(is_popout),
            ));
        } else if total_panes > 1 {
            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Popout, 12),
                Message::Popout,
                Some("Pop out"),
                tooltip_pos,
                control_btn_style(is_popout),
            ));
        }

        if total_panes > 1 {
            let (resize_icon, message) = if is_maximized {
                (Icon::ResizeSmall, Message::Restore)
            } else {
                (Icon::ResizeFull, Message::MaximizePane(pane))
            };

            buttons = buttons.push(button_with_tooltip(
                icon_text(resize_icon, 12),
                message,
                None,
                tooltip_pos,
                control_btn_style(is_maximized),
            ));

            buttons = buttons.push(button_with_tooltip(
                icon_text(Icon::Close, 12),
                Message::ClosePane(pane),
                None,
                tooltip_pos,
                control_btn_style(false),
            ));
        }

        buttons
            .padding(padding::right(4).left(4))
            .align_y(Alignment::Center)
            .height(Length::Fixed(32.0))
            .into()
    }

    fn compose_stack_view<'a, F>(
        &'a self,
        base: Element<'a, Message>,
        pane: pane_grid::Pane,
        indicator_modal: Option<Element<'a, Message>>,
        compact_controls: Option<Element<'a, Message>>,
        settings_modal: F,
        selected_tickers: Option<&'a [TickerInfo]>,
        tickers_table: &'a TickersTable,
    ) -> Element<'a, Message>
    where
        F: FnOnce() -> Element<'a, Message>,
    {
        let base =
            widget::toast::Manager::new(base, &self.notifications, Alignment::End, move |msg| {
                Message::PaneEvent(pane, Event::DeleteNotification(msg))
            })
            .into();

        let on_blur = Message::PaneEvent(pane, Event::HideModal);

        match &self.modal {
            Some(Modal::LinkGroup) => {
                let content = link_group_modal(pane, self.link_group);

                stack_modal(
                    base,
                    content,
                    on_blur,
                    padding::right(12).left(4),
                    Alignment::Start,
                )
            }
            Some(Modal::StreamModifier(modifier)) => stack_modal(
                base,
                modifier.view(self.stream_pair_kind()).map(move |message| {
                    Message::PaneEvent(pane, Event::StreamModifierChanged(message))
                }),
                Message::PaneEvent(pane, Event::HideModal),
                padding::right(12).left(48),
                Alignment::Start,
            ),
            Some(Modal::MiniTickersList(panel)) => {
                let mini_list = panel
                    .view(tickers_table, selected_tickers, self.stream_pair())
                    .map(move |msg| {
                        Message::PaneEvent(pane, Event::MiniTickersListInteraction(msg))
                    });

                let content: Element<_> = container(mini_list)
                    .max_width(260)
                    .padding(16)
                    .style(style::chart_modal)
                    .into();

                stack_modal(
                    base,
                    content,
                    Message::PaneEvent(pane, Event::HideModal),
                    padding::left(12),
                    Alignment::Start,
                )
            }
            Some(Modal::Settings) => stack_modal(
                base,
                settings_modal(),
                on_blur,
                padding::right(12).left(12),
                Alignment::End,
            ),
            Some(Modal::Indicators) => stack_modal(
                base,
                indicator_modal.unwrap_or_else(|| column![].into()),
                on_blur,
                padding::right(12).left(12),
                Alignment::End,
            ),
            Some(Modal::Controls) => stack_modal(
                base,
                if let Some(controls) = compact_controls {
                    controls
                } else {
                    column![].into()
                },
                on_blur,
                padding::left(12),
                Alignment::End,
            ),
            None => base,
        }
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        self.streams.matches_stream(stream)
    }

    fn show_modal_with_focus(&mut self, requested_modal: Modal) -> Option<Effect> {
        let should_toggle_close = match (&self.modal, &requested_modal) {
            (Some(Modal::StreamModifier(open)), Modal::StreamModifier(req)) => {
                open.view_mode == req.view_mode
            }
            (Some(open), req) => core::mem::discriminant(open) == core::mem::discriminant(req),
            _ => false,
        };

        if should_toggle_close {
            self.modal = None;
            return None;
        }

        let focus_widget_id = match &requested_modal {
            Modal::MiniTickersList(m) => Some(m.search_box_id.clone()),
            _ => None,
        };

        self.modal = Some(requested_modal);
        focus_widget_id.map(Effect::FocusWidget)
    }

    pub fn invalidate(&mut self, now: Instant) -> Option<Action> {
        match &mut self.content {
            Content::Heatmap { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::Kline { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::TimeAndSales(panel) => panel
                .as_mut()
                .and_then(|p| p.invalidate(Some(now)).map(Action::Panel)),
            Content::Ladder(panel) => panel
                .as_mut()
                .and_then(|p| p.invalidate(Some(now)).map(Action::Panel)),
            Content::Starter => None,
            Content::Comparison(chart) => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::ShaderHeatmap { chart, .. } => chart
                .as_mut()
                .and_then(|c| c.invalidate(Some(now)).map(Action::Chart)),
            Content::OrderEntry(_) | Content::OrderList(_) | Content::BuyingPower(_) => None,
        }
    }

    pub fn park_for_inactive_layout(&mut self) {
        if let Content::ShaderHeatmap { chart, .. } = &mut self.content {
            *chart = None;
            self.status = Status::Ready;
        }
    }

    /// リプレイ用: EventStore から得た klines をこのペインの kline chart に注入する。
    pub fn ingest_replay_klines(&mut self, klines: &[Kline]) {
        if let Content::Kline { chart: Some(c), .. } = &mut self.content {
            c.set_replay_mode(true);
            c.ingest_historical_klines(klines);
        }
    }

    /// リプレイ seek 時: kline chart のデータをリセットする。
    /// replay_mode=true を保持することで fetch_missing_data の live fetch を抑制する。
    pub fn reset_for_seek(&mut self) {
        if let Content::Kline { chart: Some(c), .. } = &mut self.content {
            c.set_replay_mode(true);
            c.reset_for_seek();
        }
    }

    pub fn update_interval(&self) -> Option<u64> {
        match &self.content {
            Content::Kline { .. } | Content::Comparison(_) => Some(1000),
            Content::Heatmap { chart, .. } => {
                if let Some(chart) = chart {
                    chart.basis_interval()
                } else {
                    None
                }
            }
            Content::Ladder(_) | Content::TimeAndSales(_) => Some(100),
            Content::ShaderHeatmap { .. } => None,
            Content::Starter => None,
            Content::OrderEntry(_) | Content::OrderList(_) | Content::BuyingPower(_) => None,
        }
    }

    pub fn last_tick(&self) -> Option<Instant> {
        self.content.last_tick()
    }

    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        let invalidate_interval: Option<u64> = self.update_interval();
        let last_tick: Option<Instant> = self.last_tick();

        if let Some(streams) = self.streams.due_streams_to_resolve(now) {
            return Some(Action::ResolveStreams(streams));
        }

        if !self.content.initialized() {
            return Some(Action::ResolveContent);
        }

        match (invalidate_interval, last_tick) {
            (Some(interval_ms), Some(previous_tick_time)) => {
                if interval_ms > 0 {
                    let interval_duration = std::time::Duration::from_millis(interval_ms);
                    if now.duration_since(previous_tick_time) >= interval_duration {
                        return self.invalidate(now);
                    }
                }
            }
            (Some(interval_ms), None) => {
                if interval_ms > 0 {
                    return self.invalidate(now);
                }
            }
            (None, _) => {
                return self.invalidate(now);
            }
        }

        None
    }

    pub fn unique_id(&self) -> uuid::Uuid {
        self.id
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            modal: None,
            content: Content::Starter,
            settings: Settings::default(),
            streams: ResolvedStream::waiting(vec![]),
            notifications: vec![],
            status: Status::Ready,
            link_group: None,
            is_virtual_mode: false,
        }
    }
}

fn by_basis_default<T>(
    basis: Option<Basis>,
    default_tf: Timeframe,
    on_time: impl FnOnce(Timeframe) -> T,
    on_tick: impl FnOnce() -> T,
) -> T {
    match basis.unwrap_or(Basis::Time(default_tf)) {
        Basis::Time(tf) => on_time(tf),
        Basis::Tick(_) => on_tick(),
    }
}

fn virtual_order_from_new_order_request(
    req: &exchange::adapter::tachibana::NewOrderRequest,
) -> Option<crate::replay::virtual_exchange::VirtualOrder> {
    use crate::replay::virtual_exchange::{
        PositionSide, VirtualOrder, VirtualOrderStatus, VirtualOrderType,
    };

    // tachibana API: side "3" = 買い, "1" = 売り
    let side = match req.side.as_str() {
        "3" => PositionSide::Long,
        "1" => PositionSide::Short,
        unknown => {
            log::warn!("仮想注文: 未知の side コード ({unknown:?}) — 注文を破棄");
            return None;
        }
    };

    let order_type = if req.price == "0" {
        VirtualOrderType::Market
    } else {
        let Ok(price) = req.price.parse::<f64>() else {
            log::warn!(
                "仮想注文: 指値価格のパース失敗 ({:?}) — 注文を破棄",
                req.price
            );
            return None;
        };
        VirtualOrderType::Limit { price }
    };

    let Ok(qty) = req.qty.parse::<f64>() else {
        log::warn!("仮想注文: 数量のパース失敗 ({:?}) — 注文を破棄", req.qty);
        return None;
    };

    Some(VirtualOrder {
        order_id: uuid::Uuid::new_v4().to_string(),
        ticker: req.issue_code.clone(),
        side,
        qty,
        order_type,
        placed_time_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is before UNIX epoch")
            .as_millis() as u64,
        status: VirtualOrderStatus::Pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注文ペインで ContentSelected を受け取っても MiniTickersList モーダルを開かないこと
    #[test]
    fn content_selected_order_entry_does_not_open_ticker_modal() {
        let mut state = State::new();
        let effect = state.update(Event::ContentSelected(ContentKind::OrderEntry));
        assert!(
            effect.is_none(),
            "OrderEntry ContentSelected should not return an effect"
        );
        assert!(
            state.modal.is_none(),
            "OrderEntry ContentSelected should not open a modal"
        );
    }

    #[test]
    fn content_selected_order_list_does_not_open_ticker_modal() {
        let mut state = State::new();
        let effect = state.update(Event::ContentSelected(ContentKind::OrderList));
        assert!(effect.is_none());
        assert!(state.modal.is_none());
    }

    #[test]
    fn content_selected_buying_power_does_not_open_ticker_modal() {
        let mut state = State::new();
        let effect = state.update(Event::ContentSelected(ContentKind::BuyingPower));
        assert!(
            matches!(effect, Some(Effect::FetchBuyingPower)),
            "BuyingPower selection should return FetchBuyingPower effect"
        );
        assert!(state.modal.is_none());
    }

    /// 非注文ペインは引き続き MiniTickersList モーダルを開くこと（リグレッション確認）
    #[test]
    fn content_selected_kline_opens_ticker_modal() {
        let mut state = State::new();
        let _effect = state.update(Event::ContentSelected(ContentKind::CandlestickChart));
        assert!(
            state.modal.is_some(),
            "CandlestickChart ContentSelected should open a modal"
        );
        assert!(matches!(state.modal, Some(Modal::MiniTickersList(_))));
    }
}
