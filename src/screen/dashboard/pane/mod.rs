mod content;
mod controls;
mod effect;
mod init;
mod update;
mod view;

pub use content::Content;
pub use effect::Effect;

use crate::{
    chart,
    connector::{ResolvedStream, fetcher::InfoKind},
    modal::{self, pane::Modal},
    screen::dashboard::{panel, tickers_table::TickersTable},
    widget::{column_drag, toast::Toast},
    window::{self, Window},
};
use data::{
    UserTimezone,
    chart::indicator::UiIndicator,
    layout::pane::{ContentKind, LinkGroup, Settings, VisualConfig},
    stream::PersistStreamKind,
};
use exchange::{Kline, OpenInterest, StreamPairKind, TickerInfo, Timeframe, adapter::StreamKind};
use iced::{Renderer, Theme, widget::pane_grid};
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
        init::set_content_and_streams(self, tickers, kind)
    }

    pub fn insert_hist_oi(&mut self, req_id: Option<uuid::Uuid>, oi: &[OpenInterest]) {
        init::insert_hist_oi(&mut self.content, req_id, oi);
    }

    /// リプレイ開始時にチャートデータをクリアし、settings/streams/layout/indicators は保持する。
    pub fn rebuild_content_for_replay(&mut self) {
        init::rebuild_content(self, true);
    }

    pub fn rebuild_content_for_live(&mut self) {
        init::rebuild_content(self, false);
    }

    pub fn insert_hist_klines(
        &mut self,
        req_id: Option<uuid::Uuid>,
        timeframe: Timeframe,
        ticker_info: TickerInfo,
        klines: &[Kline],
    ) {
        init::insert_hist_klines(&mut self.content, req_id, timeframe, ticker_info, klines);
    }

    pub(super) fn has_stream(&self) -> bool {
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
        view::render_pane(
            self,
            id,
            panes,
            is_focused,
            maximized,
            window,
            main_window,
            timezone,
            tickers_table,
            is_replay,
            theme,
        )
    }

    pub fn update(&mut self, msg: Event) -> Option<Effect> {
        update::dispatch(self, msg)
    }

    pub fn matches_stream(&self, stream: &StreamKind) -> bool {
        self.streams.matches_stream(stream)
    }

    pub(super) fn show_modal_with_focus(&mut self, requested_modal: Modal) -> Option<Effect> {
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
