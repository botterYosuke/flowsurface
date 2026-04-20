use super::{Content, State};
use crate::{
    chart::{comparison::ComparisonChart, kline::KlineChart},
    connector::ResolvedStream,
    screen::dashboard::panel::{ladder::Ladder, timeandsales::TimeAndSales},
    widget::chart::heatmap::HeatmapShader,
};
use data::{
    chart::{Basis, heatmap::HeatmapStudy, indicator::HeatmapIndicator},
    layout::pane::{ContentKind, PaneSetup, Settings},
};
use exchange::{Kline, OpenInterest, TickMultiplier, TickerInfo, Timeframe, adapter::StreamKind};

pub(super) fn set_content_and_streams(
    state: &mut State,
    tickers: Vec<TickerInfo>,
    kind: ContentKind,
) -> Vec<StreamKind> {
    if state.content.kind() != kind {
        state.settings.selected_basis = None;
        state.settings.tick_multiply = None;
    }

    let Some(&base_ticker) = tickers.first() else {
        log::warn!("set_content_and_streams: empty tickers — skipping");
        return vec![];
    };
    let prev_base_ticker = state.stream_pair();

    let derived_plan = PaneSetup::new(
        kind,
        base_ticker,
        prev_base_ticker,
        state.settings.selected_basis,
        state.settings.tick_multiply,
    );

    state.settings.selected_basis = derived_plan.basis;
    state.settings.tick_multiply = derived_plan.tick_multiplier;

    let Some((content, streams, override_basis)) = build_content_and_streams(
        kind,
        &derived_plan,
        base_ticker,
        &tickers,
        &state.content,
        &state.settings,
    ) else {
        return vec![];
    };

    if let Some(basis) = override_basis {
        state.settings.selected_basis = Some(basis);
    }

    state.content = content;
    let result = streams.clone();
    state.streams = ResolvedStream::Ready(streams);
    result
}

// ContentKind ごとのストリーム構築ロジックが独立しており、さらに分割すると
// 引数の受け渡しが複雑になるため match 関数として許容。
#[allow(clippy::too_many_lines)]
fn build_content_and_streams(
    kind: ContentKind,
    derived_plan: &PaneSetup,
    base_ticker: TickerInfo,
    tickers: &[TickerInfo],
    existing_content: &Content,
    settings: &Settings,
) -> Option<(Content, Vec<StreamKind>, Option<Basis>)> {
    let kline_stream = |ti: TickerInfo, tf: Timeframe| StreamKind::Kline {
        ticker_info: ti,
        timeframe: tf,
    };
    let depth_stream = |plan: &PaneSetup| StreamKind::Depth {
        ticker_info: plan.ticker_info,
        depth_aggr: plan.depth_aggr,
        push_freq: plan.push_freq,
    };
    let trades_stream = |plan: &PaneSetup| StreamKind::Trades {
        ticker_info: plan.ticker_info,
    };

    match kind {
        ContentKind::HeatmapChart => {
            let content = Content::new_heatmap(
                existing_content,
                derived_plan.ticker_info,
                settings,
                derived_plan.price_step,
            );
            let streams = vec![depth_stream(derived_plan), trades_stream(derived_plan)];
            Some((content, streams, None))
        }
        ContentKind::FootprintChart => {
            let content = Content::new_kline(
                kind,
                existing_content,
                derived_plan.ticker_info,
                settings,
                derived_plan.price_step,
            );
            let streams = by_basis_default(
                derived_plan.basis,
                Timeframe::M5,
                |tf| {
                    vec![
                        trades_stream(derived_plan),
                        kline_stream(derived_plan.ticker_info, tf),
                    ]
                },
                || vec![trades_stream(derived_plan)],
            );
            Some((content, streams, None))
        }
        ContentKind::CandlestickChart => {
            let content = Content::new_kline(
                kind,
                existing_content,
                derived_plan.ticker_info,
                settings,
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
                    ..*derived_plan
                };
                vec![trades_stream(&temp)]
            };
            let streams = by_basis_default(
                derived_plan.basis,
                Timeframe::M15,
                time_basis_stream,
                tick_basis_stream,
            );
            Some((content, streams, None))
        }
        ContentKind::TimeAndSales => {
            let config = settings
                .visual_config
                .clone()
                .and_then(|cfg| cfg.time_and_sales());
            let content =
                Content::TimeAndSales(Some(TimeAndSales::new(config, derived_plan.ticker_info)));
            let temp = PaneSetup {
                push_freq: exchange::PushFrequency::ServerDefault,
                ..*derived_plan
            };
            let streams = vec![trades_stream(&temp)];
            Some((content, streams, None))
        }
        ContentKind::Ladder => {
            let config = settings.visual_config.clone().and_then(|cfg| cfg.ladder());
            let content = Content::Ladder(Some(Ladder::new(
                config,
                derived_plan.ticker_info,
                derived_plan.price_step,
            )));
            let streams = vec![depth_stream(derived_plan), trades_stream(derived_plan)];
            Some((content, streams, None))
        }
        ContentKind::ComparisonChart => Some(build_comparison(
            derived_plan,
            tickers,
            settings,
            kline_stream,
        )),
        ContentKind::ShaderHeatmap => {
            let basis = derived_plan
                .basis
                .unwrap_or(Basis::default_heatmap_time(Some(derived_plan.ticker_info)));

            let (studies, indicators) = if let Content::ShaderHeatmap {
                chart,
                indicators,
                studies,
            } = existing_content
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
            let streams = vec![depth_stream(derived_plan), trades_stream(derived_plan)];
            Some((content, streams, None))
        }
        ContentKind::Starter
        | ContentKind::OrderEntry
        | ContentKind::OrderList
        | ContentKind::BuyingPower => {
            log::warn!(
                "set_content_and_streams: unexpected kind {:?} — skipping",
                kind
            );
            None
        }
    }
}

fn build_comparison(
    derived_plan: &PaneSetup,
    tickers: &[TickerInfo],
    settings: &Settings,
    kline_stream: impl Fn(TickerInfo, Timeframe) -> StreamKind,
) -> (Content, Vec<StreamKind>, Option<Basis>) {
    let config = settings
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
    let content = Content::Comparison(Some(ComparisonChart::new(basis, tickers, config)));
    let streams = tickers
        .iter()
        .copied()
        .map(|ti| kline_stream(ti, timeframe))
        .collect();
    (content, streams, Some(basis))
}

pub(super) fn rebuild_content(state: &mut State, replay_mode: bool) {
    // ticker_info を先に取得してからコンテンツを変更する（借用競合を回避）
    let ticker_info = state.stream_pair();

    match &mut state.content {
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

pub(super) fn insert_hist_oi(
    content: &mut Content,
    req_id: Option<uuid::Uuid>,
    oi: &[OpenInterest],
) {
    match content {
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
                content.kind()
            );
        }
    }
}

pub(super) fn insert_hist_klines(
    content: &mut Content,
    req_id: Option<uuid::Uuid>,
    timeframe: Timeframe,
    ticker_info: TickerInfo,
    klines: &[Kline],
) {
    match content {
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
                content.kind()
            );
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
