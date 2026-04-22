/// Headless モード — GUI なしで tokio ランタイム + HTTP API サーバーだけを動かす。
///
/// `flowsurface --headless --ticker HyperliquidLinear:BTC --timeframe M1` で起動する。
/// iced::daemon を一切起動しないため、Python SDK のような外部プログラムから
/// HTTP API (port 9876) 経由で高速に強化学習ループを回せる。
use std::{collections::HashSet, time::Instant};

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

// ── CLI 引数 ────────────────────────────────────────────────────────────────────

/// `--headless` 起動時に必要な CLI 引数。
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessArgs {
    pub ticker: String,
    pub timeframe: String,
}

/// `args` スライス（`std::env::args().collect()` の結果）から headless 用引数をパースする。
///
/// - `--ticker <ExchangeName:Symbol>` — 必須
/// - `--timeframe <TF>` — 省略時は `"M1"`
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

// ── ティッカー / タイムフレーム ─────────────────────────────────────────────────

/// "BinanceLinear:BTCUSDT" や "HyperliquidLinear:BTC" を `Ticker` にパースする。
pub fn parse_ticker_str(s: &str) -> Result<Ticker, String> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid ticker format: expected 'Exchange:Symbol', got '{s}'"
        ));
    }
    let exchange_str = parts[0];
    // "BinanceLinear" → "Binance Linear" to match main.rs::parse_ser_ticker.
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

/// "M1", "M5", "H1" 等を `Timeframe` にパースする。
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

// ── HeadlessEngine ─────────────────────────────────────────────────────────────

/// kline ロードタスクの完了通知。
#[allow(dead_code)] // サブフェーズ S でエンジンごと整理
enum LoadResult {
    Ok {
        stream: StreamKind,
        range: std::ops::Range<u64>,
        klines: Vec<exchange::Kline>,
    },
    Err(String),
}

/// headless モードのインメモリペイン。GUI ペインツリーの代替として ticker/timeframe を保持する。
struct HeadlessPane {
    id: uuid::Uuid,
    ticker: String,
    timeframe: Timeframe,
}

/// headless モード専用リプレイエンジン。
/// iced への依存を持たず、tokio 非同期タスクで動作する。
#[allow(dead_code)] // サブフェーズ S でフィールド整理
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
    /// Agent 専用 Replay API の "default" セッション state（Phase 4b-1 サブフェーズ E）。
    /// `client_order_id` 冪等性マップを持ち、`VirtualExchange::session_generation()`
    /// の変化で自動クリアされる。
    agent_session_state: crate::api::agent_session_state::AgentSessionState,
}

// NOTE: ADR-0001 §2 自動再生機構の廃止に伴い、
// HeadlessEngine の play / step_forward / step_backward / resume / pause 系メソッドは
// サブフェーズ M で match arm が削除されて orphaned 化している。
// サブフェーズ S（headless 重複実装削除）で構造体・メソッド・関連フィールドごと整理する。
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
        Self {
            state,
            virtual_engine: VirtualExchangeEngine::new(1_000_000.0),
            ticker,
            ticker_str,
            timeframe,
            load_tx,
            panes: vec![initial_pane],
            narrative_store: std::sync::Arc::new(
                crate::narrative::store::NarrativeStore::open_default()
                    .expect("failed to open narrative store"),
            ),
            snapshot_store: crate::narrative::snapshot_store::SnapshotStore::new(data_root.clone()),
            data_root,
            agent_session_state: crate::api::agent_session_state::AgentSessionState::new(),
        }
    }

    fn is_playing(&self) -> bool {
        self.state.is_playing()
    }

    /// `POST /api/replay/play {"start":"...","end":"..."}` を処理する。
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

        let mut clock = StepClock::new(start_ms, end_ms, step_ms);
        clock.set_waiting();

        let mut active_streams = HashSet::new();
        active_streams.insert(stream);

        self.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams,
        };
        self.state.range_input.start = start.to_string();
        self.state.range_input.end = end.to_string();
        self.virtual_engine.reset();
        // ADR-0001 SessionLifecycleEvent::Started — agent state map をクリアさせる。
        self.virtual_engine.mark_session_started();

        // kline ロードを別タスクで実行
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
                    clock,
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
                    if *pending_count == 0 {
                        clock.resume_from_waiting(Instant::now());
                        true
                    } else {
                        false
                    }
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

    /// Playing 中の毎 tick 処理（100ms ごと）。
    fn tick(&mut self, now: Instant) {
        use crate::replay::dispatcher::dispatch_tick;

        let (clock, store, active_streams) = match &mut self.state.session {
            ReplaySession::Loading {
                clock,
                store,
                active_streams,
                ..
            }
            | ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => (clock, store, active_streams),
            ReplaySession::Idle => return,
        };

        let dispatch = dispatch_tick(clock, store, active_streams, now);

        // 仮想約定エンジンにトレードを通知する
        let ticker_str = &self.ticker_str;
        for (_, trades) in &dispatch.trade_events {
            let fills = self
                .virtual_engine
                .on_tick(ticker_str, trades, dispatch.current_time);
            // ナラティブ outcome 自動更新（Phase 4a C-1）
            for fill in fills {
                let store = self.narrative_store.clone();
                let order_id = fill.order_id.clone();
                let fill_price = fill.fill_price;
                let fill_time_ms =
                    crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64();
                tokio::spawn(async move {
                    if let Err(e) = crate::narrative::service::update_outcome_from_fill(
                        &store,
                        &order_id,
                        fill_price,
                        fill_time_ms,
                        None,
                    )
                    .await
                    {
                        log::warn!(
                            "headless: failed to update narrative outcome for order {order_id}: {e}"
                        );
                    }
                });
            }
        }

        if dispatch.reached_end {
            log::info!("headless: replay reached end at {}", dispatch.current_time);
        }
    }

    /// `POST /api/replay/step-forward` を処理する。Playing 中は先に自動 pause する。
    fn step_forward(&mut self) -> String {
        let was_playing = self.state.is_playing();
        if was_playing {
            let _ = self.pause();
        }
        if !self.state.is_paused() {
            return serde_json::json!({"ok": false, "error": "not paused"}).to_string();
        }

        // Playing 中に呼ばれた場合は End まで一気にシークして返す（GUI 仕様に合わせる）
        if was_playing {
            if let ReplaySession::Active { clock, .. } = &mut self.state.session {
                let end = clock.full_range().end;
                clock.seek(end);
            }
            return serde_json::json!({"ok": true}).to_string();
        }

        let pane_step = self.min_step_ms();
        let step_ms = match &self.state.session {
            ReplaySession::Active { clock, .. } => {
                let current = clock.now_ms();
                let end = clock.full_range().end;
                if current + pane_step > end {
                    return serde_json::json!({"ok": false, "error": "at end of range"})
                        .to_string();
                }
                pane_step
            }
            _ => return serde_json::json!({"ok": false, "error": "not active"}).to_string(),
        };

        let new_time = self.state.current_time() + step_ms;

        // clock を seek する（Paused 状態でシーク）
        if let ReplaySession::Active {
            clock,
            store,
            active_streams,
            ..
        } = &mut self.state.session
        {
            clock.seek(new_time);

            // kline の close 価格から合成 Trade を生成して仮想約定エンジンに通知する。
            // active_streams は Kline バリアントのみを持ち、Trades バリアントはストアに
            // 保存されていないため store.trades_in() は常に空を返す。
            let ticker_str = self.ticker_str.clone();
            let synthetic: Vec<exchange::Trade> = active_streams
                .iter()
                .filter(|s| matches!(s, StreamKind::Kline { .. }))
                .filter_map(|stream| {
                    let klines = store.klines_in(stream, 0..new_time.saturating_add(1));
                    klines
                        .iter()
                        .rev()
                        .find(|k| k.time <= new_time)
                        .map(|k| exchange::Trade {
                            time: new_time,
                            is_sell: false,
                            price: k.close,
                            qty: exchange::unit::qty::Qty::from_f32(1.0),
                        })
                })
                .collect();
            if !synthetic.is_empty() {
                let fills = self
                    .virtual_engine
                    .on_tick(&ticker_str, &synthetic, new_time);
                // ナラティブ outcome 自動更新（Phase 4a C-1）: step-forward 経由でも
                // fills を拾わないと S52 の outcome 自動反映が発火しない。
                for fill in fills {
                    let store = self.narrative_store.clone();
                    let order_id = fill.order_id.clone();
                    let fill_price = fill.fill_price;
                    let fill_time_ms =
                        crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64();
                    tokio::spawn(async move {
                        if let Err(e) = crate::narrative::service::update_outcome_from_fill(
                            &store,
                            &order_id,
                            fill_price,
                            fill_time_ms,
                            None,
                        )
                        .await
                        {
                            log::warn!(
                                "headless step_forward: failed to update narrative outcome for order {order_id}: {e}"
                            );
                        }
                    });
                }
            }
        }

        serde_json::json!({"ok": true, "current_time": new_time}).to_string()
    }

    /// `POST /api/replay/step-backward` を処理する。Playing 中は先に自動 pause して start にシークする。
    fn step_backward(&mut self) -> String {
        let was_playing = self.state.is_playing();
        if was_playing {
            let _ = self.pause();
        }
        if !self.state.is_paused() {
            return serde_json::json!({"ok": false, "error": "not paused"}).to_string();
        }

        // Playing 中に呼ばれた場合は start にシークしてリセット（GUI 仕様に合わせる）
        if was_playing {
            if let ReplaySession::Active { clock, .. } = &mut self.state.session {
                let start = clock.full_range().start;
                clock.seek(start);
            }
            self.virtual_engine.reset();
            self.virtual_engine.mark_session_reset();
            return serde_json::json!({"ok": true}).to_string();
        }

        let pane_step = self.min_step_ms();
        let (prev_time, start_ms, step_ms, current_time) = match &self.state.session {
            ReplaySession::Active {
                clock,
                store,
                active_streams,
            } => {
                let current = clock.now_ms();
                let start = clock.full_range().start;
                if current <= start {
                    return serde_json::json!({"ok": false, "error": "at start of range"})
                        .to_string();
                }
                let prev = active_streams
                    .iter()
                    .filter(|s| matches!(s, StreamKind::Kline { .. }))
                    .filter_map(|stream| {
                        store
                            .klines_in(stream, 0..current)
                            .iter()
                            .rev()
                            .find(|k| k.time < current)
                            .map(|k| k.time)
                    })
                    .max();
                (prev, start, pane_step, current)
            }
            _ => return serde_json::json!({"ok": false, "error": "not active"}).to_string(),
        };

        let new_time =
            crate::replay::compute_step_backward_target(prev_time, current_time, start_ms, step_ms);

        if let ReplaySession::Active { clock, .. } = &mut self.state.session {
            clock.seek(new_time);
        }

        // 時刻が後退したため仮想エンジンをリセット + lifecycle Reset 発火
        self.virtual_engine.reset();
        self.virtual_engine.mark_session_reset();

        serde_json::json!({"ok": true, "current_time": new_time}).to_string()
    }

    fn pause(&mut self) -> String {
        if let ReplaySession::Active { clock, .. } = &mut self.state.session {
            clock.pause();
            serde_json::json!({"ok": true}).to_string()
        } else {
            serde_json::json!({"ok": false, "error": "not active"}).to_string()
        }
    }

    fn resume(&mut self) -> String {
        if let ReplaySession::Active { clock, .. } = &mut self.state.session {
            clock.play(Instant::now());
            serde_json::json!({"ok": true}).to_string()
        } else {
            serde_json::json!({"ok": false, "error": "not active"}).to_string()
        }
    }

    fn get_status_json(&self) -> String {
        serde_json::to_string(&self.state.to_status())
            .unwrap_or_else(|e| format!(r#"{{"error":"serialize failed: {e}"}}"#))
    }

    /// `get_state_json` のロジックを構造化データとして返す（agent API 用）。
    /// JSON 文字列化せず、`StepObservation` 構造体を返す。
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

    /// `POST /api/agent/session/default/advance` を処理する（Phase 4b-1 サブフェーズ G）。
    ///
    /// ADR-0001 / phase4b_agent_replay_api.md §4.3 に基づく。任意区間を wall-time
    /// 非依存で instant 実行する。stop_on に応じて fill / narrative 更新発生時点で
    /// 停止する。Headless ランタイム専用（GUI 側は app/api/mod.rs で 400 拒否済み）。
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
        if self.is_playing() {
            let _ = self.pause();
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
                _ => break,
            };

            if reached_end {
                stopped_reason = AdvanceStoppedReason::End;
                break;
            }

            let fills: Vec<_> = if let ReplaySession::Active {
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

    /// `POST /api/agent/session/default/order` を処理する（Phase 4b-1 サブフェーズ E）。
    ///
    /// ADR-0001 / phase4b_agent_replay_api.md §3.3, §4.4, §4.5 に基づく。
    /// - session 未起動は 404 + hint
    /// - `client_order_id` 重複 & body 一致 → 200 + `idempotent_replay: true`
    /// - `client_order_id` 重複 & body 相違 → 409 Conflict
    /// - 新規 → 201 Created
    fn agent_session_place_order(
        &mut self,
        request: crate::api::order_request::AgentOrderRequest,
    ) -> (u16, String) {
        use crate::api::agent_session_state::PlaceOrderOutcome;
        use crate::api::order_request::{AgentOrderSide, AgentOrderType};
        use crate::replay::virtual_exchange::{
            PositionSide, VirtualOrder, VirtualOrderStatus, VirtualOrderType,
        };

        // session 状態チェック（step と同じ 404 / 503 ルール）。
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

        // 発注前に lifecycle イベントを観測して map を必要に応じクリア。
        self.agent_session_state
            .observe_generation(self.virtual_engine.session_generation());

        let key = request.to_key();
        // 仮 UUID を採番（idempotent replay や conflict では使われない = 捨てられる）。
        let prospective_order_id = uuid::Uuid::new_v4().to_string();
        let outcome = self.agent_session_state.place_or_replay(
            request.client_order_id.clone(),
            key,
            prospective_order_id,
        );

        match outcome {
            PlaceOrderOutcome::Created { order_id } => {
                // VirtualExchange に実発注。
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
                    // VirtualOrder.ticker は現状 symbol 単体。
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
                // 新規/冪等リプレイともに 200 で統一する。レスポンスボディの形は同一なので、
                // Python SDK は `idempotent_replay` フラグだけで分岐できる
                // （ステータスコードで分岐しない方が実装が単純になる）。
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

    /// `POST /api/agent/session/default/step` を処理する（Phase 4b-1 サブフェーズ C / D）。
    ///
    /// ADR-0001 / phase4b_agent_replay_api.md §4.2 に基づき、1 バー進行した tick の
    /// `clock_ms` / `reached_end` / `observation` / `fills` / `updated_narrative_ids`
    /// を同梱したレスポンスを返す。サブフェーズ D 以降、narrative outcome 更新は
    /// 同期 `await` で確定させる（agent 側 polling を不要にするため）。
    async fn agent_session_step(&mut self) -> (u16, String) {
        use crate::api::step_response::{StepFill, StepResponse};

        let overall_start = std::time::Instant::now();

        // セッション状態チェック。Idle は 404、Loading は 503。
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

        // Agent state を lifecycle イベントに同期（ハンドラ入口で 1 回のみ）。
        // 前回の step/order 以降に UI リモコン経由で /play や seek が走った場合、
        // ここで stale な client_order_id マップが破棄される。
        self.agent_session_state
            .observe_generation(self.virtual_engine.session_generation());

        // Playing 中なら自動 pause（step-forward 仕様と対称）。
        if self.is_playing() {
            let _ = self.pause();
        }

        // 1 バー進める。範囲終端なら reached_end = true で現在時刻を据え置き。
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
            // 冒頭で ReplaySession::Active をチェック済みだが、await を挟まない
            // ため状態遷移は起こらない。それでも panic ではなく 500 を返すことで、
            // 将来 await が混入した場合の silent crash を防ぐ。
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

        // 進行処理（reached_end でない場合のみ）
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

        // narrative outcome 更新を同期 await で確定させる（サブフェーズ D）。
        // plan §5.2 / §8 R1: fill_count 件 → 100ms/p95 を超えたら非同期化に戻す判断基準。
        // 失敗は log::warn! のみで step 全体を落とさない（plan §7.2 方針）。
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
                 {fill_count} fill(s) — exceeds R1 p95 budget (100ms). Consider non-blocking fallback.",
                fill_count = fills.len()
            );
        }

        // observation 構築（session は Active のはず）
        let observation = match self.build_step_observation(200) {
            Some(obs) => obs,
            None => {
                return (
                    500,
                    r#"{"error":"observation build failed: session not active"}"#.to_string(),
                );
            }
        };

        // サブフェーズ E: fill.order_id から agent_session_state を逆引きして
        // client_order_id を埋める。他経路（UI リモコン `/api/replay/order`）発注の fill
        // は agent の map に無いため None のまま（設計通り）。
        // 世代同期はハンドラ開始時に済んでいるため、ここでは再取得しない
        // （step 内で generation を変える処理は存在しない）。
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

        // R1 計測用: 全体の step ハンドラ処理時間。
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

    /// アクティブセッションの最新 close 価格を返す。
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

    /// 全ペインの最小タイムフレーム（ms）を返す。step forward/backward のステップ幅に使う。
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
                clock.pause();
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
            clock.pause();
            clock.seek(start);
        }
        serde_json::json!({ "ok": true }).to_string()
    }

    /// `StepClock::now_ms()` を取得する。未開始なら 0。
    fn now_ms(&self) -> i64 {
        use crate::replay::ReplaySession;
        match &self.state.session {
            ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                clock.now_ms() as i64
            }
            _ => 0,
        }
    }

    /// ナラティブコマンドをサービスレイヤーに委譲する。
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

    /// API コマンドを処理し、ReplySender でレスポンスを返す。
    async fn handle_command(&mut self, cmd: ApiCommand, reply: crate::replay_api::ReplySender) {
        use crate::replay::ReplayCommand;

        match cmd {
            ApiCommand::Replay(ReplayCommand::GetStatus) => {
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::Toggle) => {
                // NOTE: サブフェーズ M スコープ内では旧 play/pause 意味論を維持する。
                // ADR-0001 §3 の Live↔Replay 切替 + SessionLifecycleEvent::Terminated
                // 発火はサブフェーズ Q で実装する。
                let result = if self.is_playing() {
                    self.pause()
                } else {
                    self.resume()
                };
                reply.send(result);
            }
            ApiCommand::Replay(ReplayCommand::SetMode { mode }) => {
                // headless では Live モードに遷移できないため実質 no-op だが、
                // `"live"` 指定時は ADR-0001 の `SessionLifecycleEvent::Terminated`
                // 契約として生成世代を進める（agent API 側の `client_order_id`
                // map を前セッションから確実に切り離すため）。
                if mode.eq_ignore_ascii_case("live") {
                    self.virtual_engine.mark_session_terminated();
                }
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::SaveState) => {
                // headless では保存不要
                reply.send(r#"{"ok":true}"#.to_string());
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
                // session_id は route 層で "default" に限定済み（非 default は 501 で既に拒否）。
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
            // headless で未対応のコマンドは 501 を返す
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

// ── エントリーポイント ──────────────────────────────────────────────────────────

/// headless モードのメインループ。
/// `--headless` フラグが渡されたとき `main()` から呼ばれる。
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

    // kline ロード結果の通知チャネル
    let (load_tx, mut load_rx) = tokio::sync::mpsc::channel::<LoadResult>(8);

    // API コマンドチャネル
    // NOTE: replay_api::start_server は GUI モードとシグネチャを共有するため
    // futures::channel::mpsc::Sender を要求する。tokio::sync::mpsc ではなく
    // futures ベースのチャネルを使うのはここが理由。
    let (api_tx, mut api_rx) = futures::channel::mpsc::channel::<ApiMessage>(32);

    // HTTP API サーバーを別タスクで起動
    tokio::spawn(async move {
        crate::replay_api::start_server(api_tx).await;
    });

    let mut engine = HeadlessEngine::new(ticker, timeframe, load_tx);

    // ADR-0001 §2 自動再生機構の全廃:
    // 以前は 100ms interval で `engine.tick()` を発火させ Playing 中の replay を自動進行させていたが、
    // agent session API (`/api/agent/session/:id/{step,advance}`) への一本化に伴い削除。

    log::info!("headless event loop started (API port: {})", {
        std::env::var("FLOWSURFACE_API_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(9876)
    });

    loop {
        tokio::select! {
            biased;

            // kline ロード完了
            Some(result) = load_rx.recv() => {
                engine.handle_load_result(result);
            }

            // API コマンド受信
            Some((cmd, reply)) = api_rx.next() => {
                engine.handle_command(cmd, reply).await;
            }
        }
    }
}

// ── テスト ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_headless_args ────────────────────────────────────────────────────

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

    // ── parse_ticker_str ──────────────────────────────────────────────────────

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

    // ── parse_timeframe_str ───────────────────────────────────────────────────

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

    // ── HeadlessEngine::play 引数バリデーション ───────────────────────────────

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
        // 注文を入れてから play — リセットされるはず
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

    // ── step_forward / pause / resume ────────────────────────────────────────

    #[test]
    fn step_forward_returns_error_when_not_paused() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let json = engine.step_forward();
        assert!(json.contains("not paused") || json.contains("not active"));
    }

    #[test]
    fn step_backward_returns_error_when_not_active() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let json = engine.step_backward();
        assert!(json.contains("not active") || json.contains("not paused"));
    }

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

        // Active セッションを手動で構築する
        let mut clock = StepClock::new(start_ms, end_ms, step_ms);
        clock.pause();
        clock.seek(initial_time);

        let mut store = EventStore::new();
        // start, start+step の 2 本の kline を挿入する
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
    fn step_backward_returns_error_when_at_start() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 2, step_ms, start_ms);
        let json = engine.step_backward();
        assert!(
            json.contains("at start"),
            "expected 'at start' error, got: {json}"
        );
    }

    #[test]
    fn step_backward_moves_back_one_step_when_paused() {
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let initial_time = start_ms + step_ms; // 2 本目の位置
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 2, step_ms, initial_time);
        let before = engine.state.current_time();
        let json = engine.step_backward();
        assert!(json.contains("\"ok\":true"), "expected ok, got: {json}");
        let after = engine.state.current_time();
        assert!(
            after < before,
            "time should decrease: before={before}, after={after}"
        );
        assert_eq!(after, start_ms);
    }

    #[test]
    fn step_backward_resets_virtual_engine() {
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let initial_time = start_ms + step_ms;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 2, step_ms, initial_time);

        // 注文を追加してから後退 → リセットされるはず
        engine.virtual_engine.place_order(VirtualOrder {
            order_id: "test-order".to_string(),
            ticker: "BTC".to_string(),
            side: PositionSide::Long,
            qty: 0.1,
            order_type: VirtualOrderType::Market,
            placed_time_ms: initial_time,
            status: VirtualOrderStatus::Pending,
        });
        assert_eq!(engine.virtual_engine.get_orders().len(), 1);

        let _ = engine.step_backward();
        assert_eq!(
            engine.virtual_engine.get_orders().len(),
            0,
            "virtual engine should be reset after step_backward"
        );
    }

    #[test]
    fn pause_returns_error_when_not_active() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let json = engine.pause();
        assert!(json.contains("not active"));
    }

    #[test]
    fn get_state_returns_error_when_not_active() {
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        let json = engine.get_state_json(50);
        assert!(json.contains("replay not active"));
    }

    // ── handle_load_result: Loading → Active 遷移 ────────────────────────────

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

        // Loading 状態を手動で設定
        let clock = StepClock::new(0, 3_600_000, 60_000);
        let mut active_streams = HashSet::new();
        active_streams.insert(stream);
        engine.state.session = ReplaySession::Loading {
            clock,
            pending_count: 1,
            store: EventStore::new(),
            active_streams,
        };

        // ダミー kline を返す
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

    // ── agent_session_step (Phase 4b-1 サブフェーズ C) ────────────────────────

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
        // 初期位置を end_ms にセット — これ以上進めない。
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

        // 成行注文を 1 件置いてから step — 次 tick の close 価格で即約定する。
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
        // narrative が linked されていない場合は空配列（サブフェーズ D）。
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
        // ADR-0001 / plan §5.2 の核不変条件:
        // step レスポンスの `updated_narrative_ids` は同期 await 後に確定し、
        // agent が polling 不要で UUID を取得できる。
        use crate::narrative::model::{Narrative, NarrativeAction, NarrativeSide, SnapshotRef};
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        // 永続 SQLite の蓄積を避けるため、このテスト実行固有の UUID を order_id に採用。
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
            // 共有 SQLite（デフォルト store）はテスト間で永続化されるため、
            // 毎回ユニークな order_id を生成して蓄積による衝突を防ぐ。
            linked_order_id: Some(unique_order_id.clone()),
            public: false,
            created_at_ms: start_ms as i64,
            idempotency_key: None,
        };
        engine.narrative_store.insert(narrative).await.unwrap();

        // 成行注文を置いて step — 次 tick で即約定する。
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

        // 1. updated_narrative_ids にこの narrative の UUID が含まれる。
        let ids = v["updated_narrative_ids"]
            .as_array()
            .expect("must be array");
        assert_eq!(ids.len(), 1, "expected one updated id, got {body}");
        assert_eq!(ids[0].as_str().unwrap(), narrative_id.to_string());

        // 2. step レスポンス返却時点で outcome が DB に書き込み完了している
        //    （polling 不要 = 同期 await の保証）。
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

    // ── agent_session_place_order (Phase 4b-1 サブフェーズ E) ────────────────

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
        // StepClock / EventStore は super::* 経由で既に import 済み。
        let ticker = parse_ticker_str("HyperliquidLinear:BTC").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let mut engine = HeadlessEngine::new(ticker, Timeframe::M1, tx);
        // Loading セッションを手動構築
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
        // 新規・冪等リプレイともに 200 で統一。分岐は idempotent_replay フラグで行う。
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

        // 同じ client_order_id + 同じ body → 200 + idempotent_replay: true
        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s2, 200, "body: {b2}");
        let v2: serde_json::Value = serde_json::from_str(&b2).unwrap();
        assert_eq!(v2["idempotent_replay"], true);
        assert_eq!(v2["order_id"], first_order_id);

        // VirtualExchange には 1 件のみ。
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

        // 同じ cli_1 で qty 違い → 409
        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.2));
        assert_eq!(s2, 409, "body: {b2}");
        let v: serde_json::Value = serde_json::from_str(&b2).unwrap();
        assert!(v["error"].as_str().unwrap().contains("conflict"));
        assert!(v["existing_order_id"].is_string());

        // VirtualExchange には 1 件のみ（衝突時は新規発注しない）。
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
        // ADR-0001 不変条件: UI リモコン /play 等が走ると agent map がクリアされる。
        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 5, step_ms, start_ms);

        let (s1, _) = engine.agent_session_place_order(sample_order_request("cli_1", 0.1));
        assert_eq!(s1, 200);

        // lifecycle event 発火（実運用では /play / step-backward が呼ぶ）。
        engine.virtual_engine.mark_session_reset();

        // 同じ cli_1 で qty 違い → 新規受付（クリア後なので 201）。
        let (s2, b2) = engine.agent_session_place_order(sample_order_request("cli_1", 0.2));
        assert_eq!(
            s2, 200,
            "after lifecycle event, same client_order_id can be reused: {b2}"
        );
    }

    #[tokio::test]
    async fn step_fill_carries_client_order_id_when_placed_via_agent_api() {
        // サブフェーズ E の重要不変条件: agent API で発注した注文の fill は
        // step レスポンスの fills 配列で client_order_id を返す。
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

    // ── agent_session_advance (Phase 4b-1 サブフェーズ G) ────────────────────

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
        // until_ms が range 終端より先なら End で停止。
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
    async fn advance_stops_on_fill() {
        use crate::api::advance_request::AdvanceStopCondition;
        use crate::replay::virtual_exchange::{PositionSide, VirtualOrder, VirtualOrderStatus};

        let start_ms = 1_000_000u64;
        let step_ms = 60_000u64;
        let mut engine =
            make_active_engine_with_klines(start_ms, start_ms + step_ms * 10, step_ms, start_ms);

        // 成行注文 → 次 tick で約定する → advance が 1 tick で stop_on: fill により停止する。
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
        // until_ms <= 現在時刻 の場合は 0 tick で UntilReached。後退は agent scope 外。
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
