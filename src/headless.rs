/// Headless モード — GUI なしで tokio ランタイム + HTTP API サーバーだけを動かす。
///
/// `flowsurface --headless --ticker HyperliquidLinear:BTC --timeframe M1` で起動する。
/// iced::daemon を一切起動しないため、Python SDK のような外部プログラムから
/// HTTP API (port 9876) 経由で高速に強化学習ループを回せる。
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

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
enum LoadResult {
    Ok {
        stream: StreamKind,
        range: std::ops::Range<u64>,
        klines: Vec<exchange::Kline>,
    },
    Err(String),
}

/// headless モード専用リプレイエンジン。
/// iced への依存を持たず、tokio 非同期タスクで動作する。
struct HeadlessEngine {
    state: ReplayState,
    virtual_engine: VirtualExchangeEngine,
    ticker: Ticker,
    ticker_str: String,
    timeframe: Timeframe,
    load_tx: tokio::sync::mpsc::Sender<LoadResult>,
}

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
        Self {
            state,
            virtual_engine: VirtualExchangeEngine::new(1_000_000.0),
            ticker,
            ticker_str,
            timeframe,
            load_tx,
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
            let _ = tx.send(msg).await;
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
            self.virtual_engine
                .on_tick(ticker_str, trades, dispatch.current_time);
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

        let step_ms = match &self.state.session {
            ReplaySession::Active {
                clock,
                active_streams,
                ..
            } => {
                let step = crate::replay::min_timeframe_ms(active_streams);
                let current = clock.now_ms();
                let end = clock.full_range().end;
                if current + step > end {
                    return serde_json::json!({"ok": false, "error": "at end of range"})
                        .to_string();
                }
                step
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
                self.virtual_engine
                    .on_tick(&ticker_str, &synthetic, new_time);
            }
        }

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
                            let items: Vec<serde_json::Value> = slice
                                .iter()
                                .map(|k| {
                                    serde_json::json!({
                                        "time": k.time,
                                        "open": k.open.to_f64(),
                                        "high": k.high.to_f64(),
                                        "low": k.low.to_f64(),
                                        "close": k.close.to_f64(),
                                    })
                                })
                                .collect();
                            klines_out.push(serde_json::json!({"stream": label, "klines": items}));
                        }

                        let trade_stream = StreamKind::Trades {
                            ticker_info: *ticker_info,
                        };
                        const TRADE_WINDOW_MS: u64 = 300_000;
                        let trade_start = now_ms.saturating_sub(TRADE_WINDOW_MS);
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
                    "current_time": now_ms,
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
        serde_json::json!({"ok": true, "order_id": id}).to_string()
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

    /// API コマンドを処理し、ReplySender でレスポンスを返す。
    fn handle_command(&mut self, cmd: ApiCommand, reply: crate::replay_api::ReplySender) {
        use crate::replay::ReplayCommand;

        match cmd {
            ApiCommand::Replay(ReplayCommand::GetStatus) => {
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::Play { start, end }) => {
                match self.play(&start, &end) {
                    Ok(json) => reply.send(json),
                    Err(e) => reply.send_status(400, serde_json::json!({"error": e}).to_string()),
                }
            }
            ApiCommand::Replay(ReplayCommand::Pause) => {
                reply.send(self.pause());
            }
            ApiCommand::Replay(ReplayCommand::Resume) => {
                reply.send(self.resume());
            }
            ApiCommand::Replay(ReplayCommand::StepForward) => {
                reply.send(self.step_forward());
            }
            ApiCommand::Replay(ReplayCommand::Toggle) => {
                // headless では常に Replay モードなので no-op
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::CycleSpeed) => {
                self.state.cycle_speed();
                reply.send(self.get_status_json());
            }
            ApiCommand::Replay(ReplayCommand::StepBackward) => {
                // headless では StepBackward は未実装
                reply.send_status(
                    501,
                    r#"{"error":"StepBackward not implemented in headless mode"}"#.to_string(),
                );
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

    // 100ms tick インターバル（Playing 中のみ有効）
    let mut tick_interval = tokio::time::interval(Duration::from_millis(100));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

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
                engine.handle_command(cmd, reply);
            }

            // 再生中の tick（Playing 時のみ処理）
            _ = tick_interval.tick() => {
                if engine.is_playing() {
                    engine.tick(Instant::now());
                }
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
}
