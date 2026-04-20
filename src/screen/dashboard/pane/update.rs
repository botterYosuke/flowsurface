use super::{Content, Effect, Event, State};
use crate::chart::kline::KlineChart;
use crate::connector::ResolvedStream;
use crate::widget::chart::heatmap::HeatmapShader;
use crate::{
    chart,
    modal::{self, pane::Modal},
    screen::dashboard::panel,
};
use data::chart::Basis;
use exchange::{
    TickMultiplier, TickerInfo,
    adapter::{StreamKind, StreamTicksize},
};

pub(super) fn dispatch(state: &mut State, msg: Event) -> Option<Effect> {
    match msg {
        Event::ShowModal(requested_modal) => {
            return state.show_modal_with_focus(requested_modal);
        }
        Event::HideModal => {
            state.modal = None;
        }
        Event::ContentSelected(kind) => {
            return handle_content_selected(state, kind);
        }
        Event::ChartInteraction(msg) => match &mut state.content {
            Content::Heatmap { chart: Some(c), .. } => {
                super::chart::update(c, &msg);
            }
            Content::Kline { chart: Some(c), .. } => {
                super::chart::update(c, &msg);
            }
            _ => {}
        },
        Event::PanelInteraction(msg) => return handle_panel_interaction(state, msg),
        Event::ToggleIndicator(ind) => {
            state.content.toggle_indicator(ind);
        }
        Event::DeleteNotification(idx) => {
            if idx < state.notifications.len() {
                state.notifications.remove(idx);
            }
        }
        Event::ReorderIndicator(e) => {
            state.content.reorder_indicators(&e);
        }
        Event::ClusterKindSelected(kind) => {
            if let Content::Kline {
                chart, kind: cur, ..
            } = &mut state.content
                && let Some(c) = chart
            {
                c.set_cluster_kind(kind);
                *cur = c.kind.clone();
            }
        }
        Event::ClusterScalingSelected(scaling) => {
            if let Content::Kline { chart, kind, .. } = &mut state.content
                && let Some(c) = chart
            {
                c.set_cluster_scaling(scaling);
                *kind = c.kind.clone();
            }
        }
        Event::StudyConfigurator(study_msg) => return handle_study_configurator(state, study_msg),
        Event::StreamModifierChanged(message) => {
            return handle_stream_modifier_changed(state, message);
        }
        Event::ComparisonChartInteraction(message) => {
            return handle_comparison_chart_interaction(state, message);
        }
        Event::HeatmapShaderInteraction(message) => {
            if let Content::ShaderHeatmap { chart: Some(c), .. } = &mut state.content {
                c.update(message);
            }
        }
        Event::MiniTickersListInteraction(message) => {
            return handle_mini_tickers_list(state, message);
        }
    }
    None
}

fn handle_content_selected(
    state: &mut State,
    kind: data::layout::pane::ContentKind,
) -> Option<Effect> {
    use crate::modal::pane::mini_tickers_list::MiniPanel;
    use data::layout::pane::ContentKind;

    state.content = Content::placeholder(kind);

    if !matches!(
        kind,
        ContentKind::Starter
            | ContentKind::OrderEntry
            | ContentKind::OrderList
            | ContentKind::BuyingPower
    ) {
        state.streams = ResolvedStream::waiting(vec![]);
        let modal = Modal::MiniTickersList(MiniPanel::new());

        if let Some(effect) = state.show_modal_with_focus(modal) {
            return Some(effect);
        }
    }

    if kind == ContentKind::BuyingPower {
        return Some(Effect::FetchBuyingPower);
    }

    None
}

fn handle_panel_interaction(state: &mut State, msg: panel::Message) -> Option<Effect> {
    match (&mut state.content, msg) {
        (Content::Ladder(Some(p)), msg) => super::panel::update(p, msg),
        (Content::TimeAndSales(Some(p)), msg) => super::panel::update(p, msg),
        (Content::OrderEntry(panel), panel::Message::OrderEntry(msg)) => {
            let is_virtual = state.is_virtual_mode;
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
                    panel::buying_power::Action::FetchBuyingPower => Some(Effect::FetchBuyingPower),
                };
            }
        }
        _ => {}
    }
    None
}

fn handle_study_configurator(
    state: &mut State,
    study_msg: modal::pane::settings::study::StudyMessage,
) -> Option<Effect> {
    match study_msg {
        modal::pane::settings::study::StudyMessage::Footprint(m) => {
            if let Content::Kline { chart, kind, .. } = &mut state.content
                && let Some(c) = chart
            {
                c.update_study_configurator(m);
                *kind = c.kind.clone();
            }
        }
        modal::pane::settings::study::StudyMessage::Heatmap(m) => {
            if let Content::Heatmap { chart, studies, .. } = &mut state.content
                && let Some(c) = chart
            {
                c.update_study_configurator(m);
                *studies = c.studies.clone();
            } else if let Content::ShaderHeatmap { chart, studies, .. } = &mut state.content
                && let Some(c) = chart
            {
                c.update_study_configurator(m);
                *studies = c.studies.clone();
            }
        }
    }
    None
}

fn handle_stream_modifier_changed(
    state: &mut State,
    message: modal::stream::Message,
) -> Option<Effect> {
    let Some(Modal::StreamModifier(mut modifier)) = state.modal.take() else {
        return None;
    };

    let mut effect: Option<Effect> = None;

    if let Some(action) = modifier.update(message) {
        match action {
            modal::stream::Action::TabSelected(tab) => {
                modifier.tab = tab;
            }
            modal::stream::Action::TicksizeSelected(tm) => {
                effect = handle_ticksize_selected(state, &mut modifier, tm);
            }
            modal::stream::Action::BasisSelected(new_basis) => {
                effect = handle_basis_selected(state, &mut modifier, new_basis);
            }
        }
    }

    state.modal = Some(Modal::StreamModifier(modifier));

    effect
}

fn handle_ticksize_selected(
    state: &mut State,
    modifier: &mut modal::stream::Modifier,
    tm: TickMultiplier,
) -> Option<Effect> {
    modifier.update_kind_with_multiplier(tm);
    state.settings.tick_multiply = Some(tm);

    if let Some(ticker) = state.stream_pair() {
        match &mut state.content {
            Content::Kline { chart: Some(c), .. } => {
                c.change_tick_size(tm.multiply_with_min_tick_step(ticker));
                c.reset_request_handler();
            }
            Content::Heatmap { chart: Some(c), .. } => {
                c.change_tick_size(tm.multiply_with_min_tick_step(ticker));
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

    let is_client = state
        .stream_pair()
        .map(|ti| ti.exchange().is_depth_client_aggr())
        .unwrap_or(false);

    if let Some(mut it) = state.streams.ready_iter_mut() {
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
        Some(Effect::RefreshStreams)
    } else {
        None
    }
}

fn handle_basis_selected(
    state: &mut State,
    modifier: &mut modal::stream::Modifier,
    new_basis: Basis,
) -> Option<Effect> {
    modifier.update_kind_with_basis(new_basis);
    state.settings.selected_basis = Some(new_basis);

    let base_ticker = state.stream_pair();

    match &mut state.content {
        Content::Heatmap { chart: Some(c), .. } => {
            c.set_basis(new_basis);
            update_depth_push_freq(&mut state.streams, new_basis);
            Some(Effect::RefreshStreams)
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
            update_depth_push_freq(&mut state.streams, new_basis);
            Some(Effect::RefreshStreams)
        }
        Content::Kline { chart: Some(c), .. } => {
            let base_ticker = base_ticker?;
            handle_basis_for_kline(
                c,
                &mut state.streams,
                state.settings.tick_multiply,
                base_ticker,
                new_basis,
            )
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

                state.streams = ResolvedStream::Ready(streams);
                let action = c.set_basis(new_basis);

                if let Some(chart::Action::RequestFetch(fetch)) = action {
                    return Some(Effect::RequestFetch(fetch));
                }
            }
            None
        }
        _ => None,
    }
}

fn update_depth_push_freq(streams: &mut ResolvedStream, new_basis: Basis) {
    if let Some(stream_type) = streams
        .ready_iter_mut()
        .and_then(|mut it| it.find(|s| matches!(s, StreamKind::Depth { .. })))
        && let StreamKind::Depth {
            push_freq,
            ticker_info,
            ..
        } = stream_type
        && ticker_info.exchange().is_custom_push_freq()
    {
        match new_basis {
            Basis::Time(tf) => *push_freq = exchange::PushFrequency::Custom(tf),
            Basis::Tick(_) => *push_freq = exchange::PushFrequency::ServerDefault,
        }
    }
}

fn handle_basis_for_kline(
    chart: &mut KlineChart,
    streams: &mut ResolvedStream,
    tick_multiply: Option<TickMultiplier>,
    base_ticker: TickerInfo,
    new_basis: Basis,
) -> Option<Effect> {
    match new_basis {
        Basis::Time(tf) => {
            let kline_stream = StreamKind::Kline {
                ticker_info: base_ticker,
                timeframe: tf,
            };
            let mut new_streams = vec![kline_stream];
            if matches!(chart.kind, data::chart::KlineChartKind::Footprint { .. }) {
                let depth_aggr = if base_ticker.exchange().is_depth_client_aggr() {
                    StreamTicksize::Client
                } else {
                    StreamTicksize::ServerSide(tick_multiply.unwrap_or(TickMultiplier(1)))
                };
                new_streams.push(StreamKind::Depth {
                    ticker_info: base_ticker,
                    depth_aggr,
                    push_freq: exchange::PushFrequency::ServerDefault,
                });
                new_streams.push(StreamKind::Trades {
                    ticker_info: base_ticker,
                });
            }
            let old_kline_stream = streams
                .ready_iter()
                .and_then(|mut it| it.find(|s| matches!(s, StreamKind::Kline { .. })))
                .copied();
            *streams = ResolvedStream::Ready(new_streams);
            let action = chart.set_basis(new_basis);
            if chart.is_replay_mode() {
                Some(Effect::ReloadReplayKlines {
                    old_stream: old_kline_stream,
                    new_stream: kline_stream,
                })
            } else if let Some(chart::Action::RequestFetch(fetch)) = action {
                Some(Effect::RequestFetch(fetch))
            } else {
                None
            }
        }
        Basis::Tick(_) => {
            let depth_aggr = if base_ticker.exchange().is_depth_client_aggr() {
                StreamTicksize::Client
            } else {
                StreamTicksize::ServerSide(tick_multiply.unwrap_or(TickMultiplier(1)))
            };
            *streams = ResolvedStream::Ready(vec![
                StreamKind::Depth {
                    ticker_info: base_ticker,
                    depth_aggr,
                    push_freq: exchange::PushFrequency::ServerDefault,
                },
                StreamKind::Trades {
                    ticker_info: base_ticker,
                },
            ]);
            chart.set_basis(new_basis);
            Some(Effect::RefreshStreams)
        }
    }
}

fn handle_comparison_chart_interaction(
    state: &mut State,
    message: crate::chart::comparison::Message,
) -> Option<Effect> {
    if let Content::Comparison(chart_opt) = &mut state.content
        && let Some(chart) = chart_opt
        && let Some(action) = chart.update(message)
    {
        match action {
            crate::chart::comparison::Action::SeriesColorChanged(t, color) => {
                chart.set_series_color(t, color);
            }
            crate::chart::comparison::Action::SeriesNameChanged(t, name) => {
                chart.set_series_name(t, name);
            }
            crate::chart::comparison::Action::OpenSeriesEditor => {
                state.modal = Some(Modal::Settings);
            }
            crate::chart::comparison::Action::RemoveSeries(ti) => {
                let rebuilt = chart.remove_ticker(&ti);
                state.streams = ResolvedStream::Ready(rebuilt);
                return Some(Effect::RefreshStreams);
            }
        }
    }
    None
}

fn handle_mini_tickers_list(
    state: &mut State,
    message: crate::modal::pane::mini_tickers_list::Message,
) -> Option<Effect> {
    if let Some(Modal::MiniTickersList(ref mut mini_panel)) = state.modal
        && let Some(action) = mini_panel.update(message)
    {
        let crate::modal::pane::mini_tickers_list::Action::RowSelected(sel) = action;
        match sel {
            crate::modal::pane::mini_tickers_list::RowSelection::Add(ti) => {
                if let Content::Comparison(chart) = &mut state.content
                    && let Some(c) = chart
                {
                    let rebuilt = c.add_ticker(&ti);
                    state.streams = ResolvedStream::Ready(rebuilt);
                    return Some(Effect::RefreshStreams);
                }
            }
            crate::modal::pane::mini_tickers_list::RowSelection::Remove(ti) => {
                if let Content::Comparison(chart) = &mut state.content
                    && let Some(c) = chart
                {
                    let rebuilt = c.remove_ticker(&ti);
                    state.streams = ResolvedStream::Ready(rebuilt);
                    return Some(Effect::RefreshStreams);
                }
            }
            crate::modal::pane::mini_tickers_list::RowSelection::Switch(ti) => {
                return Some(Effect::SwitchTickersInGroup(ti));
            }
        }
    }
    None
}

pub(super) fn virtual_order_from_new_order_request(
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
            .unwrap_or_default()
            .as_millis() as u64,
        status: VirtualOrderStatus::Pending,
    })
}
