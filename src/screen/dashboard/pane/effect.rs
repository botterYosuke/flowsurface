use crate::connector::fetcher::FetchSpec;

#[derive(Debug, Clone)]
pub enum Effect {
    RefreshStreams,
    RequestFetch(Vec<FetchSpec>),
    SwitchTickersInGroup(exchange::TickerInfo),
    FocusWidget(iced::widget::Id),
    /// リプレイ中に kline stream の basis が変わったとき、コントローラに再ロードを依頼する。
    ReloadReplayKlines {
        old_stream: Option<exchange::adapter::StreamKind>,
        new_stream: exchange::adapter::StreamKind,
    },
    // ── 注文関連 Effect ───────────────────────────────────────────────────────
    SubmitNewOrder(exchange::adapter::tachibana::NewOrderRequest),
    SubmitCorrectOrder(exchange::adapter::tachibana::CorrectOrderRequest),
    SubmitCancelOrder(exchange::adapter::tachibana::CancelOrderRequest),
    /// REPLAYモード専用：仮想注文を VirtualExchangeEngine に登録する
    SubmitVirtualOrder(crate::replay::virtual_exchange::VirtualOrder),
    FetchOrders,
    FetchOrderDetail {
        order_num: String,
        eig_day: String,
    },
    FetchBuyingPower,
    FetchHoldings {
        issue_code: String,
    },
}
