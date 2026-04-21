use super::controls::{basis_modifier, link_group_modal, ticksize_modifier};
use super::{Content, Event, Message, State, Status};
use crate::chart::{comparison::ComparisonChart, heatmap::HeatmapChart, kline::KlineChart};
use crate::screen::dashboard::panel::{
    buying_power::BuyingPowerPanel, ladder::Ladder, order_entry::OrderEntryPanel,
    order_list::OrderListPanel, timeandsales::TimeAndSales,
};
use crate::{
    chart,
    connector::fetcher::InfoKind,
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
    screen::dashboard::{panel, tickers_table::TickersTable},
    style::{self, Icon, icon_text},
    widget::{self, button_with_tooltip, chart::heatmap::HeatmapShader, link_group_button},
    window::{self, Window},
};
use data::{
    UserTimezone,
    chart::{
        Basis, KlineChartKind,
        indicator::{HeatmapIndicator, KlineIndicator},
    },
    layout::pane::ContentKind,
};
use exchange::{StreamPairKind, TickMultiplier, adapter::MarketKind};
use iced::{
    Alignment, Element, Length, Renderer, Theme, padding,
    widget::{button, center, column, container, pane_grid, pick_list, row, text, tooltip},
};

pub(super) fn render_pane<'a>(
    state: &'a State,
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
    let is_ticker_modal_active = matches!(state.modal, Some(Modal::MiniTickersList(_)));

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

    let mut top_left_buttons = build_top_left_buttons(state, id, &mini_tickers_btn);

    let modifier: Option<modal::stream::Modifier> = state.modal.as_ref().and_then(|m| {
        if let Modal::StreamModifier(modifier) = m {
            Some(*modifier)
        } else {
            None
        }
    });

    let compact_controls = if state.modal == Some(Modal::Controls) {
        Some(
            container(view_controls(
                state,
                id,
                panes,
                maximized,
                window != main_window.id,
            ))
            .style(style::chart_modal)
            .into(),
        )
    } else {
        None
    };

    let has_stream = state.has_stream();

    let (body, content_modifiers) = render_body(
        state,
        id,
        timezone,
        tickers_table,
        modifier,
        compact_controls,
        theme,
        is_replay,
        has_stream,
    );

    if let Some(m) = content_modifiers {
        top_left_buttons = top_left_buttons.push(m);
    }

    match &state.status {
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

    if is_replay
        && matches!(
            state.content,
            Content::Heatmap { .. } | Content::ShaderHeatmap { .. } | Content::Ladder(_)
        )
    {
        top_left_buttons = top_left_buttons.push(text("Replay: Depth unavailable").size(11));
    }

    let pane_content =
        pane_grid::Content::new(body).style(move |theme| style::pane_background(theme, is_focused));

    let top_right_buttons = {
        let compact_control = container(
            button(text("...").size(13).align_y(Alignment::End))
                .on_press(Message::PaneEvent(id, Event::ShowModal(Modal::Controls)))
                .style(move |theme, status| {
                    style::button::transparent(
                        theme,
                        status,
                        state.modal == Some(Modal::Controls)
                            || state.modal == Some(Modal::Settings),
                    )
                }),
        )
        .align_y(Alignment::Center)
        .padding(4);

        if state.modal == Some(Modal::Controls) {
            pane_grid::Controls::new(compact_control)
        } else {
            pane_grid::Controls::dynamic(
                view_controls(state, id, panes, maximized, window != main_window.id),
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

    pane_content.title_bar(if state.modal.is_none() {
        title_bar
    } else {
        title_bar.always_show_controls()
    })
}

fn build_top_left_buttons<'a>(
    state: &'a State,
    id: pane_grid::Pane,
    mini_tickers_btn: &impl Fn(Element<'a, Message>) -> button::Button<'a, Message, Theme, Renderer>,
) -> iced::widget::Row<'a, Message, Theme, Renderer> {
    let mut buttons = if Content::Starter == state.content {
        row![]
    } else {
        row![link_group_button(id, state.link_group, |id| {
            Message::PaneEvent(id, Event::ShowModal(Modal::LinkGroup))
        })]
    };

    if let Some(kind) = state.stream_pair_kind() {
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

        buttons = buttons.push(mini_tickers_btn(content.into()));
    } else if !matches!(
        state.content,
        Content::Starter | Content::OrderEntry(_) | Content::OrderList(_) | Content::BuyingPower(_)
    ) && !state.has_stream()
    {
        let content = row![
            text("Choose a ticker")
                .size(13)
                .align_y(Alignment::Center)
                .line_height(1.4)
        ]
        .align_y(Alignment::Center);

        buttons = buttons.push(mini_tickers_btn(content.into()));
    }

    match &state.content {
        Content::OrderEntry(_) => {
            buttons = buttons.push(
                text("注文入力")
                    .size(13)
                    .align_y(Alignment::Center)
                    .line_height(1.4),
            );
        }
        Content::OrderList(_) => {
            buttons = buttons.push(
                text("注文一覧")
                    .size(13)
                    .align_y(Alignment::Center)
                    .line_height(1.4),
            );
        }
        Content::BuyingPower(_) => {
            buttons = buttons.push(
                text("買付余力")
                    .size(13)
                    .align_y(Alignment::Center)
                    .line_height(1.4),
            );
        }
        _ => {}
    }

    buttons
}

fn render_body<'a>(
    state: &'a State,
    id: pane_grid::Pane,
    timezone: UserTimezone,
    tickers_table: &'a TickersTable,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    theme: &'a Theme,
    is_replay: bool,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    match &state.content {
        Content::Starter => {
            let body = render_starter(state, id, compact_controls, tickers_table);
            (body, None)
        }
        Content::Comparison(chart) => render_comparison(
            state,
            chart,
            id,
            timezone,
            modifier,
            compact_controls,
            tickers_table,
            has_stream,
        ),
        Content::TimeAndSales(panel) => {
            let body = render_timesales(
                state,
                panel,
                id,
                timezone,
                compact_controls,
                tickers_table,
                has_stream,
            );
            (body, None)
        }
        Content::Ladder(panel) => render_ladder(
            state,
            panel,
            id,
            timezone,
            modifier,
            compact_controls,
            tickers_table,
            has_stream,
        ),
        Content::Heatmap {
            chart, indicators, ..
        } => render_heatmap(
            state,
            chart,
            indicators,
            id,
            timezone,
            modifier,
            compact_controls,
            tickers_table,
            has_stream,
        ),
        Content::Kline {
            chart,
            indicators,
            kind: chart_kind,
            ..
        } => render_kline(
            state,
            chart,
            indicators,
            chart_kind,
            id,
            timezone,
            modifier,
            compact_controls,
            tickers_table,
            has_stream,
        ),
        Content::ShaderHeatmap {
            chart, indicators, ..
        } => render_shader_heatmap(
            state,
            chart,
            indicators,
            id,
            timezone,
            modifier,
            compact_controls,
            tickers_table,
            has_stream,
        ),
        Content::OrderEntry(panel) => {
            let body = render_order_entry(
                state,
                panel,
                id,
                theme,
                is_replay,
                compact_controls,
                tickers_table,
            );
            (body, None)
        }
        Content::OrderList(panel) => {
            let body = render_order_list(state, panel, id, theme, compact_controls, tickers_table);
            (body, None)
        }
        Content::BuyingPower(panel) => {
            let body =
                render_buying_power(state, panel, id, theme, compact_controls, tickers_table);
            (body, None)
        }
    }
}

fn uninitialized_base<'a>(kind: ContentKind, has_stream: bool) -> Element<'a, Message> {
    if has_stream {
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
}

fn render_starter<'a>(
    state: &'a State,
    id: pane_grid::Pane,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message> {
    let content_picklist = pick_list(ContentKind::ALL, Some(ContentKind::Starter), move |kind| {
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
        &state.notifications,
        Alignment::End,
        move |msg| Message::PaneEvent(id, Event::DeleteNotification(msg)),
    )
    .into();

    compose_stack_view(
        state,
        base,
        id,
        None,
        compact_controls,
        || column![].into(),
        None,
        tickers_table,
    )
}

fn render_comparison<'a>(
    state: &'a State,
    chart: &'a Option<ComparisonChart>,
    id: pane_grid::Pane,
    timezone: UserTimezone,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    if let Some(c) = chart {
        let selected_basis = Basis::Time(c.timeframe);
        let kind = ModifierKind::Comparison(selected_basis);
        let modifiers = row![basis_modifier(id, selected_basis, modifier, kind),].spacing(4);

        let base = c
            .view(timezone)
            .map(move |message| Message::PaneEvent(id, Event::ComparisonChartInteraction(message)));

        let settings_modal = || comparison_cfg_view(id, c);

        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            settings_modal,
            Some(c.selected_tickers()),
            tickers_table,
        );
        (body, Some(modifiers.into()))
    } else {
        let base = uninitialized_base(ContentKind::ComparisonChart, has_stream);
        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            || column![].into(),
            None,
            tickers_table,
        );
        (body, None)
    }
}

fn render_timesales<'a>(
    state: &'a State,
    panel: &'a Option<TimeAndSales>,
    id: pane_grid::Pane,
    timezone: UserTimezone,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> Element<'a, Message> {
    if let Some(panel) = panel {
        let base = panel::view(panel, timezone)
            .map(move |message| Message::PaneEvent(id, Event::PanelInteraction(message)));
        let settings_modal = || modal::pane::settings::timesales_cfg_view(panel.config, id);
        compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            settings_modal,
            None,
            tickers_table,
        )
    } else {
        let base = uninitialized_base(ContentKind::TimeAndSales, has_stream);
        compose_stack_view(
            state,
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

fn render_ladder<'a>(
    state: &'a State,
    panel: &'a Option<Ladder>,
    id: pane_grid::Pane,
    timezone: UserTimezone,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    if let Some(panel) = panel {
        let basis = state
            .settings
            .selected_basis
            .unwrap_or(Basis::default_heatmap_time(state.stream_pair()));
        let tick_multiply = state.settings.tick_multiply.unwrap_or(TickMultiplier(1));

        let stream_pair = state.stream_pair();
        let price_step = stream_pair
            .map(|ti| tick_multiply.unscale_step_or_min_tick(panel.step, ti.min_ticksize))
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

        let base = panel::view(panel, timezone)
            .map(move |message| Message::PaneEvent(id, Event::PanelInteraction(message)));
        let settings_modal = || modal::pane::settings::ladder_cfg_view(panel.config, id);

        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            settings_modal,
            None,
            tickers_table,
        );
        (body, Some(modifiers))
    } else {
        let base = uninitialized_base(ContentKind::Ladder, has_stream);
        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            || column![].into(),
            None,
            tickers_table,
        );
        (body, None)
    }
}

fn build_heatmap_modifiers<'a>(
    state: &State,
    id: pane_grid::Pane,
    tick_size: exchange::unit::PriceStep,
    modifier: Option<modal::stream::Modifier>,
) -> (Basis, Element<'a, Message>) {
    let ticker_info = state.stream_pair();
    let exchange = ticker_info.as_ref().map(|info| info.ticker.exchange);
    let basis = state
        .settings
        .selected_basis
        .unwrap_or(Basis::default_heatmap_time(ticker_info));
    let tick_multiply = state.settings.tick_multiply.unwrap_or(TickMultiplier(5));
    let kind = ModifierKind::Heatmap(basis, tick_multiply);
    let price_step = ticker_info
        .map(|ti| tick_multiply.unscale_step_or_min_tick(tick_size, ti.min_ticksize))
        .unwrap_or_else(|| tick_multiply.unscale_step(tick_size));
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

    (basis, modifiers.into())
}

fn build_heatmap_indicator_modal<'a>(
    state: &'a State,
    id: pane_grid::Pane,
    indicators: &'a [HeatmapIndicator],
) -> Option<Element<'a, Message>> {
    if state.modal == Some(Modal::Indicators) {
        Some(modal::indicators::view(
            id,
            state,
            indicators,
            state.stream_pair().map(|i| i.ticker.market_type()),
        ))
    } else {
        None
    }
}

fn render_heatmap<'a>(
    state: &'a State,
    chart: &'a Option<HeatmapChart>,
    indicators: &'a [HeatmapIndicator],
    id: pane_grid::Pane,
    timezone: UserTimezone,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    if let Some(chart) = chart {
        let (basis, modifiers) = build_heatmap_modifiers(state, id, chart.tick_size(), modifier);

        let base = chart::view(chart, indicators, timezone)
            .map(move |message| Message::PaneEvent(id, Event::ChartInteraction(message)));
        let settings_modal = || {
            heatmap_cfg_view(
                chart.visual_config(),
                id,
                chart.study_configurator(),
                &chart.studies,
                basis,
            )
        };

        let indicator_modal = build_heatmap_indicator_modal(state, id, indicators);

        let body = compose_stack_view(
            state,
            base,
            id,
            indicator_modal,
            compact_controls,
            settings_modal,
            None,
            tickers_table,
        );
        (body, Some(modifiers))
    } else {
        let base = uninitialized_base(ContentKind::HeatmapChart, has_stream);
        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            || column![].into(),
            None,
            tickers_table,
        );
        (body, None)
    }
}

fn render_kline<'a>(
    state: &'a State,
    chart: &'a Option<KlineChart>,
    indicators: &'a [KlineIndicator],
    chart_kind: &'a KlineChartKind,
    id: pane_grid::Pane,
    timezone: UserTimezone,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    if let Some(chart) = chart {
        let kline_modifiers = match chart_kind {
            KlineChartKind::Footprint { .. } => {
                let basis = chart.basis();
                let tick_multiply = state.settings.tick_multiply.unwrap_or(TickMultiplier(10));
                let kind = ModifierKind::Footprint(basis, tick_multiply);
                let stream_pair = state.stream_pair();
                let price_step = stream_pair
                    .map(|ti| {
                        tick_multiply.unscale_step_or_min_tick(chart.tick_size(), ti.min_ticksize)
                    })
                    .unwrap_or_else(|| tick_multiply.unscale_step(chart.tick_size()));
                let exchange = stream_pair.as_ref().map(|info| info.ticker.exchange);
                let min_ticksize = stream_pair.map(|ti| ti.min_ticksize);

                row![
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
                .spacing(4)
            }
            KlineChartKind::Candles => {
                let selected_basis = chart.basis();
                let kind = ModifierKind::Candlestick(selected_basis);
                row![basis_modifier(id, selected_basis, modifier, kind),].spacing(4)
            }
        };

        let base = chart::view(chart, indicators, timezone)
            .map(move |message| Message::PaneEvent(id, Event::ChartInteraction(message)));
        let settings_modal = || {
            kline_cfg_view(
                chart.study_configurator(),
                data::chart::kline::Config {},
                chart_kind,
                id,
                chart.basis(),
            )
        };

        let indicator_modal = if state.modal == Some(Modal::Indicators) {
            Some(modal::indicators::view(
                id,
                state,
                indicators,
                state.stream_pair().map(|i| i.ticker.market_type()),
            ))
        } else {
            None
        };

        let body = compose_stack_view(
            state,
            base,
            id,
            indicator_modal,
            compact_controls,
            settings_modal,
            None,
            tickers_table,
        );
        (body, Some(kline_modifiers.into()))
    } else {
        let content_kind = match chart_kind {
            KlineChartKind::Candles => ContentKind::CandlestickChart,
            KlineChartKind::Footprint { .. } => ContentKind::FootprintChart,
        };
        let base = uninitialized_base(content_kind, has_stream);
        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            || column![].into(),
            None,
            tickers_table,
        );
        (body, None)
    }
}

fn render_shader_heatmap<'a>(
    state: &'a State,
    chart: &'a Option<Box<HeatmapShader>>,
    indicators: &'a [HeatmapIndicator],
    id: pane_grid::Pane,
    timezone: UserTimezone,
    modifier: Option<modal::stream::Modifier>,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
    has_stream: bool,
) -> (Element<'a, Message>, Option<Element<'a, Message>>) {
    if let Some(chart) = chart {
        let (basis, modifiers) = build_heatmap_modifiers(state, id, chart.tick_size(), modifier);

        let settings_modal = || {
            heatmap_shader_cfg_view(
                chart.visual_config(),
                id,
                chart.study_configurator(),
                &chart.studies,
                basis,
            )
        };

        let indicator_modal = build_heatmap_indicator_modal(state, id, indicators);

        let base = HeatmapShader::view(chart, timezone)
            .map(move |message| Message::PaneEvent(id, Event::HeatmapShaderInteraction(message)));

        let body = compose_stack_view(
            state,
            base,
            id,
            indicator_modal,
            compact_controls,
            settings_modal,
            None,
            tickers_table,
        );
        (body, Some(modifiers))
    } else {
        let base = uninitialized_base(ContentKind::ShaderHeatmap, has_stream);
        let body = compose_stack_view(
            state,
            base,
            id,
            None,
            compact_controls,
            || column![].into(),
            None,
            tickers_table,
        );
        (body, None)
    }
}

fn render_order_entry<'a>(
    state: &'a State,
    panel: &'a OrderEntryPanel,
    id: pane_grid::Pane,
    theme: &'a Theme,
    is_replay: bool,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message> {
    let base = panel.view(theme, is_replay).map(move |msg| {
        Message::PaneEvent(id, Event::PanelInteraction(panel::Message::OrderEntry(msg)))
    });

    let composed = compose_stack_view(
        state,
        base,
        id,
        None,
        compact_controls,
        || column![].into(),
        None,
        tickers_table,
    );

    if let Some(mini_panel) = &panel.modal {
        let mini_list = mini_panel.view(tickers_table, None, None).map(move |msg| {
            Message::PaneEvent(
                id,
                Event::PanelInteraction(panel::Message::OrderEntry(
                    super::super::panel::order_entry::Message::MiniTickers(msg),
                )),
            )
        });
        let overlay: Element<_> = container(mini_list)
            .max_width(260)
            .padding(16)
            .style(crate::style::chart_modal)
            .into();
        crate::modal::pane::stack_modal(
            composed,
            overlay,
            Message::PaneEvent(id, Event::HideModal),
            iced::padding::left(12),
            iced::Alignment::Start,
        )
    } else {
        composed
    }
}

fn render_order_list<'a>(
    state: &'a State,
    panel: &'a OrderListPanel,
    id: pane_grid::Pane,
    theme: &'a Theme,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message> {
    let base = panel.view(theme).map(move |msg| {
        Message::PaneEvent(id, Event::PanelInteraction(panel::Message::OrderList(msg)))
    });
    compose_stack_view(
        state,
        base,
        id,
        None,
        compact_controls,
        || column![].into(),
        None,
        tickers_table,
    )
}

fn render_buying_power<'a>(
    state: &'a State,
    panel: &'a BuyingPowerPanel,
    id: pane_grid::Pane,
    theme: &'a Theme,
    compact_controls: Option<Element<'a, Message>>,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message> {
    let base = panel.view(theme).map(move |msg| {
        Message::PaneEvent(
            id,
            Event::PanelInteraction(panel::Message::BuyingPower(msg)),
        )
    });
    compose_stack_view(
        state,
        base,
        id,
        None,
        compact_controls,
        || column![].into(),
        None,
        tickers_table,
    )
}

fn view_controls<'a>(
    state: &'a State,
    pane: pane_grid::Pane,
    total_panes: usize,
    is_maximized: bool,
    is_popout: bool,
) -> Element<'a, Message> {
    let modal_btn_style = |modal: Modal| {
        let is_active = state.modal == Some(modal);
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
        matches!(&state.content, Content::Starter) || !state.content.initialized();

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
            &state.content,
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
    state: &'a State,
    base: Element<'a, Message>,
    pane: pane_grid::Pane,
    indicator_modal: Option<Element<'a, Message>>,
    compact_controls: Option<Element<'a, Message>>,
    settings_modal: F,
    selected_tickers: Option<&'a [exchange::TickerInfo]>,
    tickers_table: &'a TickersTable,
) -> Element<'a, Message>
where
    F: FnOnce() -> Element<'a, Message>,
{
    let base =
        widget::toast::Manager::new(base, &state.notifications, Alignment::End, move |msg| {
            Message::PaneEvent(pane, Event::DeleteNotification(msg))
        })
        .into();

    let on_blur = Message::PaneEvent(pane, Event::HideModal);

    match &state.modal {
        Some(Modal::LinkGroup) => {
            let content = link_group_modal(pane, state.link_group);
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
            modifier.view(state.stream_pair_kind()).map(move |message| {
                Message::PaneEvent(pane, Event::StreamModifierChanged(message))
            }),
            Message::PaneEvent(pane, Event::HideModal),
            padding::right(12).left(48),
            Alignment::Start,
        ),
        Some(Modal::MiniTickersList(panel)) => {
            let mini_list = panel
                .view(tickers_table, selected_tickers, state.stream_pair())
                .map(move |msg| Message::PaneEvent(pane, Event::MiniTickersListInteraction(msg)));

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
