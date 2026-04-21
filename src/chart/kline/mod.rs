mod candle;
mod draw;
mod footprint;

use super::{
    Action, Basis, Chart, Message, PlotConstants, PlotData, ViewState, indicator, request_fetch,
    scale::linear::PriceInfoLabel,
};
use crate::chart::indicator::kline::KlineIndicatorImpl;
use crate::connector::fetcher::{FetchRange, RequestHandler, is_trade_fetch_enabled};
use crate::modal::pane::settings::study;
use data::aggr::ticks::TickAggr;
use data::aggr::time::TimeSeries;
use data::chart::indicator::{Indicator, KlineIndicator};
use data::chart::kline::{ClusterKind, ClusterScaling, FootprintStudy, KlineDataPoint};
use data::chart::{Autoscale, KlineChartKind, ViewConfig};

use exchange::unit::{Price, PriceStep};
use exchange::{Kline, OpenInterest as OIData, TickerInfo, Trade};

use iced::task::Handle;
use iced::{Element, Vector};

use enum_map::EnumMap;
use std::time::Instant;

impl Chart for KlineChart {
    type IndicatorKind = KlineIndicator;

    fn state(&self) -> &ViewState {
        &self.chart
    }

    fn mut_state(&mut self) -> &mut ViewState {
        &mut self.chart
    }

    fn invalidate_crosshair(&mut self) {
        self.chart.cache.clear_crosshair();
        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.clear_crosshair_caches());
    }

    fn invalidate_all(&mut self) {
        self.invalidate(None);
    }

    fn view_indicators(&'_ self, enabled: &[Self::IndicatorKind]) -> Vec<Element<'_, Message>> {
        let chart_state = self.state();
        let visible_region = chart_state.visible_region(chart_state.bounds.size());
        let (earliest, latest) = chart_state.interval_range(&visible_region);
        if earliest > latest {
            return vec![];
        }

        let market = chart_state.ticker_info.market_type();
        let mut elements = vec![];

        for selected_indicator in enabled {
            if !KlineIndicator::for_market(market).contains(selected_indicator) {
                continue;
            }
            if let Some(indi) = self.indicators[*selected_indicator].as_ref() {
                elements.push(indi.element(chart_state, earliest..=latest));
            }
        }
        elements
    }

    fn visible_timerange(&self) -> Option<(u64, u64)> {
        let chart = self.state();
        let region = chart.visible_region(chart.bounds.size());

        if region.width == 0.0 {
            return None;
        }

        Some(chart.interval_range(&region))
    }

    fn interval_keys(&self) -> Option<Vec<u64>> {
        match &self.data_source {
            PlotData::TimeBased(_) => None,
            PlotData::TickBased(tick_aggr) => Some(
                tick_aggr
                    .datapoints
                    .iter()
                    .map(|dp| dp.kline.time)
                    .collect(),
            ),
        }
    }

    fn autoscaled_coords(&self) -> Vector {
        let chart = self.state();
        let x_translation = match &self.kind {
            KlineChartKind::Footprint { .. } => {
                0.5 * (chart.bounds.width / chart.scaling) - (chart.cell_width / chart.scaling)
            }
            KlineChartKind::Candles => {
                0.5 * (chart.bounds.width / chart.scaling)
                    - (8.0 * chart.cell_width / chart.scaling)
            }
        };
        Vector::new(x_translation, chart.translation.y)
    }

    fn supports_fit_autoscaling(&self) -> bool {
        true
    }

    fn is_empty(&self) -> bool {
        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries.datapoints.is_empty(),
            PlotData::TickBased(tick_aggr) => tick_aggr.datapoints.is_empty(),
        }
    }
}

impl PlotConstants for KlineChart {
    fn min_scaling(&self) -> f32 {
        self.kind.min_scaling()
    }

    fn max_scaling(&self) -> f32 {
        self.kind.max_scaling()
    }

    fn max_cell_width(&self) -> f32 {
        self.kind.max_cell_width()
    }

    fn min_cell_width(&self) -> f32 {
        self.kind.min_cell_width()
    }

    fn max_cell_height(&self) -> f32 {
        self.kind.max_cell_height()
    }

    fn min_cell_height(&self) -> f32 {
        self.kind.min_cell_height()
    }

    fn default_cell_width(&self) -> f32 {
        self.kind.default_cell_width()
    }
}

pub struct KlineChart {
    chart: ViewState,
    data_source: PlotData<KlineDataPoint>,
    raw_trades: Vec<Trade>,
    indicators: EnumMap<KlineIndicator, Option<Box<dyn KlineIndicatorImpl>>>,
    fetching_trades: (bool, Option<Handle>),
    pub(crate) kind: KlineChartKind,
    request_handler: RequestHandler,
    study_configurator: study::Configurator<FootprintStudy>,
    last_tick: Instant,
    /// リプレイ中は true。fetch_missing_data による live API fetch を抑制する。
    replay_mode: bool,
}

impl KlineChart {
    pub fn new(
        layout: ViewConfig,
        basis: Basis,
        step: PriceStep,
        klines_raw: &[Kline],
        raw_trades: Vec<Trade>,
        enabled_indicators: &[KlineIndicator],
        ticker_info: TickerInfo,
        kind: &KlineChartKind,
    ) -> Self {
        match basis {
            Basis::Time(interval) => {
                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, klines_raw)
                    .with_trades(&raw_trades);

                let base_price_y = timeseries.base_price();
                let latest_x = timeseries.latest_timestamp().unwrap_or(0);
                let (scale_high, scale_low) = timeseries.price_scale({
                    match kind {
                        KlineChartKind::Footprint { .. } => 12,
                        KlineChartKind::Candles => 60,
                    }
                });

                let low_rounded = scale_low.round_to_side_step(true, step);
                let high_rounded = scale_high.round_to_side_step(false, step);

                let y_ticks = Price::steps_between_inclusive(low_rounded, high_rounded, step)
                    .map(|n| n.saturating_sub(1))
                    .unwrap_or(1)
                    .max(1) as f32;

                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 800.0 / y_ticks,
                    KlineChartKind::Candles => 200.0 / y_ticks,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    step.decimal_places(),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits,
                        autoscale: Some(Autoscale::FitToVisible),
                    },
                    cell_width,
                    cell_height,
                );
                chart.base_price_y = base_price_y;
                chart.latest_x = latest_x;

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                let data_source = PlotData::TimeBased(timeseries);

                let mut indicators = EnumMap::default();
                for &i in enabled_indicators {
                    let mut indi = indicator::kline::make_empty(i);
                    indi.rebuild_from_source(&data_source);
                    indicators[i] = Some(indi);
                }

                KlineChart {
                    chart,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                    replay_mode: false,
                }
            }
            Basis::Tick(interval) => {
                let cell_width = match kind {
                    KlineChartKind::Footprint { .. } => 80.0,
                    KlineChartKind::Candles => 4.0,
                };
                let cell_height = match kind {
                    KlineChartKind::Footprint { .. } => 90.0,
                    KlineChartKind::Candles => 8.0,
                };

                let mut chart = ViewState::new(
                    basis,
                    step,
                    step.decimal_places(),
                    ticker_info,
                    ViewConfig {
                        splits: layout.splits,
                        autoscale: Some(Autoscale::FitToVisible),
                    },
                    cell_width,
                    cell_height,
                );

                let x_translation = match &kind {
                    KlineChartKind::Footprint { .. } => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (chart.cell_width / chart.scaling)
                    }
                    KlineChartKind::Candles => {
                        0.5 * (chart.bounds.width / chart.scaling)
                            - (8.0 * chart.cell_width / chart.scaling)
                    }
                };
                chart.translation.x = x_translation;

                let data_source = PlotData::TickBased(TickAggr::new(interval, step, &raw_trades));

                let mut indicators = EnumMap::default();
                for &i in enabled_indicators {
                    let mut indi = indicator::kline::make_empty(i);
                    indi.rebuild_from_source(&data_source);
                    indicators[i] = Some(indi);
                }

                KlineChart {
                    chart,
                    data_source,
                    raw_trades,
                    indicators,
                    fetching_trades: (false, None),
                    request_handler: RequestHandler::default(),
                    kind: kind.clone(),
                    study_configurator: study::Configurator::new(),
                    last_tick: Instant::now(),
                    replay_mode: false,
                }
            }
        }
    }

    pub fn update_latest_kline(&mut self, kline: &Kline) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_klines(&[*kline]);

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(&[*kline]));

                let chart = self.mut_state();

                if (kline.time) > chart.latest_x {
                    chart.latest_x = kline.time;
                }

                chart.last_price = Some(PriceInfoLabel::new(kline.close, kline.open));
            }
            PlotData::TickBased(_) => {}
        }
    }

    pub fn kind(&self) -> &KlineChartKind {
        &self.kind
    }

    fn fetch_missing_data(&mut self) -> Option<Action> {
        // リプレイ中は live API fetch を行わない。
        // EventStore から注入されたデータのみを表示し、live データの混入を防ぐ。
        if self.replay_mode {
            return None;
        }
        match &self.data_source {
            PlotData::TimeBased(timeseries) => {
                let timeframe_ms = timeseries.interval.to_milliseconds();

                if timeseries.datapoints.is_empty() {
                    let latest = chrono::Utc::now().timestamp_millis() as u64;
                    let earliest = latest.saturating_sub(450 * timeframe_ms);

                    let range = FetchRange::Kline(earliest, latest);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                let (visible_earliest, visible_latest) = self.visible_timerange()?;
                let (kline_earliest, kline_latest) = timeseries.timerange();
                let visible_span = visible_latest.saturating_sub(visible_earliest);
                let prefetch_earliest = visible_earliest.saturating_sub(visible_span);

                // priority 1, initial klines for visible range
                if visible_earliest < kline_earliest {
                    let range = FetchRange::Kline(prefetch_earliest, kline_earliest);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }

                // priority 2, trades
                if let KlineChartKind::Footprint { .. } = self.kind
                    && !self.fetching_trades.0
                    && is_trade_fetch_enabled()
                    && let Some((fetch_from, fetch_to)) =
                        timeseries.suggest_trade_fetch_range(visible_earliest, visible_latest)
                {
                    let range = FetchRange::Trades(fetch_from, fetch_to);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        self.fetching_trades = (true, None);
                        return Some(action);
                    }
                }

                // priority 3, indicators
                // (e.g. open interest needs external fetch as it's not derived from klines)
                let ctx = indicator::kline::FetchCtx {
                    main_chart: &self.chart,
                    timeframe: timeseries.interval,
                    visible_earliest,
                    kline_latest,
                    prefetch_earliest,
                };
                for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
                    if let Some(range) = indi.fetch_range(&ctx)
                        && let Some(action) = request_fetch(&mut self.request_handler, range)
                    {
                        return Some(action);
                    }
                }

                // priority 4, missing klines & integrity check
                let check_earliest = prefetch_earliest.max(kline_earliest);
                let check_latest = visible_latest.saturating_add(timeframe_ms);

                if let Some(missing_keys) =
                    timeseries.check_kline_integrity(check_earliest, check_latest)
                {
                    let latest = missing_keys
                        .iter()
                        .max()
                        .unwrap_or(&visible_latest)
                        .saturating_add(timeframe_ms);
                    let earliest = missing_keys
                        .iter()
                        .min()
                        .unwrap_or(&visible_earliest)
                        .saturating_sub(timeframe_ms);

                    let range = FetchRange::Kline(earliest, latest);
                    if let Some(action) = request_fetch(&mut self.request_handler, range) {
                        return Some(action);
                    }
                }
            }
            PlotData::TickBased(_) => {
                // TODO: implement trade fetch
            }
        }

        None
    }

    pub fn reset_request_handler(&mut self) {
        self.request_handler = RequestHandler::default();
        self.fetching_trades = (false, None);
    }

    /// リプレイモードかどうかを返す。
    pub fn is_replay_mode(&self) -> bool {
        self.replay_mode
    }

    /// リプレイモードを設定する。true の場合 fetch_missing_data は live fetch を行わない。
    pub fn set_replay_mode(&mut self, enabled: bool) {
        self.replay_mode = enabled;
        if enabled {
            // replay 中は live fetch の状態をリセット（進行中のリクエストを無効化）
            self.request_handler = RequestHandler::default();
        }
    }

    pub fn raw_trades(&self) -> Vec<Trade> {
        self.raw_trades.clone()
    }

    pub fn set_handle(&mut self, handle: Handle) {
        self.fetching_trades.1 = Some(handle);
    }

    pub fn tick_size(&self) -> PriceStep {
        self.chart.tick_size
    }

    pub fn study_configurator(&self) -> &study::Configurator<FootprintStudy> {
        &self.study_configurator
    }

    /// チャートに保持されているバー数を返す。
    pub fn bar_count(&self) -> usize {
        match &self.data_source {
            PlotData::TimeBased(ts) => ts.datapoints.len(),
            PlotData::TickBased(ta) => ta.datapoints.len(),
        }
    }

    /// 最も古いバーのタイムスタンプ（ミリ秒）を返す。TickBased の場合は None。
    pub fn oldest_timestamp(&self) -> Option<u64> {
        match &self.data_source {
            PlotData::TimeBased(ts) => ts.datapoints.keys().next().copied(),
            PlotData::TickBased(_) => None,
        }
    }

    /// 最も新しいバーのタイムスタンプ（ミリ秒）を返す。TickBased の場合は None。
    pub fn newest_timestamp(&self) -> Option<u64> {
        match &self.data_source {
            PlotData::TimeBased(ts) => ts.datapoints.keys().last().copied(),
            PlotData::TickBased(_) => None,
        }
    }

    /// pandas DataFrame エクスポート用に OHLCV バーを返す。
    /// `since_ts`: このタイムスタンプ（ミリ秒）以降のバーのみ返す。
    /// `limit`: 返す最大バー数（最新 N 本）。
    /// TickBased チャートには対応していないため空 Vec を返す。
    pub fn bars_for_export(
        &self,
        since_ts: Option<u64>,
        limit: Option<usize>,
    ) -> Vec<(&u64, &exchange::Kline)> {
        let PlotData::TimeBased(ts) = &self.data_source else {
            return vec![];
        };

        let iter = ts
            .datapoints
            .iter()
            .filter(move |(t, _)| since_ts.map_or(true, |s| **t >= s));

        let bars: Vec<_> = iter.map(|(t, dp)| (t, &dp.kline)).collect();

        match limit {
            Some(n) if n < bars.len() => bars[bars.len() - n..].to_vec(),
            _ => bars,
        }
    }

    pub fn update_study_configurator(&mut self, message: study::Message<FootprintStudy>) {
        let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        else {
            return;
        };

        match self.study_configurator.update(message) {
            Some(study::Action::ToggleStudy(study, is_selected)) => {
                if is_selected {
                    let already_exists = studies.iter().any(|s| s.is_same_type(&study));
                    if !already_exists {
                        studies.push(study);
                    }
                } else {
                    studies.retain(|s| !s.is_same_type(&study));
                }
            }
            Some(study::Action::ConfigureStudy(study)) => {
                if let Some(existing_study) = studies.iter_mut().find(|s| s.is_same_type(&study)) {
                    *existing_study = study;
                }
            }
            None => {}
        }

        self.invalidate(None);
    }

    pub fn chart_layout(&self) -> ViewConfig {
        self.chart.layout()
    }

    pub fn set_cluster_kind(&mut self, new_kind: ClusterKind) {
        if let KlineChartKind::Footprint {
            ref mut clusters, ..
        } = self.kind
        {
            *clusters = new_kind;
        }

        self.invalidate(None);
    }

    pub fn set_cluster_scaling(&mut self, new_scaling: ClusterScaling) {
        if let KlineChartKind::Footprint {
            ref mut scaling, ..
        } = self.kind
        {
            *scaling = new_scaling;
        }

        self.invalidate(None);
    }

    pub fn basis(&self) -> Basis {
        self.chart.basis
    }

    pub fn change_tick_size(&mut self, new_step: PriceStep) {
        let chart = self.mut_state();

        chart.cell_height *= (new_step.units as f32) / (chart.tick_size.units as f32);
        chart.tick_size = new_step;

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.change_tick_size(new_step, &self.raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.change_tick_size(new_step, &self.raw_trades);
            }
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_ticksize_change(&self.data_source));

        self.invalidate(None);
    }

    pub fn set_basis(&mut self, new_basis: Basis) -> Option<Action> {
        self.chart.last_price = None;
        self.chart.basis = new_basis;

        match new_basis {
            Basis::Time(interval) => {
                let step = self.chart.tick_size;
                let timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, &[]);
                self.data_source = PlotData::TimeBased(timeseries);
            }
            Basis::Tick(tick_count) => {
                let step = self.chart.tick_size;
                let tick_aggr = TickAggr::new(tick_count, step, &self.raw_trades);
                self.data_source = PlotData::TickBased(tick_aggr);
            }
        }

        self.indicators
            .values_mut()
            .filter_map(Option::as_mut)
            .for_each(|indi| indi.on_basis_change(&self.data_source));

        self.reset_request_handler();
        self.invalidate(Some(Instant::now()))
    }

    pub fn studies(&self) -> Option<Vec<FootprintStudy>> {
        match &self.kind {
            KlineChartKind::Footprint { studies, .. } => Some(studies.clone()),
            _ => None,
        }
    }

    pub fn set_studies(&mut self, new_studies: Vec<FootprintStudy>) {
        if let KlineChartKind::Footprint {
            ref mut studies, ..
        } = self.kind
        {
            *studies = new_studies;
        }

        self.invalidate(None);
    }

    pub fn insert_trades(&mut self, buffer: &[Trade]) {
        self.raw_trades.extend_from_slice(buffer);

        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                let old_dp_len = tick_aggr.datapoints.len();
                tick_aggr.insert_trades(buffer);

                if let Some(last_dp) = tick_aggr.datapoints.last() {
                    self.chart.last_price =
                        Some(PriceInfoLabel::new(last_dp.kline.close, last_dp.kline.open));
                } else {
                    self.chart.last_price = None;
                }

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_trades(buffer, old_dp_len, &self.data_source));

                self.invalidate(None);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades_existing_buckets(buffer);

                if let Some(last_trade) = buffer.last() {
                    let rounded = (last_trade.time / timeseries.interval.to_milliseconds())
                        * timeseries.interval.to_milliseconds();
                    if let Some(dp) = timeseries.datapoints.get(&rounded) {
                        self.chart.last_price =
                            Some(PriceInfoLabel::new(dp.kline.close, dp.kline.open));
                    }
                }

                self.invalidate(None);
            }
        }
    }

    pub fn insert_raw_trades(&mut self, raw_trades: Vec<Trade>, is_batches_done: bool) {
        match self.data_source {
            PlotData::TickBased(ref mut tick_aggr) => {
                tick_aggr.insert_trades(&raw_trades);
            }
            PlotData::TimeBased(ref mut timeseries) => {
                timeseries.insert_trades_existing_buckets(&raw_trades);
            }
        }

        self.raw_trades.extend(raw_trades);

        if is_batches_done {
            self.fetching_trades = (false, None);
        }
    }

    pub fn insert_hist_klines(&mut self, req_id: uuid::Uuid, klines_raw: &[Kline]) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                let before = timeseries.datapoints.len();
                timeseries.insert_klines(klines_raw);
                timeseries.insert_trades_existing_buckets(&self.raw_trades);
                let after = timeseries.datapoints.len();

                // latest_x が未初期化（0）の場合、挿入したデータの最新タイムスタンプで更新する。
                // 保存状態から空チャートで復元された場合、ここで正しいビューポート位置が設定される。
                let ts_latest = timeseries.latest_timestamp();

                self.indicators
                    .values_mut()
                    .filter_map(Option::as_mut)
                    .for_each(|indi| indi.on_insert_klines(klines_raw));

                if let Some(ts_latest) = ts_latest {
                    let chart = self.mut_state();
                    if ts_latest > chart.latest_x {
                        chart.latest_x = ts_latest;
                    }
                }

                if klines_raw.is_empty() || (before > 0 && after == before) {
                    // 新しいデータポイントが追加されなかった場合（株式市場の土日祝日ギャップなど）、
                    // 再試行を防ぐため失敗としてマークする
                    self.request_handler
                        .mark_failed(req_id, "No new data received".to_string());
                } else {
                    self.request_handler.mark_completed(req_id);
                }
                self.invalidate(None);
            }
            PlotData::TickBased(_) => {}
        }
    }

    /// リプレイ用: EventStore から得た klines を一括挿入する。
    /// Live の `insert_hist_klines` とほぼ同一だが req_id 不要。
    pub fn ingest_historical_klines(&mut self, klines: &[Kline]) {
        if let PlotData::TimeBased(ref mut timeseries) = self.data_source {
            timeseries.insert_klines(klines);
            timeseries.insert_trades_existing_buckets(&self.raw_trades);

            self.indicators
                .values_mut()
                .filter_map(Option::as_mut)
                .for_each(|indi| indi.on_insert_klines(klines));

            if let Some(ts_latest) = timeseries.latest_timestamp() {
                let chart = self.mut_state();
                if ts_latest > chart.latest_x {
                    chart.latest_x = ts_latest;
                }
            }

            if let Some(last_k) = klines.last() {
                self.chart.last_price = Some(PriceInfoLabel::new(last_k.close, last_k.open));
            }

            self.invalidate(None);
        }
    }

    /// リプレイ seek 時にチャートデータをリセットする。
    /// ビューポート（translation, scaling, bounds）は保持し、データのみクリアする。
    pub fn reset_for_seek(&mut self) {
        match self.data_source {
            PlotData::TimeBased(ref mut timeseries) => {
                let interval = timeseries.interval;
                let step = self.chart.tick_size;
                *timeseries = TimeSeries::<KlineDataPoint>::new(interval, step, &[]);
            }
            PlotData::TickBased(ref mut tick_aggr) => {
                let interval = tick_aggr.interval;
                let step = self.chart.tick_size;
                *tick_aggr = TickAggr::new(interval, step, &[]);
            }
        }
        self.chart.last_price = None;
        self.request_handler = RequestHandler::default();
        // インジケータも空の data_source からリビルドしてスタレデータを除去する
        for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
            indi.rebuild_from_source(&self.data_source);
        }
        self.invalidate(None);
    }

    pub fn insert_open_interest(&mut self, req_id: Option<uuid::Uuid>, oi_data: &[OIData]) {
        if let Some(req_id) = req_id {
            if oi_data.is_empty() {
                self.request_handler
                    .mark_failed(req_id, "No data received".to_string());
            } else {
                self.request_handler.mark_completed(req_id);
            }
        }

        if let Some(indi) = self.indicators[KlineIndicator::OpenInterest].as_mut() {
            indi.on_open_interest(oi_data);
        }
    }

    fn calc_qty_scales(
        &self,
        earliest: u64,
        latest: u64,
        highest: Price,
        lowest: Price,
        step: PriceStep,
        cluster_kind: ClusterKind,
    ) -> f32 {
        let rounded_highest = highest.round_to_side_step(false, step).add_steps(1, step);
        let rounded_lowest = lowest.round_to_side_step(true, step).add_steps(-1, step);

        match &self.data_source {
            PlotData::TimeBased(timeseries) => timeseries
                .max_qty_ts_range(
                    cluster_kind,
                    earliest,
                    latest,
                    rounded_highest,
                    rounded_lowest,
                )
                .into(),
            PlotData::TickBased(tick_aggr) => {
                let earliest = earliest as usize;
                let latest = latest as usize;

                tick_aggr
                    .max_qty_idx_range(
                        cluster_kind,
                        earliest,
                        latest,
                        rounded_highest,
                        rounded_lowest,
                    )
                    .into()
            }
        }
    }

    pub fn last_update(&self) -> Instant {
        self.last_tick
    }

    pub fn invalidate(&mut self, now: Option<Instant>) -> Option<Action> {
        let chart = &mut self.chart;

        if let Some(autoscale) = chart.layout.autoscale {
            match autoscale {
                super::Autoscale::CenterLatest => {
                    let x_translation = match &self.kind {
                        KlineChartKind::Footprint { .. } => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (chart.cell_width / chart.scaling)
                        }
                        KlineChartKind::Candles => {
                            0.5 * (chart.bounds.width / chart.scaling)
                                - (8.0 * chart.cell_width / chart.scaling)
                        }
                    };
                    chart.translation.x = x_translation;

                    let calculate_target_y = |kline: exchange::Kline| -> f32 {
                        let y_low = chart.price_to_y(kline.low);
                        let y_high = chart.price_to_y(kline.high);
                        let y_close = chart.price_to_y(kline.close);

                        let mut target_y_translation = -(y_low + y_high) / 2.0;

                        if chart.bounds.height > f32::EPSILON && chart.scaling > f32::EPSILON {
                            let visible_half_height = (chart.bounds.height / chart.scaling) / 2.0;

                            let view_center_y_centered = -target_y_translation;

                            let visible_y_top = view_center_y_centered - visible_half_height;
                            let visible_y_bottom = view_center_y_centered + visible_half_height;

                            let padding = chart.cell_height;

                            if y_close < visible_y_top {
                                target_y_translation = -(y_close - padding + visible_half_height);
                            } else if y_close > visible_y_bottom {
                                target_y_translation = -(y_close + padding - visible_half_height);
                            }
                        }
                        target_y_translation
                    };

                    chart.translation.y = self.data_source.latest_y_midpoint(calculate_target_y);
                }
                super::Autoscale::FitToVisible => {
                    let visible_region = chart.visible_region(chart.bounds.size());
                    let (start_interval, end_interval) = chart.interval_range(&visible_region);

                    if let Some((lowest, highest)) = self
                        .data_source
                        .visible_price_range(start_interval, end_interval)
                    {
                        let padding = (highest - lowest) * 0.05;
                        let price_span = (highest - lowest) + (2.0 * padding);

                        if price_span > 0.0 && chart.bounds.height > f32::EPSILON {
                            let padded_highest = highest + padding;
                            let chart_height = chart.bounds.height;
                            let tick_size = chart.tick_size.to_f32_lossy();

                            if tick_size > 0.0 {
                                chart.cell_height = (chart_height * tick_size) / price_span;
                                chart.base_price_y = Price::from_f32(padded_highest);
                                chart.translation.y = -chart_height / 2.0;
                            }
                        }
                    }
                }
            }
        }

        chart.cache.clear_all();
        for indi in self.indicators.values_mut().filter_map(Option::as_mut) {
            indi.clear_all_caches();
        }

        if let Some(t) = now {
            self.last_tick = t;
            self.fetch_missing_data()
        } else {
            None
        }
    }

    pub fn toggle_indicator(&mut self, indicator: KlineIndicator) {
        let prev_indi_count = self.indicators.values().filter(|v| v.is_some()).count();

        if self.indicators[indicator].is_some() {
            self.indicators[indicator] = None;
        } else {
            let mut box_indi = indicator::kline::make_empty(indicator);
            box_indi.rebuild_from_source(&self.data_source);
            self.indicators[indicator] = Some(box_indi);
        }

        if let Some(main_split) = self.chart.layout.splits.first() {
            let current_indi_count = self.indicators.values().filter(|v| v.is_some()).count();
            self.chart.layout.splits = data::util::calc_panel_splits(
                *main_split,
                current_indi_count,
                Some(prev_indi_count),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_kline(time: u64) -> Kline {
        use exchange::{
            Volume,
            unit::{Qty, price::Price},
        };
        Kline {
            time,
            open: Price::from_f32(100.0),
            high: Price::from_f32(110.0),
            low: Price::from_f32(90.0),
            close: Price::from_f32(105.0),
            volume: Volume::TotalOnly(Qty::zero()),
        }
    }

    fn build_test_kline_chart(basis: Basis) -> KlineChart {
        use data::chart::{Autoscale, KlineChartKind, ViewConfig};
        use exchange::{Ticker, adapter::Exchange};

        let ticker_info = TickerInfo::new(
            Ticker::new("BTCUSDT", Exchange::BinanceSpot),
            1.0,
            1.0,
            None,
        );
        let layout = ViewConfig {
            splits: Vec::new(),
            autoscale: Some(Autoscale::FitToVisible),
        };
        let step = PriceStep { units: 1 };
        KlineChart::new(
            layout,
            basis,
            step,
            &[],
            Vec::new(),
            &[],
            ticker_info,
            &KlineChartKind::Candles,
        )
    }

    #[test]
    fn ingest_historical_klines_inserts_into_timeseries() {
        let mut chart = build_test_kline_chart(Basis::Time(exchange::Timeframe::M1));
        let klines: Vec<Kline> = (0..5).map(|i| make_kline(i * 60_000)).collect();

        chart.ingest_historical_klines(&klines);

        if let PlotData::TimeBased(ref ts) = chart.data_source {
            assert_eq!(ts.datapoints.len(), 5);
        } else {
            panic!("expected TimeBased data source");
        }
    }

    #[test]
    fn reset_for_seek_clears_timeseries() {
        let mut chart = build_test_kline_chart(Basis::Time(exchange::Timeframe::M1));
        let klines: Vec<Kline> = (0..5).map(|i| make_kline(i * 60_000)).collect();
        chart.ingest_historical_klines(&klines);

        chart.reset_for_seek();

        if let PlotData::TimeBased(ref ts) = chart.data_source {
            assert!(
                ts.datapoints.is_empty(),
                "reset_for_seek should clear all data"
            );
        } else {
            panic!("expected TimeBased data source");
        }
        assert!(chart.chart.last_price.is_none());
    }

    #[test]
    fn replay_mode_suppresses_fetch() {
        // Arrange: TimeBased chart with replay_mode enabled
        let mut chart = build_test_kline_chart(Basis::Time(exchange::Timeframe::M1));
        chart.set_replay_mode(true);

        // Act: invalidate with a timestamp triggers fetch_missing_data internally.
        // When replay_mode is true, fetch_missing_data must return None.
        let action = chart.invalidate(Some(std::time::Instant::now()));

        // Assert: no fetch action is emitted
        assert!(
            action.is_none(),
            "replay_mode=true should suppress fetch_missing_data; got Some(action)"
        );
    }

    #[test]
    fn bar_count_returns_correct_count() {
        // Arrange
        let mut chart = build_test_kline_chart(Basis::Time(exchange::Timeframe::M1));
        assert_eq!(
            chart.bar_count(),
            0,
            "freshly created chart should have 0 bars"
        );

        // Act
        let klines: Vec<Kline> = (0..5).map(|i| make_kline(i * 60_000)).collect();
        chart.ingest_historical_klines(&klines);

        // Assert
        assert_eq!(
            chart.bar_count(),
            5,
            "after ingesting 5 klines bar_count should be 5"
        );
    }

    #[test]
    fn reset_for_seek_resets_request_handler() {
        use crate::connector::fetcher::FetchRange;

        let mut chart = build_test_kline_chart(Basis::Time(exchange::Timeframe::M1));

        // Submit a request and mark it completed to activate the cooldown.
        let uuid = chart
            .request_handler
            .add_request(FetchRange::Kline(0, 1000))
            .expect("first add_request should not error")
            .expect("first add_request should return Some(uuid)");
        chart.request_handler.mark_completed(uuid);

        // Confirm cooldown is active: same range returns Ok(None).
        let during_cooldown = chart
            .request_handler
            .add_request(FetchRange::Kline(0, 1000))
            .expect("cooldown check should not error");
        assert!(
            during_cooldown.is_none(),
            "expected Ok(None) during cooldown, got Ok(Some(...))"
        );

        // After reset_for_seek, the handler should be cleared so the same
        // range can be re-requested (returns Ok(Some(_))).
        chart.reset_for_seek();

        let after_reset = chart
            .request_handler
            .add_request(FetchRange::Kline(0, 1000))
            .expect("post-reset add_request should not error");
        assert!(
            after_reset.is_some(),
            "expected Ok(Some(_)) after reset_for_seek, but got Ok(None) — request_handler was not reset"
        );
    }
}
