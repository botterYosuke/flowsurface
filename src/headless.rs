/// Headless 繝｢繝ｼ繝・窶・GUI 縺ｪ縺励〒 tokio 繝ｩ繝ｳ繧ｿ繧､繝 + HTTP API 繧ｵ繝ｼ繝舌・縺縺代ｒ蜍輔°縺吶・///
/// `flowsurface --headless --ticker HyperliquidLinear:BTC --timeframe M1` 縺ｧ襍ｷ蜍輔☆繧九・/// iced::daemon 繧剃ｸ蛻・ｵｷ蜍輔＠縺ｪ縺・◆繧√￣ython SDK 縺ｮ繧医≧縺ｪ螟夜Κ繝励Ο繧ｰ繝ｩ繝縺九ｉ
/// HTTP API (port 9876) 邨檎罰縺ｧ鬮倬溘↓蠑ｷ蛹門ｭｦ鄙偵Ν繝ｼ繝励ｒ蝗槭○繧九・
use std::collections::HashSet;

use exchange::{
    Ticker, TickerInfo, Timeframe,
    adapter::{Exchange, StreamKind},
};
use futures::StreamExt;

use crate::replay::{
    ReplayMode, ReplaySession, ReplayState,
    clock::StepClock,
    loader,
    store::{EventStore, LoadedData},
    virtual_exchange::{PortfolioSnapshot, VirtualExchangeEngine, VirtualOrder, VirtualOrderType},
};
use crate::replay_api::{ApiCommand, ApiMessage, VirtualExchangeCommand};

// 笏笏 CLI 蠑墓焚 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

/// `--headless` 襍ｷ蜍墓凾縺ｫ蠢・ｦ√↑ CLI 蠑墓焚縲・
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessArgs {
    pub ticker: String,
    pub timeframe: String,
}

/// `args` 繧ｹ繝ｩ繧､繧ｹ・・std::env::args().collect()` 縺ｮ邨先棡・峨°繧・headless 逕ｨ蠑墓焚繧偵ヱ繝ｼ繧ｹ縺吶ｋ縲・///
/// - `--ticker <ExchangeName:Symbol>` 窶・蠢・・/// - `--timeframe <TF>` 窶・逵∫払譎ゅ・ `"M1"`
pub fn parse_headless_args(args: &[String]) -> Result<HeadlessArgs, String> {
    let mut ticker: Option<String> = None;
    let mut timeframe = "M1".to_string();

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--ticker" => {
                i += 1;
                ticker = args.get(i).cloned();
            }
            "--timeframe" => {
                i += 1;
                if let Some(t) = args.get(i) {
                    timeframe = t.clone();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let ticker = ticker.ok_or_else(|| "--ticker is required in headless mode".to_string())?;
    Ok(HeadlessArgs { ticker, timeframe })
}

// 笏笏 繝・ぅ繝・き繝ｼ / 繧ｿ繧､繝繝輔Ξ繝ｼ繝 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

/// "BinanceLinear:BTCUSDT" 繧・"HyperliquidLinear:BTC" 繧・`Ticker` 縺ｫ繝代・繧ｹ縺吶ｋ縲・
pub fn parse_ticker_str(s: &str) -> Result<Ticker, String> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid ticker format: expected 'Exchange:Symbol', got '{s}'"
        ));
    }
    let exchange_str = parts[0];
    // "BinanceLinear" -> "Binance Linear" to match `main.rs::parse_ser_ticker`.
    // Supported suffixes: Linear, Inverse, Spot.
    let normalized = ["Linear", "Inverse", "Spot"]
        .into_iter()
        .find_map(|suffix| {
            exchange_str
                .strip_suffix(suffix)
                .map(|prefix| format!("{prefix} {suffix}"))
        })
        .unwrap_or_else(|| exchange_str.to_owned());
    let exchange: Exchange = normalized
        .parse()
        .map_err(|_| format!("unknown exchange: {exchange_str}"))?;
    Ok(Ticker::new(parts[1], exchange))
}

/// "M1", "M5", "H1" 遲峨ｒ `Timeframe` 縺ｫ繝代・繧ｹ縺吶ｋ縲・
pub fn parse_timeframe_str(s: &str) -> Result<Timeframe, String> {
    let tf = match s {
        "MS100" => Timeframe::MS100,
        "MS200" => Timeframe::MS200,
        "MS300" => Timeframe::MS300,
        "MS500" => Timeframe::MS500,
        "MS1000" | "S1" => Timeframe::MS1000,
        "M1" | "1m" => Timeframe::M1,
        "M3" | "3m" => Timeframe::M3,
        "M5" | "5m" => Timeframe::M5,
        "M15" | "15m" => Timeframe::M15,
        "M30" | "30m" => Timeframe::M30,
        "H1" | "1h" => Timeframe::H1,
        "H2" | "2h" => Timeframe::H2,
        "H4" | "4h" => Timeframe::H4,
        "H12" | "12h" => Timeframe::H12,
        "D1" | "1d" => Timeframe::D1,
        _ => return Err(format!("unknown timeframe: {s}")),
    };
    Ok(tf)
}

// 笏笏 HeadlessEngine 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

#[allow(dead_code)]
enum LoadResult {
    Ok {
        stream: StreamKind,
        range: std::ops::Range<u64>,
        klines: Vec<exchange::Kline>,
    },
    Err(String),
}

struct HeadlessPane {
    id: uuid::Uuid,
    ticker: String,
    timeframe: Timeframe,
}

#[allow(dead_code)]
struct HeadlessEngine {
    state: ReplayState,
    virtual_engine: VirtualExchangeEngine,
    ticker: Ticker,
    ticker_str: String,
    timeframe: Timeframe,
    load_tx: tokio::sync::mpsc::Sender<LoadResult>,
    panes: Vec<HeadlessPane>,
    narrative_store: std::sync::Arc<crate::narrative::store::NarrativeStore>,
    snapshot_store: crate::narrative::snapshot_store::SnapshotStore,
    data_root: std::path::PathBuf,
    /// Agent replay API state for the default session.
    agent_session_state: crate::api::agent_session_state::AgentSessionState,
}

// NOTE: ADR-0001 ﾂｧ2 閾ｪ蜍募・逕滓ｩ滓ｧ九・蟒・ｭ｢縺ｫ莨ｴ縺・・// HeadlessEngine 縺ｮ play / step_forward / step_backward / resume / pause 邉ｻ繝｡繧ｽ繝・ラ縺ｯ
// 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ M 縺ｧ match arm 縺悟炎髯､縺輔ｌ縺ｦ orphaned 蛹悶＠縺ｦ縺・ｋ縲・// 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ S・・eadless 驥崎､・ｮ溯｣・炎髯､・峨〒讒矩菴薙・繝｡繧ｽ繝・ラ繝ｻ髢｢騾｣繝輔ぅ繝ｼ繝ｫ繝峨＃縺ｨ謨ｴ逅・☆繧九・
#[allow(dead_code)]
impl HeadlessEngine {
    fn new(
        ticker: Ticker,
        timeframe: Timeframe,
        load_tx: tokio::sync::mpsc::Sender<LoadResult>,
    ) -> Self {
        let state = ReplayState {
            mode: ReplayMode::Replay,
            ..Default::default()
        };
        let ticker_str = ticker.to_string();
        let initial_pane = HeadlessPane {
            id: uuid::Uuid::new_v4(),
            ticker: ticker_str.clone(),
            timeframe,
        };
        let data_root = data::data_path(None);
        #[cfg(test)]
        let narrative_store = std::sync::Arc::new(
            crate::narrative::store::NarrativeStore::open_in_memory()
                .expect("failed to open in-memory narrative store"),
        );
        #[cfg(not(test))]
        let narrative_store = std::sync::Arc::new(
            crate::narrative::store::NarrativeStore::open_default()
                .expect("failed to open narrative store"),
        );
        Self {
            state,
            virtual_engine: VirtualExchangeEngine::new(1_000_000.0),
            ticker,
            ticker_str,
            timeframe,
            load_tx,
            panes: vec![initial_pane],
            narrative_store,
            snapshot_store: crate::narrative::snapshot_store::SnapshotStore::new(data_root.clone()),
            data_root,
            agent_session_state: crate::api::agent_session_state::AgentSessionState::new(),
        }
    }

    fn enter_replay_mode(&mut self) {
        self.state.mode = ReplayMode::Replay;
    }

    fn enter_live_mode(&mut self) {
        let had_session = !matches!(self.state.session, ReplaySession::Idle);
        self.state.mode = ReplayMode::Live;
        self.state.session = ReplaySession::Idle;
        self.virtual_engine.reset();
        if had_session {
            self.virtual_engine.mark_session_terminated();
        }
    }

    /// `POST /api/replay/play {"start":"...","end":"..."}` 繧貞・逅・☆繧九・    
    fn play(&mut self, start: &str, end: &str) -> Result<String, String> {
        use crate::replay::parse_replay_range;

        let (start_ms, end_ms) = parse_replay_range(start, end).map_err(|e| e.to_string())?;

        let ticker_info = TickerInfo::new(self.ticker, 0.01, 0.001, None);
        let stream = StreamKind::Kline {
            ticker_info,
            timeframe: self.timeframe,
        };
        let step_ms = self.timeframe.to_milliseconds();
        let range = crate::replay::compute_load_range(start_ms, end_ms, step_ms);

        let clock = StepClock::new(start_ms, end_ms, step_ms);

        let mut active_streams = HashSet::new();
        active_streams.insert(stream);

        self.enter_replay_mode();
        self.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams,
        };
        self.state.range_input.start = start.to_string();
        self.state.range_input.end = end.to_string();
        self.virtual_engine.reset();
        // ADR-0001 SessionLifecycleEvent::Started 窶・agent state map 繧偵け繝ｪ繧｢縺輔○繧九・
        self.virtual_engine.mark_session_started();

        // kline 繝ｭ繝ｼ繝峨ｒ蛻･繧ｿ繧ｹ繧ｯ縺ｧ螳溯｡・

        let tx = self.load_tx.clone();
        tokio::spawn(async move {
            let result = loader::load_klines(stream, range).await;
            let msg = match result {
                Ok(r) => LoadResult::Ok {
                    stream: r.stream,
                    range: r.range,
                    klines: r.klines,
                },
                Err(e) => LoadResult::Err(e),
            };
            if let Err(e) = tx.send(msg).await {
                log::error!(
                    "headless: kline load result channel closed before delivery; \
                     main loop may have exited prematurely: {e}"
                );
            }
        });

        Ok(serde_json::json!({
            "ok": true,
            "status": "loading",
            "start": start,
            "end": end,
        })
        .to_string())
    }

    fn handle_load_result(&mut self, result: LoadResult) {
        match result {
            LoadResult::Ok {
                stream,
                range,
                klines,
            } => {
                let should_activate = if let ReplaySession::Loading {
                    pending_count,
                    store,
                    ..
                } = &mut self.state.session
                {
                    store.ingest_loaded(
                        stream,
                        range,
                        LoadedData {
                            klines,
                            trades: vec![],
                        },
                    );
                    *pending_count = pending_count.saturating_sub(1);
                    *pending_count == 0
                } else {
                    false
                };

                if should_activate {
                    let old = std::mem::replace(&mut self.state.session, ReplaySession::Idle);
                    if let ReplaySession::Loading {
                        clock,
                        store,
                        active_streams,
                        ..
                    } = old
                    {
                        self.state.session = ReplaySession::Active {
                            clock,
                            store,
                            active_streams,
                        };
                    }
                }
            }
            LoadResult::Err(e) => {
                log::error!("headless kline load failed: {e}");
                self.state.session = ReplaySession::Idle;
            }
        }
    }

    fn get_status_json(&self) -> String {
        serde_json::to_string(&self.state.to_status())
            .unwrap_or_else(|e| format!(r#"{{"error":"serialize failed: {e}"}}"#))
    }

    /// `get_state_json` 縺ｮ繝ｭ繧ｸ繝・け繧呈ｧ矩蛹悶ョ繝ｼ繧ｿ縺ｨ縺励※霑斐☆・・gent API 逕ｨ・峨・    /// JSON 譁・ｭ怜・蛹悶○縺壹～StepObservation` 讒矩菴薙ｒ霑斐☆縲・    
    fn build_step_observation(
        &self,
        limit: usize,
    ) -> Option<crate::api::step_response::StepObservation> {
        use crate::api::step_response::StepObservation;
        match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => {
                let now_ms = clock.now_ms();
                let mut ohlcv = Vec::new();
                let mut recent_trades = Vec::new();

                let mut sorted: Vec<_> = active_streams.iter().collect();
                sorted.sort_by_cached_key(|s| format!("{s:?}"));

                for stream in sorted {
                    if let StreamKind::Kline {
                        ticker_info,
                        timeframe,
                    } = stream
                    {
                        let now_end = now_ms.saturating_add(1);
                        let all_klines = store.klines_in(stream, 0..now_end);
                        let slice = if all_klines.len() > limit {
                            &all_klines[all_klines.len() - limit..]
                        } else {
                            all_klines
                        };
                        if !slice.is_empty() {
                            let exchange_str =
                                format!("{:?}", ticker_info.ticker.exchange).replace(' ', "");
                            let label =
                                format!("{exchange_str}:{}:{timeframe}", ticker_info.ticker);
                            for k in slice {
                                ohlcv.push(serde_json::json!({
                                    "stream": label,
                                    "time": k.time,
                                    "open": k.open.to_f64(),
                                    "high": k.high.to_f64(),
                                    "low": k.low.to_f64(),
                                    "close": k.close.to_f64(),
                                    "volume": k.volume.total().to_f64(),
                                }));
                            }
                        }

                        let trade_stream = StreamKind::Trades {
                            ticker_info: *ticker_info,
                        };
                        let trade_start =
                            now_ms.saturating_sub(crate::replay::controller::api::TRADE_WINDOW_MS);
                        let all_trades = store.trades_in(&trade_stream, trade_start..now_ms + 1);
                        let trade_slice = if all_trades.len() > limit {
                            &all_trades[all_trades.len() - limit..]
                        } else {
                            all_trades
                        };
                        if !trade_slice.is_empty() {
                            let exchange_str =
                                format!("{:?}", ticker_info.ticker.exchange).replace(' ', "");
                            let label = format!("{exchange_str}:{}:Trades", ticker_info.ticker);
                            let items: Vec<serde_json::Value> = trade_slice
                                .iter()
                                .map(|t| {
                                    serde_json::json!({
                                        "time": t.time,
                                        "price": t.price.to_f64(),
                                        "qty": t.qty.to_f64(),
                                        "is_sell": t.is_sell,
                                    })
                                })
                                .collect();
                            recent_trades
                                .push(serde_json::json!({"stream": label, "trades": items}));
                        }
                    }
                }

                let current_price = self.last_close_price().unwrap_or(0.0);
                let portfolio = self.virtual_engine.portfolio_snapshot(current_price);
                Some(StepObservation {
                    ohlcv,
                    recent_trades,
                    portfolio,
                })
            }
            _ => None,
        }
    }

    /// `POST /api/agent/session/default/advance` 繧貞・逅・☆繧具ｼ・hase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ G・峨・    ///
    /// ADR-0001 / phase4b_agent_replay_api.md ﾂｧ4.3 縺ｫ蝓ｺ縺･縺上ゆｻｻ諢丞玄髢薙ｒ wall-time
    /// 髱樔ｾ晏ｭ倥〒 instant 螳溯｡後☆繧九Ｔtop_on 縺ｫ蠢懊§縺ｦ fill / narrative 譖ｴ譁ｰ逋ｺ逕滓凾轤ｹ縺ｧ
    /// 蛛懈ｭ｢縺吶ｋ縲・eadless 繝ｩ繝ｳ繧ｿ繧､繝蟆ら畑・・UI 蛛ｴ縺ｯ app/api/mod.rs 縺ｧ 400 諡貞凄貂医∩・峨・    
    async fn agent_session_advance(
        &mut self,
        request: crate::api::advance_request::AgentAdvanceRequest,
    ) -> (u16, String) {
        use crate::api::advance_request::{
            AdvanceResponse, AdvanceStopCondition, AdvanceStoppedReason,
        };
        use crate::api::step_response::StepFill;

        match &self.state.session {
            ReplaySession::Idle => {
                return (
                    404,
                    r#"{"error":"session not started","hint":"start a replay session first (see agent_replay_api.md Getting Started)"}"#
                        .to_string(),
                );
            }
            ReplaySession::Loading { .. } => {
                return (503, r#"{"error":"session loading"}"#.to_string());
            }
            ReplaySession::Active { .. } => {}
        }

        let stop_on_fill = request.stop_on.contains(&AdvanceStopCondition::Fill);
        let stop_on_narrative = request.stop_on.contains(&AdvanceStopCondition::Narrative);

        let start_time = self.state.current_time();
        let until_ms = request.until_ms.as_u64();
        if until_ms <= start_time {
            let final_portfolio = self
                .virtual_engine
                .portfolio_snapshot(self.last_close_price().unwrap_or(0.0));
            let resp = AdvanceResponse {
                clock_ms: crate::api::contract::EpochMs::from(start_time),
                stopped_reason: AdvanceStoppedReason::UntilReached,
                ticks_advanced: 0,
                aggregate_fills: 0,
                aggregate_updated_narratives: 0,
                fills: if request.include_fills {
                    Some(Vec::new())
                } else {
                    None
                },
                final_portfolio,
            };
            return match serde_json::to_string(&resp) {
                Ok(json) => (200, json),
                Err(e) => (
                    500,
                    serde_json::json!({"error": format!("serialize failed: {e}")}).to_string(),
                ),
            };
        }

        let overall_start = std::time::Instant::now();
        let mut ticks_advanced: u64 = 0;
        let mut aggregate_fills: usize = 0;
        let mut aggregate_updated_narratives: usize = 0;
        let mut collected_fills: Vec<StepFill> = Vec::new();
        let mut stopped_reason = AdvanceStoppedReason::UntilReached;

        self.agent_session_state
            .observe_generation(self.virtual_engine.session_generation());

        loop {
            let pane_step = self.min_step_ms();
            // pane_step == 0 は active_streams が空等の縮退状態を意味する。
            // このまま loop に入ると `new_time == current` で clock が進まず
            // 30 秒 timeout まで 504 を返せなくなるため 500 で早期終了する。
            if pane_step == 0 {
                log::error!("agent_session_advance: min_step_ms() returned 0 (no active streams?)");
                return (
                    500,
                    r#"{"error":"no active stream step size available"}"#.to_string(),
                );
            }
            let new_time = match &self.state.session {
                ReplaySession::Active { clock, .. } => {
                    let current = clock.now_ms();
                    let end = clock.full_range().end;
                    if current >= end {
                        stopped_reason = AdvanceStoppedReason::End;
                        break;
                    }
                    current.saturating_add(pane_step).min(until_ms).min(end)
                }
                _ => break,
            };

            let fills: Vec<_> = if let ReplaySession::Active {
                clock,
                store,
                active_streams,
            } = &mut self.state.session
            {
                clock.tick_until(new_time);
                let current_time = clock.now_ms();
                let ticker_str = self.ticker_str.clone();
                let synthetic: Vec<exchange::Trade> = active_streams
                    .iter()
                    .filter(|s| matches!(s, StreamKind::Kline { .. }))
                    .filter_map(|stream| {
                        let klines = store.klines_in(stream, 0..current_time.saturating_add(1));
                        klines
                            .iter()
                            .rev()
                            .find(|k| k.time <= current_time)
                            .map(|k| exchange::Trade {
                                time: current_time,
                                is_sell: false,
                                price: k.close,
                                qty: exchange::unit::qty::Qty::from_f32(1.0),
                            })
                    })
                    .collect();
                if synthetic.is_empty() {
                    Vec::new()
                } else {
                    self.virtual_engine
                        .on_tick(&ticker_str, &synthetic, current_time)
                }
            } else {
                Vec::new()
            };

            let mut tick_updated_narratives = 0usize;
            for fill in &fills {
                match crate::narrative::service::update_outcome_from_fill_returning_ids(
                    &self.narrative_store,
                    &fill.order_id,
                    fill.fill_price,
                    crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64(),
                    None,
                )
                .await
                {
                    Ok(ids) => tick_updated_narratives += ids.len(),
                    Err(e) => log::warn!(
                        "agent_session_advance: narrative outcome update failed for {oid}: {e}",
                        oid = fill.order_id
                    ),
                }
            }

            ticks_advanced += 1;
            aggregate_fills += fills.len();
            aggregate_updated_narratives += tick_updated_narratives;

            if request.include_fills {
                for fill in &fills {
                    let client_order_id = self
                        .agent_session_state
                        .client_order_id_for(&fill.order_id)
                        .map(|c| c.as_str().to_string());
                    collected_fills.push(StepFill::from_event(fill, client_order_id));
                }
            }

            if stop_on_fill && !fills.is_empty() {
                stopped_reason = AdvanceStoppedReason::Fill;
                break;
            }
            if stop_on_narrative && tick_updated_narratives > 0 {
                stopped_reason = AdvanceStoppedReason::Narrative;
                break;
            }
            if let ReplaySession::Active { clock, .. } = &self.state.session
                && clock.now_ms() >= clock.full_range().end
            {
                stopped_reason = AdvanceStoppedReason::End;
                break;
            }
            if new_time >= until_ms {
                stopped_reason = AdvanceStoppedReason::UntilReached;
                break;
            }
        }

        let clock_ms = self.state.current_time();
        let final_portfolio = self
            .virtual_engine
            .portfolio_snapshot(self.last_close_price().unwrap_or(0.0));

        log::debug!(
            "agent_session_advance: {ticks} ticks in {ms}ms, fills={fills}, narratives={nar}, stopped={stopped:?}",
            ticks = ticks_advanced,
            ms = overall_start.elapsed().as_millis(),
            fills = aggregate_fills,
            nar = aggregate_updated_narratives,
            stopped = stopped_reason
        );

        let resp = AdvanceResponse {
            clock_ms: crate::api::contract::EpochMs::from(clock_ms),
            stopped_reason,
            ticks_advanced,
            aggregate_fills,
            aggregate_updated_narratives,
            fills: if request.include_fills {
                Some(collected_fills)
            } else {
                None
            },
            final_portfolio,
        };

        match serde_json::to_string(&resp) {
            Ok(json) => (200, json),
            Err(e) => (
                500,
                serde_json::json!({"error": format!("serialize failed: {e}")}).to_string(),
            ),
        }
    }

    /// `POST /api/agent/session/default/rewind-to-start` 繧貞・逅・☆繧九・    ///
    /// ADR-0001 ﾂｧ4 / ﾂｧ6 縺ｫ蝓ｺ縺･縺・
    /// - Active + body 譛・辟｡: clock 繧・range.start 縺ｫ謌ｻ縺励～VirtualExchange::reset()` +
    ///   `mark_session_reset()` 繧堤匱轣ｫ縺吶ｋ・・ody 縺ｯ辟｡隕悶＆繧後ｋ・峨・    /// - Loading: 409 Conflict縲・    /// - Idle + body: `play(start, end)` 繧貞他縺ｳ蜃ｺ縺励※ session 繧呈眠隕丞・譛溷喧縺吶ｋ
    ///   (ADR-0001 ﾂｧ4 縲梧悴蛻晄悄蛹匁凾縺ｮ蛻晄悄蛹也ｵ瑚ｷｯ縲・縲・    /// - Idle + body 縺ｪ縺・ 400 Bad Request縲・    
    async fn agent_session_rewind(
        &mut self,
        init_range: Option<(String, String)>,
    ) -> (u16, String) {
        use crate::replay::ReplaySession;

        match &self.state.session {
            ReplaySession::Loading { .. } => {
                return (409, r#"{"error":"session loading"}"#.to_string());
            }
            ReplaySession::Idle => {
                let Some((start, end)) = init_range else {
                    return (
                        400,
                        r#"{"error":"body required for init (session not initialized)"}"#
                            .to_string(),
                    );
                };
                return match self.play(&start, &end) {
                    Ok(json) => (200, json),
                    Err(e) => (400, serde_json::json!({"error": e}).to_string()),
                };
            }
            ReplaySession::Active { .. } => {}
        }

        // Active: clock seek + SessionLifecycleEvent::Reset
        if let ReplaySession::Active { clock, .. } = &mut self.state.session {
            let start = clock.full_range().start;
            clock.seek(start);
        }
        self.virtual_engine.reset();
        self.virtual_engine.mark_session_reset();

        let clock_ms = self.state.current_time();
        let snapshot_price = self.last_close_price().unwrap_or(0.0);
        let final_portfolio = self.virtual_engine.portfolio_snapshot(snapshot_price);
        let body = serde_json::json!({
            "ok": true,
            "clock_ms": clock_ms,
            "final_portfolio": final_portfolio,
        });
        (200, body.to_string())
    }

    /// `POST /api/agent/session/default/order` 繧貞・逅・☆繧具ｼ・hase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ E・峨・    ///
    /// ADR-0001 / phase4b_agent_replay_api.md ﾂｧ3.3, ﾂｧ4.4, ﾂｧ4.5 縺ｫ蝓ｺ縺･縺上・    /// - session 譛ｪ襍ｷ蜍輔・ 404 + hint
    /// - `client_order_id` 驥崎､・& body 荳閾ｴ 竊・200 + `idempotent_replay: true`
    /// - `client_order_id` 驥崎､・& body 逶ｸ驕・竊・409 Conflict
    /// - 譁ｰ隕・竊・201 Created
    fn agent_session_place_order(
        &mut self,
        request: crate::api::order_request::AgentOrderRequest,
    ) -> (u16, String) {
        use crate::api::agent_session_state::PlaceOrderOutcome;
        use crate::api::order_request::{AgentOrderSide, AgentOrderType};
        use crate::replay::virtual_exchange::{
            PositionSide, VirtualOrder, VirtualOrderStatus, VirtualOrderType,
        };

        // session 迥ｶ諷九メ繧ｧ繝・け・・tep 縺ｨ蜷後§ 404 / 503 繝ｫ繝ｼ繝ｫ・峨・

        match &self.state.session {
            ReplaySession::Idle => {
                return (
                    404,
                    r#"{"error":"session not started","hint":"start a replay session first (see agent_replay_api.md Getting Started)"}"#
                        .to_string(),
                );
            }
            ReplaySession::Loading { .. } => {
                return (503, r#"{"error":"session loading"}"#.to_string());
            }
            ReplaySession::Active { .. } => {}
        }

        // 逋ｺ豕ｨ蜑阪↓ lifecycle 繧､繝吶Φ繝医ｒ隕ｳ貂ｬ縺励※ map 繧貞ｿ・ｦ√↓蠢懊§繧ｯ繝ｪ繧｢縲・

        self.agent_session_state
            .observe_generation(self.virtual_engine.session_generation());

        let key = request.to_key();
        // 莉ｮ UUID 繧呈治逡ｪ・・dempotent replay 繧・conflict 縺ｧ縺ｯ菴ｿ繧上ｌ縺ｪ縺・= 謐ｨ縺ｦ繧峨ｌ繧具ｼ峨・
        let prospective_order_id = uuid::Uuid::new_v4().to_string();
        let outcome = self.agent_session_state.place_or_replay(
            request.client_order_id.clone(),
            key,
            prospective_order_id,
        );

        match outcome {
            PlaceOrderOutcome::Created { order_id } => {
                // VirtualExchange 縺ｫ螳溽匱豕ｨ縲・
                let side = match request.side {
                    AgentOrderSide::Buy => PositionSide::Long,
                    AgentOrderSide::Sell => PositionSide::Short,
                };
                let order_type = match request.order_type {
                    AgentOrderType::Market {} => VirtualOrderType::Market,
                    AgentOrderType::Limit { price } => VirtualOrderType::Limit { price },
                };
                let virtual_order = VirtualOrder {
                    order_id: order_id.clone(),
                    // VirtualOrder keeps the bare symbol, not the exchange-prefixed label.
                    ticker: request.ticker.symbol.clone(),
                    side,
                    qty: request.qty,
                    order_type,
                    placed_time_ms: self.state.current_time(),
                    status: VirtualOrderStatus::Pending,
                };
                self.virtual_engine.place_order(virtual_order);
                let body = serde_json::json!({
                    "order_id": order_id,
                    "client_order_id": request.client_order_id.as_str(),
                    "idempotent_replay": false,
                });
                // New orders and idempotent replays both use HTTP 200; clients distinguish
                // them through the `idempotent_replay` flag in the response body.
                (200, body.to_string())
            }
            PlaceOrderOutcome::IdempotentReplay { order_id } => {
                let body = serde_json::json!({
                    "order_id": order_id,
                    "client_order_id": request.client_order_id.as_str(),
                    "idempotent_replay": true,
                });
                (200, body.to_string())
            }
            PlaceOrderOutcome::Conflict { existing_order_id } => {
                let body = serde_json::json!({
                    "error": "client_order_id conflict with different request body",
                    "existing_order_id": existing_order_id,
                });
                (409, body.to_string())
            }
        }
    }

    /// `POST /api/agent/session/default/step` 繧貞・逅・☆繧具ｼ・hase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ C / D・峨・    ///
    /// ADR-0001 / phase4b_agent_replay_api.md ﾂｧ4.2 縺ｫ蝓ｺ縺･縺阪・ 繝舌・騾ｲ陦後＠縺・tick 縺ｮ
    /// `clock_ms` / `reached_end` / `observation` / `fills` / `updated_narrative_ids`
    /// 繧貞酔譴ｱ縺励◆繝ｬ繧ｹ繝昴Φ繧ｹ繧定ｿ斐☆縲ゅし繝悶ヵ繧ｧ繝ｼ繧ｺ D 莉･髯阪］arrative outcome 譖ｴ譁ｰ縺ｯ
    /// 蜷梧悄 `await` 縺ｧ遒ｺ螳壹＆縺帙ｋ・・gent 蛛ｴ polling 繧剃ｸ崎ｦ√↓縺吶ｋ縺溘ａ・峨・    
    async fn agent_session_step(&mut self) -> (u16, String) {
        use crate::api::step_response::{StepFill, StepResponse};

        let overall_start = std::time::Instant::now();

        // 繧ｻ繝・す繝ｧ繝ｳ迥ｶ諷九メ繧ｧ繝・け縲・dle 縺ｯ 404縲´oading 縺ｯ 503縲・

        match &self.state.session {
            ReplaySession::Idle => {
                return (
                    404,
                    r#"{"error":"session not started","hint":"start a replay session first (see agent_replay_api.md Getting Started)"}"#
                        .to_string(),
                );
            }
            ReplaySession::Loading { .. } => {
                return (503, r#"{"error":"session loading"}"#.to_string());
            }
            ReplaySession::Active { .. } => {}
        }

        // Agent state 繧・lifecycle 繧､繝吶Φ繝医↓蜷梧悄・医ワ繝ｳ繝峨Λ蜈･蜿｣縺ｧ 1 蝗槭・縺ｿ・峨・        // 蜑榊屓縺ｮ step/order 莉･髯阪↓ UI 繝ｪ繝｢繧ｳ繝ｳ邨檎罰縺ｧ /play 繧・seek 縺瑚ｵｰ縺｣縺溷ｴ蜷医・        // 縺薙％縺ｧ stale 縺ｪ client_order_id 繝槭ャ繝励′遐ｴ譽・＆繧後ｋ縲・

        self.agent_session_state
            .observe_generation(self.virtual_engine.session_generation());

        // Playing 荳ｭ縺ｪ繧芽・蜍・pause・・tep-forward 莉墓ｧ倥→蟇ｾ遘ｰ・峨・
        // 1 繝舌・騾ｲ繧√ｋ縲らｯ・峇邨らｫｯ縺ｪ繧・reached_end = true 縺ｧ迴ｾ蝨ｨ譎ょ綾繧呈紺縺育ｽｮ縺阪・
        let pane_step = self.min_step_ms();
        let (new_time, reached_end) = match &self.state.session {
            ReplaySession::Active { clock, .. } => {
                let current = clock.now_ms();
                let end = clock.full_range().end;
                if current + pane_step > end {
                    (current, true)
                } else {
                    (current + pane_step, false)
                }
            }
            other => {
                log::error!(
                    "agent_session_step: session state unexpectedly changed to {:?}",
                    std::mem::discriminant(other)
                );
                return (
                    500,
                    r#"{"error":"internal: session state changed unexpectedly"}"#.to_string(),
                );
            }
        };

        // 騾ｲ陦悟・逅・ｼ・eached_end 縺ｧ縺ｪ縺・ｴ蜷医・縺ｿ・・

        let fills: Vec<_> = if !reached_end {
            if let ReplaySession::Active {
                clock,
                store,
                active_streams,
            } = &mut self.state.session
            {
                clock.seek(new_time);
                let ticker_str = self.ticker_str.clone();
                let synthetic: Vec<exchange::Trade> =
                    active_streams
                        .iter()
                        .filter(|s| matches!(s, StreamKind::Kline { .. }))
                        .filter_map(|stream| {
                            let klines = store.klines_in(stream, 0..new_time.saturating_add(1));
                            klines.iter().rev().find(|k| k.time <= new_time).map(|k| {
                                exchange::Trade {
                                    time: new_time,
                                    is_sell: false,
                                    price: k.close,
                                    qty: exchange::unit::qty::Qty::from_f32(1.0),
                                }
                            })
                        })
                        .collect();
                if synthetic.is_empty() {
                    Vec::new()
                } else {
                    self.virtual_engine
                        .on_tick(&ticker_str, &synthetic, new_time)
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // narrative outcome 譖ｴ譁ｰ繧貞酔譛・await 縺ｧ遒ｺ螳壹＆縺帙ｋ・医し繝悶ヵ繧ｧ繝ｼ繧ｺ D・峨・        // plan ﾂｧ5.2 / ﾂｧ8 R1: fill_count 莉ｶ 竊・100ms/p95 繧定ｶ・∴縺溘ｉ髱槫酔譛溷喧縺ｫ謌ｻ縺吝愛譁ｭ蝓ｺ貅悶・        // 螟ｱ謨励・ log::warn! 縺ｮ縺ｿ縺ｧ step 蜈ｨ菴薙ｒ關ｽ縺ｨ縺輔↑縺・ｼ・lan ﾂｧ7.2 譁ｹ驥晢ｼ峨・

        let narrative_start = std::time::Instant::now();
        let mut updated_narrative_ids: Vec<String> = Vec::new();
        for fill in &fills {
            match crate::narrative::service::update_outcome_from_fill_returning_ids(
                &self.narrative_store,
                &fill.order_id,
                fill.fill_price,
                crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64(),
                None,
            )
            .await
            {
                Ok(ids) => {
                    for id in ids {
                        updated_narrative_ids.push(id.to_string());
                    }
                }
                Err(e) => {
                    log::warn!(
                        "agent_session_step: narrative outcome update failed for order {order_id}: {e}",
                        order_id = fill.order_id
                    );
                }
            }
        }
        let narrative_elapsed_ms = narrative_start.elapsed().as_millis();
        if narrative_elapsed_ms > 100 {
            log::warn!(
                "agent_session_step: narrative outcome update took {narrative_elapsed_ms}ms for \
                 {fill_count} fill(s) 窶・exceeds R1 p95 budget (100ms). Consider non-blocking fallback.",
                fill_count = fills.len()
            );
        }

        // observation 讒狗ｯ会ｼ・ession 縺ｯ Active 縺ｮ縺ｯ縺夲ｼ・

        let observation = match self.build_step_observation(200) {
            Some(obs) => obs,
            None => {
                return (
                    500,
                    r#"{"error":"observation build failed: session not active"}"#.to_string(),
                );
            }
        };

        // 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ E: fill.order_id 縺九ｉ agent_session_state 繧帝・ｼ輔″縺励※
        // client_order_id 繧貞沂繧√ｋ縲ゆｻ也ｵ瑚ｷｯ・・I 繝ｪ繝｢繧ｳ繝ｳ `/api/replay/order`・臥匱豕ｨ縺ｮ fill
        // 縺ｯ agent 縺ｮ map 縺ｫ辟｡縺・◆繧・None 縺ｮ縺ｾ縺ｾ・郁ｨｭ險磯壹ｊ・峨・        // 荳紋ｻ｣蜷梧悄縺ｯ繝上Φ繝峨Λ髢句ｧ区凾縺ｫ貂医ｓ縺ｧ縺・ｋ縺溘ａ縲√％縺薙〒縺ｯ蜀榊叙蠕励＠縺ｪ縺・        // ・・tep 蜀・〒 generation 繧貞､峨∴繧句・逅・・蟄伜惠縺励↑縺・ｼ峨・
        let step_fills: Vec<StepFill> = fills
            .iter()
            .map(|f| {
                let client_order_id = self
                    .agent_session_state
                    .client_order_id_for(&f.order_id)
                    .map(|c| c.as_str().to_string());
                StepFill::from_event(f, client_order_id)
            })
            .collect();

        let resp = StepResponse::new(new_time, reached_end, observation, step_fills)
            .with_updated_narrative_ids(updated_narrative_ids);

        // R1 險域ｸｬ逕ｨ: 蜈ｨ菴薙・ step 繝上Φ繝峨Λ蜃ｦ逅・凾髢薙・

        log::debug!(
            "agent_session_step: total {total_ms}ms (narrative sync {narrative_elapsed_ms}ms)",
            total_ms = overall_start.elapsed().as_millis()
        );

        match resp.to_json_string() {
            Ok(json) => (200, json),
            Err(e) => (
                500,
                serde_json::json!({"error": format!("serialize failed: {e}")}).to_string(),
            ),
        }
    }

    fn get_state_json(&self, limit: usize) -> String {
        match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => {
                let now_ms = clock.now_ms();
                let mut klines_out = Vec::new();
                let mut trades_out = Vec::new();

                let mut sorted: Vec<_> = active_streams.iter().collect();
                sorted.sort_by_cached_key(|s| format!("{s:?}"));

                for stream in sorted {
                    if let StreamKind::Kline {
                        ticker_info,
                        timeframe,
                    } = stream
                    {
                        let now_end = now_ms.saturating_add(1);
                        let all_klines = store.klines_in(stream, 0..now_end);
                        let slice = if all_klines.len() > limit {
                            &all_klines[all_klines.len() - limit..]
                        } else {
                            all_klines
                        };

                        if !slice.is_empty() {
                            let exchange_str =
                                format!("{:?}", ticker_info.ticker.exchange).replace(' ', "");
                            let label =
                                format!("{exchange_str}:{}:{timeframe}", ticker_info.ticker);
                            for k in slice {
                                klines_out.push(serde_json::json!({
                                    "stream": label,
                                    "time": k.time,
                                    "open": k.open.to_f64(),
                                    "high": k.high.to_f64(),
                                    "low": k.low.to_f64(),
                                    "close": k.close.to_f64(),
                                    "volume": k.volume.total().to_f64(),
                                }));
                            }
                        }

                        let trade_stream = StreamKind::Trades {
                            ticker_info: *ticker_info,
                        };
                        let trade_start =
                            now_ms.saturating_sub(crate::replay::controller::api::TRADE_WINDOW_MS);
                        let all_trades = store.trades_in(&trade_stream, trade_start..now_ms + 1);
                        let trade_slice = if all_trades.len() > limit {
                            &all_trades[all_trades.len() - limit..]
                        } else {
                            all_trades
                        };
                        if !trade_slice.is_empty() {
                            let exchange_str =
                                format!("{:?}", ticker_info.ticker.exchange).replace(' ', "");
                            let label = format!("{exchange_str}:{}:Trades", ticker_info.ticker);
                            let items: Vec<serde_json::Value> = trade_slice
                                .iter()
                                .map(|t| {
                                    serde_json::json!({
                                        "time": t.time,
                                        "price": t.price.to_f64(),
                                        "qty": t.qty.to_f64(),
                                        "is_sell": t.is_sell,
                                    })
                                })
                                .collect();
                            trades_out.push(serde_json::json!({"stream": label, "trades": items}));
                        }
                    }
                }

                serde_json::json!({
                    "current_time_ms": now_ms,
                    "klines": klines_out,
                    "trades": trades_out,
                })
                .to_string()
            }
            _ => r#"{"error":"replay not active"}"#.to_string(),
        }
    }

    fn get_portfolio_json(&self) -> String {
        let current_price = self.last_close_price().unwrap_or(0.0);
        let snapshot: PortfolioSnapshot = self.virtual_engine.portfolio_snapshot(current_price);
        serde_json::to_string(&snapshot)
            .unwrap_or_else(|e| format!(r#"{{"error":"serialize failed: {e}"}}"#))
    }

    fn get_orders_json(&self) -> String {
        let orders = self.virtual_engine.get_orders();
        serde_json::json!({"orders": orders}).to_string()
    }

    fn place_order_json(
        &mut self,
        ticker: &str,
        side: &str,
        qty: f64,
        order_type: &str,
        limit_price: Option<f64>,
    ) -> String {
        let pos_side = match side {
            "buy" => crate::replay::virtual_exchange::PositionSide::Long,
            "sell" => crate::replay::virtual_exchange::PositionSide::Short,
            _ => {
                return serde_json::json!({"error": "invalid side"}).to_string();
            }
        };
        let vot = match order_type {
            "market" => VirtualOrderType::Market,
            "limit" => {
                let lp = match limit_price {
                    Some(p) => p,
                    None => {
                        return serde_json::json!({"error": "limit_price required for limit order"})
                            .to_string();
                    }
                };
                VirtualOrderType::Limit { price: lp }
            }
            _ => {
                return serde_json::json!({"error": "invalid order_type"}).to_string();
            }
        };
        let now_ms = self.state.current_time();
        let order = VirtualOrder {
            order_id: uuid::Uuid::new_v4().to_string(),
            ticker: ticker.to_string(),
            side: pos_side,
            qty,
            order_type: vot,
            placed_time_ms: now_ms,
            status: crate::replay::virtual_exchange::VirtualOrderStatus::Pending,
        };
        let id = self.virtual_engine.place_order(order);
        serde_json::json!({"ok": true, "order_id": id, "status": "pending"}).to_string()
    }

    /// 繧｢繧ｯ繝・ぅ繝悶そ繝・す繝ｧ繝ｳ縺ｮ譛譁ｰ close 萓｡譬ｼ繧定ｿ斐☆縲・    
    fn last_close_price(&self) -> Option<f64> {
        if let ReplaySession::Active {
            clock,
            store,
            active_streams,
        } = &self.state.session
        {
            let now_ms = clock.now_ms();
            for stream in active_streams {
                if matches!(stream, StreamKind::Kline { .. }) {
                    let klines = store.klines_in(stream, 0..now_ms + 1);
                    if let Some(last) = klines.last() {
                        return Some(last.close.to_f64());
                    }
                }
            }
        }
        None
    }

    /// 蜈ｨ繝壹う繝ｳ縺ｮ譛蟆上ち繧､繝繝輔Ξ繝ｼ繝・・s・峨ｒ霑斐☆縲Ｔtep forward/backward 縺ｮ繧ｹ繝・ャ繝怜ｹ・↓菴ｿ縺・・    
    fn min_step_ms(&self) -> u64 {
        self.panes
            .iter()
            .map(|p| p.timeframe.to_milliseconds())
            .min()
            .unwrap_or(60_000)
    }

    fn list_panes_json(&self) -> String {
        let panes: Vec<serde_json::Value> = self
            .panes
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id.to_string(),
                    "ticker": p.ticker,
                    "timeframe": format!("{}", p.timeframe),
                    "streams_ready": true,
                })
            })
            .collect();
        serde_json::json!({ "panes": panes }).to_string()
    }

    fn split_pane(&mut self, pane_id: uuid::Uuid) -> String {
        let source = self.panes.iter().find(|p| p.id == pane_id);
        let (ticker, timeframe) = match source {
            Some(p) => (p.ticker.clone(), p.timeframe),
            None => {
                return serde_json::json!({
                    "ok": false,
                    "error": format!("pane not found: {pane_id}")
                })
                .to_string();
            }
        };
        let new_pane = HeadlessPane {
            id: uuid::Uuid::new_v4(),
            ticker,
            timeframe,
        };
        let new_id = new_pane.id;
        self.panes.push(new_pane);
        serde_json::json!({ "ok": true, "new_pane_id": new_id.to_string() }).to_string()
    }

    fn close_pane(&mut self, pane_id: uuid::Uuid) -> String {
        if self.panes.len() <= 1 {
            return serde_json::json!({ "ok": false, "error": "cannot close the last pane" })
                .to_string();
        }
        let before = self.panes.len();
        self.panes.retain(|p| p.id != pane_id);
        if self.panes.len() < before {
            serde_json::json!({ "ok": true }).to_string()
        } else {
            serde_json::json!({ "ok": false, "error": "pane not found" }).to_string()
        }
    }

    fn set_pane_ticker(&mut self, pane_id: uuid::Uuid, ticker: String) -> String {
        let Some(idx) = self.panes.iter().position(|p| p.id == pane_id) else {
            return serde_json::json!({ "ok": false, "error": "pane not found" }).to_string();
        };
        self.panes[idx].ticker = ticker;

        // Only reset the clock for the primary (first) pane. Secondary panes added via split
        // are label-only in headless mode; changing their ticker must not interrupt playback.
        if idx == 0 {
            if let crate::replay::ReplaySession::Active { clock, .. } = &mut self.state.session {
                let start = clock.full_range().start;
                clock.seek(start);
            }
            self.virtual_engine.reset();
            self.virtual_engine.mark_session_reset();
        }
        serde_json::json!({ "ok": true }).to_string()
    }

    fn set_pane_timeframe(&mut self, pane_id: uuid::Uuid, timeframe_str: &str) -> String {
        let tf = match parse_timeframe_str(timeframe_str) {
            Ok(t) => t,
            Err(e) => {
                return serde_json::json!({ "ok": false, "error": e }).to_string();
            }
        };
        let Some(idx) = self.panes.iter().position(|p| p.id == pane_id) else {
            return serde_json::json!({ "ok": false, "error": "pane not found" }).to_string();
        };
        self.panes[idx].timeframe = tf;
        // Mirror set_pane_ticker: only reset the clock for the primary pane.
        if idx == 0
            && let crate::replay::ReplaySession::Active { clock, .. } = &mut self.state.session
        {
            let start = clock.full_range().start;
            clock.seek(start);
        }
        serde_json::json!({ "ok": true }).to_string()
    }

    /// `StepClock::now_ms()` 繧貞叙蠕励☆繧九よ悴髢句ｧ九↑繧・0縲・    
    fn now_ms(&self) -> i64 {
        use crate::replay::ReplaySession;
        match &self.state.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                clock.now_ms() as i64
            }
            _ => 0,
        }
    }

    /// 繝翫Λ繝・ぅ繝悶さ繝槭Φ繝峨ｒ繧ｵ繝ｼ繝薙せ繝ｬ繧､繝､繝ｼ縺ｫ蟋碑ｭｲ縺吶ｋ縲・    
    async fn handle_narrative_command(
        &self,
        cmd: crate::replay_api::NarrativeCommand,
        reply: crate::replay_api::ReplySender,
    ) {
        use crate::narrative::service;
        use crate::replay_api::NarrativeCommand;
        let now_ms = self.now_ms();
        let created_at_ms = chrono::Utc::now().timestamp_millis();
        let (status, body) = match cmd {
            NarrativeCommand::Create(req) => {
                service::create_narrative(
                    &self.narrative_store,
                    &self.snapshot_store,
                    *req,
                    now_ms,
                    created_at_ms,
                )
                .await
            }
            NarrativeCommand::List(q) => service::list_narratives(&self.narrative_store, q).await,
            NarrativeCommand::Get { id } => service::get_narrative(&self.narrative_store, id).await,
            NarrativeCommand::GetSnapshot { id } => {
                service::get_narrative_snapshot(&self.narrative_store, &self.snapshot_store, id)
                    .await
            }
            NarrativeCommand::Patch { id, public } => {
                service::patch_narrative(&self.narrative_store, id, public).await
            }
            NarrativeCommand::StorageStats => service::storage_stats(&self.narrative_store).await,
            NarrativeCommand::Orphans => {
                service::orphans(&self.narrative_store, self.data_root.clone()).await
            }
        };
        reply.send_status(status, body);
    }

    /// API 繧ｳ繝槭Φ繝峨ｒ蜃ｦ逅・＠縲ヽeplySender 縺ｧ繝ｬ繧ｹ繝昴Φ繧ｹ繧定ｿ斐☆縲・    
    async fn handle_command(&mut self, cmd: ApiCommand, reply: crate::replay_api::ReplySender) {
        use crate::replay::ReplayCommand;

        match cmd {
            ApiCommand::Replay(ReplayCommand::GetStatus) => {
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::Toggle { init_range }) => {
                let result = if let Some((start, end)) = init_range {
                    self.play(&start, &end)
                } else if matches!(self.state.mode, ReplayMode::Replay) {
                    self.enter_live_mode();
                    Ok(self.get_status_json())
                } else {
                    self.enter_replay_mode();
                    Ok(self.get_status_json())
                };

                match result {
                    Ok(body) => reply.send(body),
                    Err(err) => {
                        reply.send_status(400, serde_json::json!({ "error": err }).to_string())
                    }
                }
            }
            ApiCommand::Replay(ReplayCommand::SetMode { mode }) => {
                if mode.eq_ignore_ascii_case("live") {
                    self.enter_live_mode();
                } else if mode.eq_ignore_ascii_case("replay") {
                    self.enter_replay_mode();
                }
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::SaveState) => {
                // headless 縺ｧ縺ｯ菫晏ｭ倅ｸ崎ｦ・                reply.send(r#"{"ok":true}"#.to_string());
            }
            ApiCommand::VirtualExchange(VirtualExchangeCommand::GetState) => {
                match &self.state.session {
                    ReplaySession::Active { .. } => reply.send(self.get_state_json(200)),
                    _ => reply.send_status(400, r#"{"error":"replay not active"}"#.to_string()),
                }
            }
            ApiCommand::VirtualExchange(VirtualExchangeCommand::GetPortfolio) => {
                reply.send(self.get_portfolio_json());
            }
            ApiCommand::VirtualExchange(VirtualExchangeCommand::GetOrders) => {
                reply.send(self.get_orders_json());
            }
            ApiCommand::VirtualExchange(VirtualExchangeCommand::PlaceOrder {
                ticker,
                side,
                qty,
                order_type,
                limit_price,
            }) => {
                reply.send(self.place_order_json(&ticker, &side, qty, &order_type, limit_price));
            }
            ApiCommand::Pane(crate::replay_api::PaneCommand::ListPanes) => {
                reply.send(self.list_panes_json());
            }
            ApiCommand::Pane(crate::replay_api::PaneCommand::Split { pane_id, .. }) => {
                reply.send(self.split_pane(pane_id));
            }
            ApiCommand::Pane(crate::replay_api::PaneCommand::Close { pane_id }) => {
                reply.send(self.close_pane(pane_id));
            }
            ApiCommand::Pane(crate::replay_api::PaneCommand::SetTicker { pane_id, ticker }) => {
                reply.send(self.set_pane_ticker(pane_id, ticker));
            }
            ApiCommand::Pane(crate::replay_api::PaneCommand::SetTimeframe {
                pane_id,
                timeframe,
            }) => {
                reply.send(self.set_pane_timeframe(pane_id, &timeframe));
            }
            ApiCommand::Narrative(cmd) => {
                self.handle_narrative_command(cmd, reply).await;
            }
            ApiCommand::AgentSession(crate::replay_api::AgentSessionCommand::Step {
                session_id: _,
            }) => {
                // session_id 縺ｯ route 螻､縺ｧ "default" 縺ｫ髯仙ｮ壽ｸ医∩・磯撼 default 縺ｯ 501 縺ｧ譌｢縺ｫ諡貞凄・峨・
                let (status, body) = self.agent_session_step().await;
                reply.send_status(status, body);
            }
            ApiCommand::AgentSession(crate::replay_api::AgentSessionCommand::PlaceOrder {
                session_id: _,
                request,
            }) => {
                let (status, body) = self.agent_session_place_order(*request);
                reply.send_status(status, body);
            }
            ApiCommand::AgentSession(crate::replay_api::AgentSessionCommand::Advance {
                session_id: _,
                request,
            }) => {
                let (status, body) = self.agent_session_advance(*request).await;
                reply.send_status(status, body);
            }
            ApiCommand::AgentSession(crate::replay_api::AgentSessionCommand::RewindToStart {
                session_id: _,
                init_range,
            }) => {
                let (status, body) = self.agent_session_rewind(init_range).await;
                reply.send_status(status, body);
            }
            // headless 縺ｧ譛ｪ蟇ｾ蠢懊・繧ｳ繝槭Φ繝峨・ 501 繧定ｿ斐☆
            ApiCommand::Pane(_)
            | ApiCommand::Auth(_)
            | ApiCommand::FetchBuyingPower
            | ApiCommand::TachibanaNewOrder { .. }
            | ApiCommand::FetchTachibanaOrders { .. }
            | ApiCommand::FetchTachibanaOrderDetail { .. }
            | ApiCommand::TachibanaCorrectOrder { .. }
            | ApiCommand::TachibanaOrderCancel { .. }
            | ApiCommand::FetchTachibanaHoldings { .. } => {
                reply.send_status(
                    501,
                    r#"{"error":"not implemented in headless mode"}"#.to_string(),
                );
            }
            #[cfg(debug_assertions)]
            ApiCommand::Test(_) => {
                reply.send_status(
                    501,
                    r#"{"error":"test commands not available in headless mode"}"#.to_string(),
                );
            }
        }
    }
}

// 笏笏 繧ｨ繝ｳ繝医Μ繝ｼ繝昴う繝ｳ繝・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

/// headless 繝｢繝ｼ繝峨・繝｡繧､繝ｳ繝ｫ繝ｼ繝励・/// `--headless` 繝輔Λ繧ｰ縺梧ｸ｡縺輔ｌ縺溘→縺・`main()` 縺九ｉ蜻ｼ縺ｰ繧後ｋ縲・
pub async fn run(args: &[String]) {
    let headless_args = match parse_headless_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("headless mode error: {e}");
            std::process::exit(1);
        }
    };

    let ticker = match parse_ticker_str(&headless_args.ticker) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("headless mode ticker error: {e}");
            std::process::exit(1);
        }
    };

    let timeframe = match parse_timeframe_str(&headless_args.timeframe) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("headless mode timeframe error: {e}");
            std::process::exit(1);
        }
    };

    log::info!(
        "headless mode: ticker={}, timeframe={:?}",
        headless_args.ticker,
        timeframe
    );

    // kline 繝ｭ繝ｼ繝臥ｵ先棡縺ｮ騾夂衍繝√Ε繝阪Ν
    let (load_tx, mut load_rx) = tokio::sync::mpsc::channel::<LoadResult>(8);

    // API 繧ｳ繝槭Φ繝峨メ繝｣繝阪Ν
    // NOTE: replay_api::start_server 縺ｯ GUI 繝｢繝ｼ繝峨→繧ｷ繧ｰ繝阪メ繝｣繧貞・譛峨☆繧九◆繧・    // futures::channel::mpsc::Sender 繧定ｦ∵ｱゅ☆繧九Ｕokio::sync::mpsc 縺ｧ縺ｯ縺ｪ縺・    // futures 繝吶・繧ｹ縺ｮ繝√Ε繝阪Ν繧剃ｽｿ縺・・縺ｯ縺薙％縺檎炊逕ｱ縲・
    let (api_tx, mut api_rx) = futures::channel::mpsc::channel::<ApiMessage>(32);

    // HTTP API 繧ｵ繝ｼ繝舌・繧貞挨繧ｿ繧ｹ繧ｯ縺ｧ襍ｷ蜍・

    tokio::spawn(async move {
        crate::replay_api::start_server(api_tx).await;
    });

    let mut engine = HeadlessEngine::new(ticker, timeframe, load_tx);

    // ADR-0001 ﾂｧ2 閾ｪ蜍募・逕滓ｩ滓ｧ九・蜈ｨ蟒・
    // 莉･蜑阪・ 100ms interval 縺ｧ `engine.tick()` 繧堤匱轣ｫ縺輔○ Playing 荳ｭ縺ｮ replay 繧定・蜍暮ｲ陦後＆縺帙※縺・◆縺後・    // agent session API (`/api/agent/session/:id/{step,advance}`) 縺ｸ縺ｮ荳譛ｬ蛹悶↓莨ｴ縺・炎髯､縲・
    log::info!("headless event loop started (API port: {})", {
        std::env::var("FLOWSURFACE_API_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(9876)
    });

    loop {
        tokio::select! {
            biased;

            // kline 繝ｭ繝ｼ繝牙ｮ御ｺ・

            Some(result) = load_rx.recv() => {
                engine.handle_load_result(result);
            }

            // API 繧ｳ繝槭Φ繝牙女菫｡
            Some((cmd, reply)) = api_rx.next() => {
                engine.handle_command(cmd, reply).await;
            }
        }
    }
}

// 笏笏 繝・せ繝・笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

#[cfg(test)]
mod tests {
    use super::*;

    // 笏笏 parse_headless_args 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_args_returns_ticker_and_explicit_timeframe() {
        let a = args(&[
            "flowsurface",
            "--headless",
            "--ticker",
            "HyperliquidLinear:BTC",
            "--timeframe",
            "H1",
        ]);
        let result = parse_headless_args(&a).unwrap();
        assert_eq!(result.ticker, "HyperliquidLinear:BTC");
        assert_eq!(result.timeframe, "H1");
    }

    #[test]
    fn parse_args_defaults_timeframe_to_m1() {
        let a = args(&[
            "flowsurface",
            "--headless",
            "--ticker",
            "BinanceLinear:BTCUSDT",
        ]);
        let result = parse_headless_args(&a).unwrap();
        assert_eq!(result.timeframe, "M1");
    }

    #[test]
    fn parse_args_returns_error_when_ticker_missing() {
        let a = args(&["flowsurface", "--headless"]);
        let result = parse_headless_args(&a);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("--ticker"));
    }

    #[test]
    fn parse_args_ignores_unknown_flags() {
        let a = args(&[
            "flowsurface",
            "--headless",
            "--unknown",
            "value",
            "--ticker",
            "HyperliquidLinear:BTC",
        ]);
        let result = parse_headless_args(&a).unwrap();
        assert_eq!(result.ticker, "HyperliquidLinear:BTC");
    }

    // 笏笏 parse_ticker_str 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    #[test]
    fn parse_ticker_str_hyperliquid_linear_btc() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        assert_eq!(
            ticker.exchange,
            exchange::adapter::Exchange::HyperliquidLinear
        );
        assert_eq!(ticker.to_string(), "BTC");
    }

    #[test]
    fn parse_ticker_str_binance_linear_btcusdt() {
        let ticker = parse_ticker_str("BinanceLinear:BTCUSDT").unwrap();
        assert_eq!(ticker.exchange, exchange::adapter::Exchange::BinanceLinear);
        assert_eq!(ticker.to_string(), "BTCUSDT");
    }

    #[test]
    fn parse_ticker_str_returns_error_for_missing_colon() {
        let result = parse_ticker_str("BinanceBTCUSDT");
        assert!(result.is_err());
    }

    #[test]
    fn parse_ticker_str_returns_error_for_unknown_exchange() {
        let result = parse_ticker_str("UnknownExchange:BTCUSDT");
        assert!(result.is_err());
    }

    // 笏笏 parse_timeframe_str 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    #[test]
    fn parse_timeframe_str_m1() {
        assert_eq!(parse_timeframe_str("M1").unwrap(), Timeframe::M1);
    }

    #[test]
    fn parse_timeframe_str_lowercase_1m_alias() {
        assert_eq!(parse_timeframe_str("1m").unwrap(), Timeframe::M1);
    }

    #[test]
    fn parse_timeframe_str_h1() {
        assert_eq!(parse_timeframe_str("H1").unwrap(), Timeframe::H1);
    }

    #[test]
    fn parse_timeframe_str_d1() {
        assert_eq!(parse_timeframe_str("D1").unwrap(), Timeframe::D1);
    }

    #[test]
    fn parse_timeframe_str_returns_error_for_unknown() {
        let result = parse_timeframe_str("X99");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown timeframe"));
    }

    // 笏笏 HeadlessEngine::play 蠑墓焚繝舌Μ繝・・繧ｷ繝ｧ繝ｳ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    #[tokio::test]
    async fn engine_play_returns_error_for_invalid_date_range() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let timeframe = Timeframe::M1;
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);
        let result = engine.play("not-a-date", "2026-01-31 23:59");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn engine_play_returns_error_when_start_after_end() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let timeframe = Timeframe::M1;
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);
        let result = engine.play("2026-01-31 00:00", "2026-01-01 00:00");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn engine_play_transitions_to_loading_state() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let timeframe = Timeframe::M1;
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);
        let result = engine.play("2026-01-01 00:00", "2026-01-01 01:00");
        assert!(result.is_ok());
        let json = result.unwrap();
        assert!(json.contains("\"status\":\"loading\""));
        assert!(engine.state.is_loading());
    }

    #[tokio::test]
    async fn engine_play_resets_virtual_engine() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let timeframe = Timeframe::M1;
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);
        engine
            .virtual_engine
            .place_order(crate::replay::virtual_exchange::VirtualOrder {
                order_id: "test".to_string(),
                ticker: "BTC".to_string(),
                side: crate::replay::virtual_exchange::PositionSide::Long,
                qty: 0.1,
                order_type: VirtualOrderType::Market,
                placed_time_ms: 0,
                status: crate::replay::virtual_exchange::VirtualOrderStatus::Pending,
            });
        assert_eq!(engine.virtual_engine.get_orders().len(), 1);
        let _ = engine.play("2026-01-01 00:00", "2026-01-01 01:00");
        assert_eq!(engine.virtual_engine.get_orders().len(), 0);
    }

    // 笏笏 step_forward / pause / resume (removed) 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    fn make_active_engine_with_klines(
        start_ms: u64,
        end_ms: u64,
        step_ms: u64,
        initial_time: u64,
    ) -> HeadlessEngine {
        use exchange::{Volume, unit::MinTicksize};

        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let timeframe = Timeframe::M1;
        let ticker_info = TickerInfo::new(ticker, 0.01, 0.001, None);
        let stream = StreamKind::Kline {
            ticker_info,
            timeframe,
        };

        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);

        // Active 繧ｻ繝・す繝ｧ繝ｳ繧呈焔蜍輔〒讒狗ｯ峨☆繧・

        let mut clock = StepClock::new(start_ms, end_ms, step_ms);
        clock.seek(initial_time);

        let mut store = EventStore::new();
        // start, start+step 縺ｮ 2 譛ｬ縺ｮ kline 繧呈諺蜈･縺吶ｋ
        let mk = |t: u64| {
            exchange::Kline::new(
                t,
                100.0,
                101.0,
                99.0,
                100.5,
                Volume::empty_total(),
                MinTicksize::from(0.01),
            )
        };
        let mut t = start_ms;
        while t <= end_ms {
            store.ingest_loaded(
                stream,
                start_ms..end_ms + 1,
                crate::replay::store::LoadedData {
                    klines: vec![mk(t)],
                    trades: vec![],
                },
            );
            t += step_ms;
        }

        let mut active_streams = HashSet::new();
        active_streams.insert(stream);

        engine.state.session = ReplaySession::Active {
            clock,
            store,
            active_streams,
        };
        engine
    }

    #[test]
    fn get_state_returns_error_when_not_active() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let json = engine.get_state_json(50);
        assert!(json.contains("replay not active"));
    }

    // 笏笏 handle_load_result: Loading 竊・Active 驕ｷ遘ｻ 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    #[test]
    fn handle_load_result_transitions_loading_to_active_when_single_stream() {
        use exchange::{Volume, unit::MinTicksize};

        let ticker = parse_ticker_str("BinanceLinear:BTCUSDT").unwrap();
        let timeframe = Timeframe::M1;
        let ticker_info = TickerInfo::new(ticker, 0.01, 0.001, None);
        let stream = StreamKind::Kline {
            ticker_info,
            timeframe,
        };

        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, timeframe, tx);

        // Loading 迥ｶ諷九ｒ謇句虚縺ｧ險ｭ螳・

        let clock = StepClock::new(0, 3_600_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(stream);
        engine.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams,
        };

        // 繝繝溘・ kline 繧定ｿ斐☆
        let dummy_kline = exchange::Kline::new(
            60_000,
            100.0,
            101.0,
            99.0,
            100.5,
            Volume::empty_total(),
            MinTicksize::from(0.01),
        );
        let result = LoadResult::Ok {
            stream,
            range: 0..3_600_000,
            klines: vec![dummy_kline],
        };

        engine.handle_load_result(result);

        assert!(
            matches!(engine.state.session, ReplaySession::Active { .. }),
            "should transition to Active"
        );
    }

    #[test]
    fn handle_load_result_err_resets_session_to_idle() {
        let ticker = parse_ticker_str("BinanceLinear:BTCUSDT").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);

        let clock = StepClock::new(0, 3_600_000, 60_000);
        engine.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams: HashSet::new(),
        };

        engine.handle_load_result(LoadResult::Err("network error".to_string()));
        assert!(matches!(engine.state.session, ReplaySession::Idle));
    }

    // 笏笏 agent_session_step (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ C) 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    #[tokio::test]
    async fn agent_session_step_returns_404_when_session_idle() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let (status, body) = engine.agent_session_step().await;
        assert_eq!(status, 404, "got body: {body}");
        assert!(
            body.contains("session not started"),
            "body missing expected error: {body}"
        );
        assert!(body.contains("\"hint\""), "body missing hint field: {body}");
    }

    #[tokio::test]
    async fn agent_session_step_returns_200_with_required_keys() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (status, body) = engine.agent_session_step().await;
        assert_eq!(status, 200, "got body: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        for key in [
            "clock_ms",
            "reached_end",
            "observation",
            "fills",
            "updated_narrative_ids",
        ] {
            assert!(
                v.get(key).is_some(),
                "missing top-level key {key} in {body}"
            );
        }
    }

    #[tokio::test]
    async fn agent_session_step_advances_clock_by_one_bar() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (_status, body) = engine.agent_session_step().await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["clock_ms"].as_u64().unwrap(),
            start_ms + step_ms,
            "expected advance by one bar"
        );
        assert_eq!(v["reached_end"], false);
    }

    #[tokio::test]
    async fn agent_session_step_sets_reached_end_at_range_boundary() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let end_ms = start_ms + step_ms * 2;
        // 蛻晄悄菴咲ｽｮ繧・end_ms 縺ｫ繧ｻ繝・ヨ 窶・縺薙ｌ莉･荳企ｲ繧√↑縺・・
        let mut engine = make_active_engine_with_klines(start_ms, end_ms, step_ms, end_ms);
        let (status, body) = engine.agent_session_step().await;
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["reached_end"], true, "body: {body}");
        assert_eq!(
            v["clock_ms"].as_u64().unwrap(),
            end_ms,
            "clock must not advance past end"
        );
    }

    #[tokio::test]
    async fn agent_session_step_returns_fills_inline_when_market_order_exists() {
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "ord_1".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (status, body) = engine.agent_session_step().await;
        assert_eq!(status, 200, "got body: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let fills = v["fills"].as_array().expect("fills must be array");
        assert_eq!(fills.len(), 1, "expected one fill, got: {body}");
        assert_eq!(fills[0]["order_id"], "ord_1");
        assert_eq!(fills[0]["side"], "buy");
        assert!(
            fills[0]["client_order_id"].is_null(),
            "subphase C: client_order_id must be null"
        );
    }

    #[tokio::test]
    async fn agent_session_step_observation_includes_portfolio() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (_status, body) = engine.agent_session_step().await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["observation"]["portfolio"]["cash"].as_f64().unwrap(),
            1_000_000.0,
            "default initial cash"
        );
    }

    #[tokio::test]
    async fn agent_session_step_updated_narrative_ids_empty_when_no_linked_narrative() {
        // narrative 縺・linked 縺輔ｌ縺ｦ縺・↑縺・ｴ蜷医・遨ｺ驟榊・・医し繝悶ヵ繧ｧ繝ｼ繧ｺ D・峨・
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (_status, body) = engine.agent_session_step().await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            v["updated_narrative_ids"]
                .as_array()
                .map(|a| a.is_empty())
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn step_updates_narrative_synchronously() {
        use crate::narrative::model::{Narrative, NarrativeAction, NarrativeSide, SnapshotRef};
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        // 豌ｸ邯・SQLite 縺ｮ闢・ｩ阪ｒ驕ｿ縺代ｋ縺溘ａ縲√％縺ｮ繝・せ繝亥ｮ溯｡悟崋譛峨・ UUID 繧・order_id 縺ｫ謗｡逕ｨ縲・

        let unique_order_id = format!("ord_d_sync_{}", uuid::Uuid::new_v4());
        let narrative_id = uuid::Uuid::new_v4();
        let narrative = Narrative {
            id: narrative_id,
            agent_id: "test_agent".to_string(),
            uagent_address: None,
            timestamp_ms: start_ms as i64,
            ticker: "BTCUSDT".to_string(),
            timeframe: "M1".to_string(),
            snapshot_ref: SnapshotRef {
                path: std::path::PathBuf::from("narratives/snapshots/test.json.gz"),
                size_bytes: 42,
                sha256: "a".repeat(64),
            },
            reasoning: "test".to_string(),
            action: NarrativeAction {
                side: NarrativeSide::Buy,
                qty: 0.1,
                price: 100.5,
            },
            confidence: 0.5,
            outcome: None,
            linked_order_id: Some(unique_order_id.clone()),
            public: false,
            created_at_ms: start_ms as i64,
            idempotency_key: None,
        };
        engine.narrative_store.insert(narrative).await.unwrap();

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: unique_order_id.clone(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (status, body) = engine.agent_session_step().await;
        assert_eq!(status, 200, "body: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();

        // 1. updated_narrative_ids 縺ｫ縺薙・ narrative 縺ｮ UUID 縺悟性縺ｾ繧後ｋ縲・

        let ids = v["updated_narrative_ids"]
            .as_array()
            .expect("must be array");
        assert_eq!(ids.len(), 1, "expected one updated id, got {body}");
        assert_eq!(ids[0].as_str().unwrap(), narrative_id.to_string());

        // 2. step 繝ｬ繧ｹ繝昴Φ繧ｹ霑泌唆譎らせ縺ｧ outcome 縺・DB 縺ｫ譖ｸ縺崎ｾｼ縺ｿ螳御ｺ・＠縺ｦ縺・ｋ
        //    ・・olling 荳崎ｦ・= 蜷梧悄 await 縺ｮ菫晁ｨｼ・峨・
        let stored = engine
            .narrative_store
            .get(narrative_id)
            .await
            .unwrap()
            .expect("narrative exists");
        let outcome = stored.outcome.expect("outcome must be populated");
        assert!(outcome.fill_price > 0.0);
        assert_eq!(outcome.fill_time_ms, (start_ms + step_ms) as i64);
    }

    // 笏笏 agent_session_place_order (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ E) 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    fn sample_order_request(
        cli_id: &str,
        qty: f64,
    ) -> crate::api::order_request::AgentOrderRequest {
        use crate::api::contract::{ClientOrderId, TickerContract};
        use crate::api::order_request::{AgentOrderRequest, AgentOrderSide, AgentOrderType};
        AgentOrderRequest {
            client_order_id: ClientOrderId::new(cli_id).unwrap(),
            ticker: TickerContract::new("HyperliquidLinear", "BTC"),
            side: AgentOrderSide::Buy,
            qty,
            order_type: AgentOrderType::Market {},
        }
    }

    #[tokio::test]
    async fn place_order_returns_404_when_session_idle() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let (status, body) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(status, 404, "body: {body}");
    }

    #[tokio::test]
    async fn place_order_returns_503_when_session_loading() {
        // StepClock / EventStore 縺ｯ super::* 邨檎罰縺ｧ譌｢縺ｫ import 貂医∩縲・
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        // Loading 繧ｻ繝・す繝ｧ繝ｳ繧呈焔蜍墓ｧ狗ｯ・
        let clock = StepClock::new(0, 3_600_000, 60_000);
        engine.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams: HashSet::new(),
        };
        let (status, body) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(status, 503, "body: {body}");
        assert!(body.contains("loading"));
    }

    #[tokio::test]
    async fn place_order_creates_new_order_and_returns_200() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (status, body) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        // First placement returns 200 and reports `idempotent_replay = false`.
        assert_eq!(status, 200, "body: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["client_order_id"], "cli_1");
        assert_eq!(v["idempotent_replay"], false);
        assert!(v["order_id"].is_string());
        assert_eq!(engine.virtual_engine.get_orders().len(), 1);
    }

    #[tokio::test]
    async fn place_order_returns_200_idempotent_replay_on_exact_rerun() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, b1) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s1, 200);
        let v1: serde_json::Value = serde_json::from_str(&b1).unwrap();
        let first_order_id = v1["order_id"].as_str().unwrap().to_string();

        // 蜷後§ client_order_id + 蜷後§ body 竊・200 + idempotent_replay: true
        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s2, 200, "body: {b2}");
        let v2: serde_json::Value = serde_json::from_str(&b2).unwrap();
        assert_eq!(v2["idempotent_replay"], true);
        assert_eq!(v2["order_id"], first_order_id);

        assert_eq!(engine.virtual_engine.get_orders().len(), 1);
    }

    #[tokio::test]
    async fn place_order_returns_409_conflict_on_different_body_same_client_order_id() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, _) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s1, 200);

        // 蜷後§ cli_1 縺ｧ qty 驕輔＞ 竊・409
        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.2));
        assert_eq!(s2, 409, "body: {b2}");
        let v: serde_json::Value = serde_json::from_str(&b2).unwrap();
        assert!(v["error"].as_str().unwrap().contains("conflict"));
        assert!(v["existing_order_id"].is_string());

        assert_eq!(engine.virtual_engine.get_orders().len(), 1);
    }

    #[tokio::test]
    async fn place_order_different_client_order_ids_both_accepted() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, _) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        let (s2, _) = engine.agent_session_place_order(sample_order_request("cli_2", 0.2));
        assert_eq!(s1, 200);
        assert_eq!(s2, 200);
        assert_eq!(engine.virtual_engine.get_orders().len(), 2);
    }

    #[tokio::test]
    async fn place_order_map_cleared_after_session_lifecycle_event() {
        // ADR-0001 荳榊､画擅莉ｶ: UI 繝ｪ繝｢繧ｳ繝ｳ /play 遲峨′襍ｰ繧九→ agent map 縺後け繝ｪ繧｢縺輔ｌ繧九・
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, _) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s1, 200);

        engine.virtual_engine.mark_session_reset();

        // 蜷後§ cli_1 縺ｧ qty 驕輔＞ 竊・譁ｰ隕丞女莉假ｼ医け繝ｪ繧｢蠕後↑縺ｮ縺ｧ 201・峨・

        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.2));
        assert_eq!(
            s2, 200,
            "after lifecycle event, same client_order_id can be reused: {b2}"
        );
    }

    #[tokio::test]
    async fn step_fill_carries_client_order_id_when_placed_via_agent_api() {
        // 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ E 縺ｮ驥崎ｦ∽ｸ榊､画擅莉ｶ: agent API 縺ｧ逋ｺ豕ｨ縺励◆豕ｨ譁・・ fill 縺ｯ
        // step 繝ｬ繧ｹ繝昴Φ繧ｹ縺ｮ fills 驟榊・縺ｧ client_order_id 繧定ｿ斐☆縲・
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, _) = engine.agent_session_place_order(sample_order_request("cli_trace", 0.1));
        assert_eq!(s1, 200);

        let (_status, body) = engine.agent_session_step().await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let fills = v["fills"].as_array().expect("fills array");
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0]["client_order_id"], "cli_trace");
    }

    // 笏笏 agent_session_advance (Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ G) 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

    fn make_advance_request(
        until_ms: u64,
        stop_on: Vec<crate::api::advance_request::AdvanceStopCondition>,
        include_fills: bool,
    ) -> crate::api::advance_request::AgentAdvanceRequest {
        crate::api::advance_request::AgentAdvanceRequest {
            until_ms: crate::api::contract::EpochMs::from(until_ms),
            stop_on,
            include_fills,
        }
    }

    #[tokio::test]
    async fn advance_returns_404_when_session_idle() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let (status, body) = engine
            .agent_session_advance(make_advance_request(100, vec![], false))
            .await;
        assert_eq!(status, 404, "body: {body}");
    }

    #[tokio::test]
    async fn advance_until_ms_reaches_and_returns_until_reached_reason() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let end_ms = start_ms + step_ms * 10;
        let mut engine = make_active_engine_with_klines(start_ms, end_ms, step_ms, start_ms);

        let until_ms = start_ms + step_ms * 3;
        let (status, body) = engine
            .agent_session_advance(make_advance_request(until_ms, vec![], false))
            .await;
        assert_eq!(status, 200, "body: {body}");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["stopped_reason"], "until_reached");
        assert_eq!(v["clock_ms"].as_u64().unwrap(), until_ms);
        assert_eq!(v["ticks_advanced"].as_u64().unwrap(), 3);
    }

    #[tokio::test]
    async fn advance_reaches_end_returns_end_reason() {
        // until_ms 縺・range 邨らｫｯ繧医ｊ蜈医↑繧・End 縺ｧ蛛懈ｭ｢縲・
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let end_ms = start_ms + step_ms * 3;
        let mut engine = make_active_engine_with_klines(start_ms, end_ms, step_ms, start_ms);

        let (_, body) = engine
            .agent_session_advance(make_advance_request(end_ms + step_ms * 100, vec![], false))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["stopped_reason"], "end", "body: {body}");
    }

    #[tokio::test]
    async fn advance_reaches_non_aligned_end_exactly() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let end_ms = start_ms + step_ms * 2 + step_ms / 2;
        let mut engine = make_active_engine_with_klines(start_ms, end_ms, step_ms, start_ms);

        let (_, body) = engine
            .agent_session_advance(make_advance_request(end_ms + step_ms, vec![], false))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["stopped_reason"], "end", "body: {body}");
        assert_eq!(v["clock_ms"].as_u64().unwrap(), end_ms, "body: {body}");
    }

    #[tokio::test]
    async fn advance_stops_on_fill() {
        use crate::api::advance_request::AdvanceStopCondition;
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 10, step_ms, start_ms);

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "ord_stop".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (_, body) = engine
            .agent_session_advance(make_advance_request(
                start_ms + step_ms * 5,
                vec![AdvanceStopCondition::Fill],
                false,
            ))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["stopped_reason"], "fill", "body: {body}");
        assert_eq!(v["ticks_advanced"].as_u64().unwrap(), 1);
        assert_eq!(v["aggregate_fills"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn advance_default_excludes_fills_array_from_response() {
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "ord_nofills".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (_, body) = engine
            .agent_session_advance(make_advance_request(
                start_ms + step_ms * 2,
                vec![],
                false, // include_fills = false
            ))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            v.get("fills").is_none(),
            "fills must be omitted when include_fills=false: {body}"
        );
        assert_eq!(v["aggregate_fills"].as_u64().unwrap(), 1);
    }

    #[tokio::test]
    async fn advance_include_fills_true_populates_fills_array() {
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "ord_fills".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (_, body) = engine
            .agent_session_advance(make_advance_request(start_ms + step_ms * 2, vec![], true))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let fills = v["fills"].as_array().expect("fills array must be present");
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0]["order_id"], "ord_fills");
    }

    #[tokio::test]
    async fn advance_past_until_ms_returns_zero_ticks() {
        // until_ms <= 迴ｾ蝨ｨ譎ょ綾 縺ｮ蝣ｴ蜷医・ 0 tick 縺ｧ UntilReached縲ょｾ碁縺ｯ agent scope 螟悶・
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);
        let (_, body) = engine
            .agent_session_advance(make_advance_request(start_ms - 1000, vec![], false))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["ticks_advanced"].as_u64().unwrap(), 0);
        assert_eq!(v["stopped_reason"], "until_reached");
    }

    #[tokio::test]
    async fn advance_final_portfolio_reflects_post_advance_state() {
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 10, step_ms, start_ms);

        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "ord_pf".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: start_ms,
            status: VirtualOrderStatus::Pending,
        });

        let (_, body) = engine
            .agent_session_advance(make_advance_request(start_ms + step_ms * 3, vec![], false))
            .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let cash = v["final_portfolio"]["cash"].as_f64().unwrap();
        assert!(
            cash < 1_000_000.0,
            "cash should decrease after buy fill: {cash}"
        );
        let positions = v["final_portfolio"]["open_positions"].as_array().unwrap();
        assert_eq!(positions.len(), 1);
    }
}
