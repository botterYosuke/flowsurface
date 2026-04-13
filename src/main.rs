#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod chart;
mod connector;
mod layout;
mod logger;
mod modal;
mod notify;
mod screen;
use screen::login::{self, LoginScreen};
mod replay;
mod replay_api;
use replay::{ReplayMessage, ReplayState};
mod style;
mod version;
mod widget;
mod window;

use data::config::theme::default_theme;
use data::{layout::WindowSpec, sidebar};
use layout::{LayoutId, configuration};
use modal::{
    LayoutManager, ThemeEditor,
    audio::AudioStream,
    network_manager::{self, NetworkManager},
};
use modal::{dashboard_modal, main_dialog_modal};
use notify::Notifications;
use screen::dashboard::{self, Dashboard};
use widget::{
    confirm_dialog_container,
    toast::{self, Toast},
    tooltip,
};

use iced::{
    Alignment, Element, Subscription, Task, keyboard, padding,
    widget::{
        button, column, container, pane_grid, pick_list, row, rule, scrollable, text, text_input,
        tooltip::Position as TooltipPosition,
    },
};
use std::{borrow::Cow, collections::HashMap, vec};

/// ResolvedStream から (ticker 表示文字列, timeframe 表示文字列) を抽出する。
/// Ready → 実行中ストリーム、Waiting → 永続化された構成から復元する。
fn extract_pane_ticker_timeframe(
    streams: &connector::ResolvedStream,
) -> (Option<String>, Option<String>) {
    use connector::ResolvedStream;
    use data::stream::PersistStreamKind;

    let format_ticker = |ticker: &exchange::Ticker| -> String {
        let ex_str = format!("{:?}", ticker.exchange).replace(' ', "");
        format!("{ex_str}:{ticker}")
    };

    match streams {
        ResolvedStream::Ready(list) => {
            let mut ticker_str: Option<String> = None;
            let mut tf_str: Option<String> = None;
            for s in list {
                if ticker_str.is_none() {
                    ticker_str = Some(format_ticker(&s.ticker_info().ticker));
                }
                if let Some((_, tf)) = s.as_kline_stream() {
                    tf_str = Some(format!("{tf:?}"));
                    break;
                }
            }
            (ticker_str, tf_str)
        }
        ResolvedStream::Waiting { streams: persist, .. } => {
            let mut ticker_str: Option<String> = None;
            let mut tf_str: Option<String> = None;
            for ps in persist {
                match ps {
                    PersistStreamKind::Kline { ticker, timeframe } => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(ticker));
                        }
                        tf_str = Some(format!("{timeframe:?}"));
                        break;
                    }
                    PersistStreamKind::Depth(d) => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(&d.ticker));
                        }
                    }
                    PersistStreamKind::Trades { ticker } => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(ticker));
                        }
                    }
                    PersistStreamKind::DepthAndTrades(d) => {
                        if ticker_str.is_none() {
                            ticker_str = Some(format_ticker(&d.ticker));
                        }
                    }
                }
            }
            (ticker_str, tf_str)
        }
    }
}

fn main() {
    logger::setup(cfg!(debug_assertions)).expect("Failed to initialize logger");

    std::thread::spawn(data::cleanup_old_market_data);

    let _ = iced::daemon(Flowsurface::new, Flowsurface::update, Flowsurface::view)
        .settings(iced::Settings {
            antialiasing: true,
            fonts: vec![
                Cow::Borrowed(style::AZERET_MONO_BYTES),
                Cow::Borrowed(style::ICONS_BYTES),
            ],
            default_text_size: iced::Pixels(12.0),
            ..Default::default()
        })
        .title(Flowsurface::title)
        .theme(Flowsurface::theme)
        .scale_factor(Flowsurface::scale_factor)
        .subscription(Flowsurface::subscription)
        .run();
}

struct Flowsurface {
    login_window: Option<window::Id>,
    login_screen: LoginScreen,
    saved_main_window_spec: Option<data::layout::WindowSpec>,
    main_window: window::Window,
    sidebar: dashboard::Sidebar,
    layout_manager: LayoutManager,
    theme_editor: ThemeEditor,
    network: NetworkManager,
    audio_stream: AudioStream,
    confirm_dialog: Option<screen::ConfirmDialog<Message>>,
    volume_size_unit: exchange::SizeUnit,
    ui_scale_factor: data::ScaleFactor,
    timezone: data::UserTimezone,
    theme: data::Theme,
    notifications: Notifications,
    replay: ReplayState,
}

#[derive(Debug, Clone)]
enum Message {
    Login(login::Message),
    Sidebar(dashboard::sidebar::Message),
    MarketWsEvent(exchange::Event),
    Dashboard {
        /// If `None`, the active layout is used for the event.
        layout_id: Option<uuid::Uuid>,
        event: dashboard::Message,
    },
    Tick(std::time::Instant),
    WindowEvent(window::Event),
    ExitRequested(HashMap<window::Id, WindowSpec>),
    RestartRequested(HashMap<window::Id, WindowSpec>),
    SaveStateRequested(HashMap<window::Id, WindowSpec>),
    GoBack,
    DataFolderRequested,
    OpenUrlRequested(Cow<'static, str>),
    ThemeSelected(iced_core::Theme),
    ScaleFactorChanged(data::ScaleFactor),
    SetTimezone(data::UserTimezone),
    ToggleTradeFetch(bool),
    ApplyVolumeSizeUnit(exchange::SizeUnit),
    RemoveNotification(usize),
    ToggleDialogModal(Option<screen::ConfirmDialog<Message>>),
    ThemeEditor(modal::theme_editor::Message),
    NetworkManager(modal::network_manager::Message),
    Layouts(modal::layout_manager::Message),
    AudioStream(modal::audio::Message),
    LoginCompleted(Result<exchange::adapter::tachibana::TachibanaSession, String>),
    SessionRestoreResult(Option<exchange::adapter::tachibana::TachibanaSession>),
    Replay(ReplayMessage),
    ReplayApi(replay_api::ApiMessage),
}

impl Flowsurface {
    fn new() -> (Self, Task<Message>) {
        let saved_state = layout::load_saved_state();

        // メインウィンドウIDをダミーで用意（起動はログイン後）
        let dummy_main_id = window::Id::unique();

        let (sidebar, launch_sidebar) = dashboard::Sidebar::new(&saved_state);
        let (audio_stream, audio_init_err) = AudioStream::new(saved_state.audio_cfg);
        let saved_main_window_spec = saved_state.main_window;

        let mut state = Self {
            login_window: None,
            login_screen: LoginScreen::new(),
            saved_main_window_spec,
            main_window: window::Window::new(dummy_main_id),
            layout_manager: saved_state.layout_manager,
            theme_editor: ThemeEditor::new(saved_state.custom_theme),
            audio_stream,
            sidebar,
            confirm_dialog: None,
            timezone: saved_state.timezone,
            ui_scale_factor: saved_state.scale_factor,
            volume_size_unit: saved_state.volume_size_unit,
            theme: saved_state.theme,
            notifications: Notifications::new(),
            network: NetworkManager::new(saved_state.proxy_cfg),
            replay: {
                let replay_mode = match saved_state.replay_config.mode.as_str() {
                    "replay" => replay::ReplayMode::Replay,
                    _ => replay::ReplayMode::Live,
                };
                ReplayState {
                    mode: replay_mode,
                    range_input: replay::ReplayRangeInput {
                        start: saved_state.replay_config.range_start,
                        end: saved_state.replay_config.range_end,
                    },
                    clock: None,
                    event_store: replay::store::EventStore::new(),
                    active_streams: std::collections::HashSet::new(),
                }
            },
        };

        if let Some(err) = audio_init_err {
            state
                .notifications
                .push(Toast::error(format!("Audio disabled: {err}")));
        }

        // 起動時にまずセッション復元を試行する（ウィンドウはまだ開かない）
        let restore_task = Task::perform(
            connector::auth::try_restore_session(),
            Message::SessionRestoreResult,
        );

        (
            state,
            Task::batch([launch_sidebar.map(Message::Sidebar), restore_task]),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Login(msg) => {
                match msg {
                    login::Message::LoginSubmit => {
                        // ログインエラーをクリアして非同期ログインを開始
                        self.login_screen.set_error(None);
                        let user_id = self.login_screen.user_id.clone();
                        let password = self.login_screen.password.clone();
                        let is_demo = self.login_screen.is_demo;

                        return Task::perform(
                            connector::auth::perform_login(user_id, password, is_demo),
                            Message::LoginCompleted,
                        );
                    }
                    other => self.login_screen.update(other),
                }
                return Task::none();
            }
            Message::LoginCompleted(result) => {
                match result {
                    Ok(session) => {
                        connector::auth::store_session(session.clone());
                        connector::auth::persist_session(&session);
                        let dashboard_task = self.transition_to_dashboard();
                        let master_task = Self::start_master_download(session);
                        return Task::batch([dashboard_task, master_task]);
                    }
                    Err(error_msg) => {
                        log::warn!("Login failed: {error_msg}");
                        self.login_screen.set_error(Some(error_msg));
                    }
                }
                return Task::none();
            }
            Message::SessionRestoreResult(result) => {
                if let Some(session) = result {
                    // 再ログイン成功 → メイン画面を直接表示
                    connector::auth::store_session(session.clone());
                    let dashboard_task = self.transition_to_dashboard();
                    let master_task = Self::start_master_download(session);
                    return Task::batch([dashboard_task, master_task]);
                }
                // 再ログイン失敗 → ログイン画面を表示
                let (login_window_id, open_login_window) = window::open(window::Settings {
                    size: iced::Size::new(900.0, 560.0),
                    position: window::Position::Centered,
                    resizable: false,
                    exit_on_close_request: true,
                    ..Default::default()
                });
                self.login_window = Some(login_window_id);
                return open_login_window.discard();
            }
            Message::MarketWsEvent(event) => {
                let main_window_id = self.main_window.id;
                let dashboard = self.active_dashboard_mut();

                match event {
                    exchange::Event::Connected(exchange) => {
                        log::info!("a stream connected to {exchange} WS");
                    }
                    exchange::Event::Disconnected(exchange, reason) => {
                        log::info!("a stream disconnected from {exchange} WS: {reason:?}");
                    }
                    exchange::Event::DepthReceived(stream, depth_update_t, depth) => {
                        let task = dashboard
                            .ingest_depth(&stream, depth_update_t, &depth, main_window_id)
                            .map(move |msg| Message::Dashboard {
                                layout_id: None,
                                event: msg,
                            });

                        return task;
                    }
                    exchange::Event::TradesReceived(stream, update_t, buffer) => {
                        let task = dashboard
                            .ingest_trades(&stream, &buffer, update_t, main_window_id)
                            .map(move |msg| Message::Dashboard {
                                layout_id: None,
                                event: msg,
                            });

                        if let Some(msg) = self.audio_stream.try_play_sound(&stream, &buffer) {
                            self.notifications.push(Toast::error(msg));
                        }

                        return task;
                    }
                    exchange::Event::KlineReceived(stream, kline) => {
                        return dashboard
                            .update_latest_klines(&stream, &kline, main_window_id)
                            .map(move |msg| Message::Dashboard {
                                layout_id: None,
                                event: msg,
                            });
                    }
                }
            }
            Message::Tick(now) => {
                let main_window_id = self.main_window.id;
                let mut all_tasks: Vec<Task<Message>> = Vec::new();

                // リプレイ再生中: dispatch_tick でイベントを抽出してチャートに注入する
                if self.replay.is_replay() {
                    if let Some(clock) = &mut self.replay.clock {
                        let dispatch = replay::dispatcher::dispatch_tick(
                            clock,
                            &self.replay.event_store,
                            &self.replay.active_streams,
                            now,
                        );

                        // klines を kline chart ペインに注入
                        for (stream, klines) in &dispatch.kline_events {
                            if !klines.is_empty() {
                                self.active_dashboard_mut()
                                    .ingest_replay_klines(stream, klines, main_window_id);
                            }
                        }

                        // trades を heatmap 等に注入
                        for (stream, trades) in &dispatch.trade_events {
                            if !trades.is_empty() {
                                let update_t = trades.last().map_or(dispatch.current_time, |t| t.time);
                                let task = self
                                    .active_dashboard_mut()
                                    .ingest_trades(stream, trades, update_t, main_window_id)
                                    .map(move |msg| Message::Dashboard {
                                        layout_id: None,
                                        event: msg,
                                    });
                                all_tasks.push(task);
                            }
                        }

                        if dispatch.reached_end {
                            // リプレイ終端に到達 → Paused のまま停止
                        }
                    }
                }

                // 通常 tick() でアニメーション更新
                let tick_task =
                    self.active_dashboard_mut()
                        .tick(now, main_window_id)
                        .map(move |msg| Message::Dashboard {
                            layout_id: None,
                            event: msg,
                        });
                all_tasks.push(tick_task);

                return Task::batch(all_tasks);
            }
            Message::WindowEvent(event) => match event {
                window::Event::CloseRequested(window) => {
                    // ログインウィンドウが閉じられたら終了
                    if Some(window) == self.login_window {
                        return iced::exit();
                    }

                    let main_window = self.main_window.id;
                    let dashboard = self.active_dashboard_mut();

                    if window != main_window {
                        dashboard.popout.remove(&window);
                        return window::close(window);
                    }

                    let mut active_windows = dashboard
                        .popout
                        .keys()
                        .copied()
                        .collect::<Vec<window::Id>>();
                    active_windows.push(main_window);

                    return window::collect_window_specs(active_windows, Message::ExitRequested);
                }
            },
            Message::ExitRequested(windows) => {
                self.save_state_to_disk(&windows);
                return iced::exit();
            }
            Message::SaveStateRequested(windows) => {
                self.save_state_to_disk(&windows);
            }
            Message::RestartRequested(windows) => {
                self.save_state_to_disk(&windows);
                return self.restart();
            }
            Message::GoBack => {
                let main_window = self.main_window.id;

                if self.confirm_dialog.is_some() {
                    self.confirm_dialog = None;
                } else if self.sidebar.active_menu().is_some() {
                    self.sidebar.set_menu(None);
                } else {
                    let dashboard = self.active_dashboard_mut();

                    if dashboard.go_back(main_window) {
                        return Task::none();
                    } else if dashboard.focus.is_some() {
                        dashboard.focus = None;
                    } else {
                        self.sidebar.hide_tickers_table();
                    }
                }
            }
            Message::ThemeSelected(theme) => {
                self.theme = data::Theme(theme.clone());

                let main_window = self.main_window.id;
                self.active_dashboard_mut()
                    .theme_updated(main_window, &theme);
            }
            Message::Dashboard {
                layout_id: id,
                event: msg,
            } => {
                let Some(active_layout) = self.layout_manager.active_layout_id() else {
                    log::error!("No active layout to handle dashboard message");
                    return Task::none();
                };

                let main_window = self.main_window;
                let layout_id = id.unwrap_or(active_layout.unique);

                if let Some(dashboard) = self.layout_manager.mut_dashboard(layout_id) {
                    let (main_task, event) = dashboard.update(msg, &main_window, &layout_id);

                    let additional_task = match event {
                        Some(dashboard::Event::DistributeFetchedData {
                            layout_id,
                            pane_id,
                            data,
                            stream,
                        }) => dashboard
                            .distribute_fetched_data(main_window.id, pane_id, data, stream)
                            .map(move |msg| Message::Dashboard {
                                layout_id: Some(layout_id),
                                event: msg,
                            }),
                        Some(dashboard::Event::Notification(toast)) => {
                            self.notifications.push(toast);
                            Task::none()
                        }
                        Some(dashboard::Event::ResolveStreams { pane_id, streams }) => {
                            let tickers_info = self.sidebar.tickers_info();

                            let has_any_ticker_info =
                                tickers_info.values().any(|opt| opt.is_some());
                            log::info!("[e2e-live] ResolveStreams pane={pane_id} streams={} has_ticker_info={has_any_ticker_info}", streams.len());
                            if !has_any_ticker_info {
                                log::debug!(
                                    "Deferring persisted stream resolution for pane {pane_id}: ticker metadata not loaded yet"
                                );
                                return Task::none();
                            }

                            let resolved_streams =
                                streams.into_iter().try_fold(vec![], |mut acc, persist| {
                                    let resolver = |t: &exchange::Ticker| {
                                        tickers_info.get(t).and_then(|opt| *opt)
                                    };

                                    match persist.into_stream_kinds(resolver) {
                                        Ok(mut resolved) => {
                                            acc.append(&mut resolved);
                                            Ok(acc)
                                        }
                                        Err(err) => Err(format!(
                                            "Persisted stream still not resolvable: {err}"
                                        )),
                                    }
                                });

                            match resolved_streams {
                                Ok(resolved) => {
                                    log::info!("[e2e-live] Streams resolved: {} streams for pane={pane_id}", resolved.len());
                                    if resolved.is_empty() {
                                        Task::none()
                                    } else {
                                        dashboard
                                            .resolve_streams(main_window.id, pane_id, resolved)
                                            .map(move |msg| Message::Dashboard {
                                                layout_id: None,
                                                event: msg,
                                            })
                                    }
                                }
                                Err(err) => {
                                    // This is typically a transient state (e.g. partial metadata, stale symbol)
                                    log::info!("[e2e-live] Stream resolution failed: {err}");
                                    Task::none()
                                }
                            }
                        }
                        Some(dashboard::Event::RequestPalette) => {
                            let theme = self.theme.0.clone();

                            let main_window = self.main_window.id;
                            self.active_dashboard_mut()
                                .theme_updated(main_window, &theme);

                            Task::none()
                        }
                        None => Task::none(),
                    };

                    // mid-replay で stream 構成変更の可能性があれば SyncReplayBuffers を chain する。
                    // §2.6.5 集約点: refresh_streams() を呼ぶ全入口を dashboard::update 経由で
                    // カバーする。Replay モードでなければハンドラ側で no-op になる。
                    return main_task
                        .map(move |msg| Message::Dashboard {
                            layout_id: Some(layout_id),
                            event: msg,
                        })
                        .chain(additional_task)
                        .chain(Task::done(Message::Replay(
                            ReplayMessage::SyncReplayBuffers,
                        )));
                }
            }
            Message::RemoveNotification(index) => {
                self.notifications.remove(index);
            }
            Message::SetTimezone(tz) => {
                self.timezone = tz;
            }
            Message::ScaleFactorChanged(value) => {
                self.ui_scale_factor = value;
            }
            Message::ToggleTradeFetch(checked) => {
                self.layout_manager
                    .iter_dashboards_mut()
                    .for_each(|dashboard| {
                        dashboard.toggle_trade_fetch(checked, &self.main_window);
                    });

                if checked {
                    self.confirm_dialog = None;
                }
            }
            Message::ToggleDialogModal(dialog) => {
                self.confirm_dialog = dialog;
            }
            Message::Layouts(message) => {
                let action = self.layout_manager.update(message);

                match action {
                    Some(modal::layout_manager::Action::Select(layout)) => {
                        let active_popout_keys = self
                            .active_dashboard()
                            .popout
                            .keys()
                            .copied()
                            .collect::<Vec<_>>();

                        let window_tasks = Task::batch(
                            active_popout_keys
                                .iter()
                                .map(|&popout_id| window::close::<window::Id>(popout_id))
                                .collect::<Vec<_>>(),
                        )
                        .discard();

                        let old_layout_id = self
                            .layout_manager
                            .active_layout_id()
                            .as_ref()
                            .map(|layout| layout.unique);

                        return window::collect_window_specs(
                            active_popout_keys,
                            dashboard::Message::SavePopoutSpecs,
                        )
                        .map(move |msg| Message::Dashboard {
                            layout_id: old_layout_id,
                            event: msg,
                        })
                        .chain(window_tasks)
                        .chain(self.load_layout(layout, self.main_window.id));
                    }
                    Some(modal::layout_manager::Action::Clone(id)) => {
                        let manager = &mut self.layout_manager;

                        let source_data = manager.get(id).map(|layout| {
                            (
                                layout.id.name.clone(),
                                layout.id.unique,
                                data::Dashboard::from(&layout.dashboard),
                            )
                        });

                        if let Some((name, old_id, ser_dashboard)) = source_data {
                            let new_uid = uuid::Uuid::new_v4();
                            let new_layout = LayoutId {
                                unique: new_uid,
                                name: manager.ensure_unique_name(&name, new_uid),
                            };

                            let mut popout_windows = Vec::new();

                            for (pane, window_spec) in &ser_dashboard.popout {
                                let configuration = configuration(pane.clone());
                                popout_windows.push((configuration, *window_spec));
                            }

                            let dashboard = Dashboard::from_config(
                                configuration(ser_dashboard.pane.clone()),
                                popout_windows,
                                old_id,
                            );

                            manager.insert_layout(new_layout.clone(), dashboard);
                        }
                    }
                    None => {}
                }
            }
            Message::AudioStream(message) => {
                if let Some(event) = self.audio_stream.update(message) {
                    match event {
                        modal::audio::UpdateEvent::RetryFailed(err) => {
                            self.notifications
                                .push(Toast::error(format!("Audio still unavailable: {err}")));
                        }
                        modal::audio::UpdateEvent::RetrySucceeded => {
                            self.notifications.push(Toast::info(
                                "Audio output re-initialized successfully".to_string(),
                            ));
                        }
                    }
                }
            }
            Message::DataFolderRequested => {
                if let Err(err) = data::open_data_folder() {
                    self.notifications
                        .push(Toast::error(format!("Failed to open data folder: {err}")));
                }
            }
            Message::OpenUrlRequested(url) => {
                if let Err(err) = data::open_url(url.as_ref()) {
                    self.notifications
                        .push(Toast::error(format!("Failed to open link: {err}")));
                }
            }
            Message::ThemeEditor(msg) => {
                let action = self.theme_editor.update(msg, &self.theme.clone().into());

                match action {
                    Some(modal::theme_editor::Action::Exit) => {
                        self.sidebar.set_menu(Some(sidebar::Menu::Settings));
                    }
                    Some(modal::theme_editor::Action::UpdateTheme(theme)) => {
                        self.theme = data::Theme(theme.clone());

                        let main_window = self.main_window.id;
                        self.active_dashboard_mut()
                            .theme_updated(main_window, &theme);
                    }
                    None => {}
                }
            }
            Message::NetworkManager(msg) => {
                let action = self.network.update(msg);

                match action {
                    Some(network_manager::Action::ApplyProxy) => {
                        if let Some(proxy) = self.network.proxy_cfg() {
                            data::config::proxy::save_proxy_auth(&proxy);
                        }

                        let main_window = self.main_window.id;
                        let dashboard = self.active_dashboard_mut();

                        let mut active_windows = dashboard
                            .popout
                            .keys()
                            .copied()
                            .collect::<Vec<window::Id>>();
                        active_windows.push(main_window);

                        return window::collect_window_specs(
                            active_windows,
                            Message::SaveStateRequested,
                        );
                    }
                    Some(network_manager::Action::Exit) => {
                        self.sidebar.set_menu(Some(sidebar::Menu::Settings));
                    }
                    None => {}
                }
            }
            Message::Sidebar(message) => {
                let (task, action) = self.sidebar.update(message);

                match action {
                    Some(dashboard::sidebar::Action::TickerSelected(ticker_info, content)) => {
                        let main_window_id = self.main_window.id;

                        let task = {
                            if let Some(kind) = content {
                                self.active_dashboard_mut().init_focused_pane(
                                    main_window_id,
                                    ticker_info,
                                    kind,
                                )
                            } else {
                                self.active_dashboard_mut()
                                    .switch_tickers_in_group(main_window_id, ticker_info)
                            }
                        };

                        // review 🟡 #4: mid-replay で heatmap-only pane を選択した場合、
                        // init_focused_pane は Task::none() を返すため Message::Dashboard 末尾の
                        // SyncReplayBuffers chain が発火しない。ここで明示的に chain する。
                        // Replay モードでない場合は SyncReplayBuffers ハンドラ側で no-op。
                        return task
                            .map(move |msg| Message::Dashboard {
                                layout_id: None,
                                event: msg,
                            })
                            .chain(Task::done(Message::Replay(
                                ReplayMessage::SyncReplayBuffers,
                            )));
                    }
                    Some(dashboard::sidebar::Action::ErrorOccurred(err)) => {
                        self.notifications.push(Toast::error(err.to_string()));
                    }
                    None => {}
                }

                return task.map(Message::Sidebar);
            }
            Message::ApplyVolumeSizeUnit(pref) => {
                self.volume_size_unit = pref;
                self.confirm_dialog = None;

                let mut active_windows: Vec<window::Id> =
                    self.active_dashboard().popout.keys().copied().collect();
                active_windows.push(self.main_window.id);

                return window::collect_window_specs(active_windows, Message::RestartRequested);
            }
            Message::Replay(msg) => {
                match msg {
                    ReplayMessage::ToggleMode => {
                        let was_replay = self.replay.is_replay();
                        self.replay.toggle_mode();

                        // Replay → Live に戻る場合はペイン content を再構築
                        // （replay_kline_buffer を無効化してライブデータが直接チャートに入るようにする）
                        if was_replay && !self.replay.is_replay() {
                            let main_window_id = self.main_window.id;
                            let dashboard = self.active_dashboard_mut();
                            dashboard.rebuild_for_live(main_window_id);
                            // WebSocket は subscription() の再評価で自動復帰する
                        }
                    }
                    ReplayMessage::StartTimeChanged(s) => {
                        self.replay.range_input.start = s;
                    }
                    ReplayMessage::EndTimeChanged(s) => {
                        self.replay.range_input.end = s;
                    }
                    ReplayMessage::Play => {
                        // 日時パース
                        let (start_ms, end_ms) = match replay::parse_replay_range(
                            &self.replay.range_input.start,
                            &self.replay.range_input.end,
                        ) {
                            Ok(range) => range,
                            Err(e) => {
                                self.notifications
                                    .push(Toast::error(format!("Replay: {e}")));
                                return Task::none();
                            }
                        };

                        let main_window_id = self.main_window.id;

                        // ペインの content をクリアし、kline ストリームを収集
                        let kline_targets = self.active_dashboard_mut().prepare_replay(main_window_id);

                        // active kline streams から最小 timeframe を計算して StepClock を初期化
                        let step_size_ms = kline_targets
                            .iter()
                            .filter_map(|(_, s)| s.as_kline_stream())
                            .map(|(_, tf)| tf.to_milliseconds())
                            .min()
                            .unwrap_or(replay::min_timeframe_ms(&Default::default()));
                        self.replay.start(start_ms, end_ms, step_size_ms);

                        // active_streams に登録
                        for (_, stream) in &kline_targets {
                            self.replay.active_streams.insert(*stream);
                        }

                        // 各 kline ストリームに対して load_klines を発行
                        let kline_tasks: Vec<Task<Message>> = kline_targets
                            .into_iter()
                            .map(|(_pane_id, stream)| {
                                let range = start_ms..end_ms;
                                Task::perform(
                                    replay::loader::load_klines(stream, range),
                                    |result| match result {
                                        Ok(r) => Message::Replay(ReplayMessage::KlinesLoadCompleted(
                                            r.stream, r.range, r.klines,
                                        )),
                                        Err(e) => Message::Replay(ReplayMessage::DataLoadFailed(e)),
                                    },
                                )
                            })
                            .collect();

                        if !kline_tasks.is_empty() {
                            return Task::batch(kline_tasks);
                        } else {
                            // kline chart 無し: 即座に Playing へ
                            self.replay.resume_from_waiting(std::time::Instant::now());
                        }
                    }
                    ReplayMessage::KlinesLoadCompleted(stream, range, klines) => {
                        let now = std::time::Instant::now();
                        let main_window_id = self.main_window.id;

                        // klines を EventStore に格納し、全 stream が揃ったら Playing 開始
                        self.replay.on_klines_loaded(stream, range, klines.clone(), now);

                        // kline chart ペインに即座に注入（現在時刻まで）
                        self.active_dashboard_mut()
                            .ingest_replay_klines(&stream, &klines, main_window_id);
                    }
                    ReplayMessage::Resume => {
                        let now = std::time::Instant::now();
                        if let Some(clock) = &mut self.replay.clock {
                            if clock.status() == replay::clock::ClockStatus::Paused {
                                clock.play(now);
                            }
                        }
                    }
                    ReplayMessage::Pause => {
                        if let Some(clock) = &mut self.replay.clock {
                            clock.pause();
                        }
                    }
                    ReplayMessage::StepForward => {
                        // EventStore から次の kline 時刻を求めてシーク
                        let main_window_id = self.main_window.id;
                        let current_time = self.replay.current_time();
                        let full_range = self.replay.clock.as_ref().map(|c| c.full_range());

                        // 全アクティブ stream の次 kline 時刻の最小値
                        let next_time = if let Some(range) = full_range {
                            self.replay.active_streams.iter().filter_map(|stream| {
                                let klines = self.replay.event_store.klines_in(stream, current_time..range.end);
                                klines.iter().find(|k| k.time > current_time).map(|k| k.time)
                            }).min()
                        } else {
                            None
                        };

                        if let (Some(new_time), Some(clock)) = (next_time, &mut self.replay.clock) {
                            clock.seek(new_time);
                            // 新時刻までの klines を即座に注入
                            for stream in self.replay.active_streams.clone().iter() {
                                let klines = self.replay.event_store.klines_in(stream, 0..new_time + 1);
                                if !klines.is_empty() {
                                    let klines_vec = klines.to_vec();
                                    self.active_dashboard_mut()
                                        .ingest_replay_klines(stream, &klines_vec, main_window_id);
                                }
                            }
                        }
                    }
                    ReplayMessage::CycleSpeed => {
                        self.replay.cycle_speed();
                    }
                    ReplayMessage::StepBackward => {
                        let main_window_id = self.main_window.id;
                        let current_time = self.replay.current_time();

                        // 全アクティブ stream の前の kline 時刻の最大値
                        let prev_time = self.replay.active_streams.iter().filter_map(|stream| {
                            let klines = self.replay.event_store.klines_in(stream, 0..current_time);
                            klines.iter().rev().find(|k| k.time < current_time).map(|k| k.time)
                        }).max();

                        let new_time = prev_time.unwrap_or(current_time);
                        if let Some(clock) = &mut self.replay.clock {
                            clock.seek(new_time);
                            clock.pause();
                        }

                        // pane chart をリビルドして new_time まで再注入
                        self.active_dashboard_mut()
                            .prepare_replay(main_window_id);

                        for stream in self.replay.active_streams.clone().iter() {
                            let klines = self.replay.event_store.klines_in(stream, 0..new_time + 1);
                            if !klines.is_empty() {
                                let klines_vec = klines.to_vec();
                                self.active_dashboard_mut()
                                    .ingest_replay_klines(stream, &klines_vec, main_window_id);
                            }
                        }
                    }
                    ReplayMessage::DataLoadFailed(err) => {
                        self.notifications
                            .push(Toast::error(format!("Replay data load failed: {err}")));
                        self.replay.clock = None;
                    }
                    ReplayMessage::SyncReplayBuffers => {
                        // mid-replay でペイン構成が変わった場合に step_size を再計算する
                        if let Some(clock) = &mut self.replay.clock {
                            let step_size_ms = replay::min_timeframe_ms(&self.replay.active_streams);
                            clock.set_step_size(step_size_ms);
                        }
                    }
                }
            }
            Message::ReplayApi((command, reply_tx)) => {
                use replay::ReplayCommand;
                use replay_api::ApiCommand;

                // ヘルパー: self.replay.to_status() を JSON 文字列化
                let reply_replay_status = |this: &Self| {
                    serde_json::to_string(&this.replay.to_status()).unwrap_or_else(|_| {
                        r#"{"error":"failed to serialize replay status"}"#.to_string()
                    })
                };

                match command {
                    ApiCommand::Replay(cmd) => match cmd {
                        ReplayCommand::GetStatus => {
                            reply_tx.send(reply_replay_status(self));
                        }
                        ReplayCommand::Toggle => {
                            let task = self.update(Message::Replay(ReplayMessage::ToggleMode));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::Play { start, end } => {
                            self.replay.range_input.start = start;
                            self.replay.range_input.end = end;
                            let task = self.update(Message::Replay(ReplayMessage::Play));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::Pause => {
                            let task = self.update(Message::Replay(ReplayMessage::Pause));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::Resume => {
                            let task = self.update(Message::Replay(ReplayMessage::Resume));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::StepForward => {
                            let task = self.update(Message::Replay(ReplayMessage::StepForward));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::StepBackward => {
                            let task = self.update(Message::Replay(ReplayMessage::StepBackward));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::CycleSpeed => {
                            let task = self.update(Message::Replay(ReplayMessage::CycleSpeed));
                            reply_tx.send(reply_replay_status(self));
                            return task;
                        }
                        ReplayCommand::SaveState => {
                            let empty_windows = HashMap::new();
                            self.save_state_to_disk(&empty_windows);
                            reply_tx.send(reply_replay_status(self));
                        }
                    },
                    ApiCommand::Pane(cmd) => {
                        let (body, task) = self.handle_pane_api(cmd);
                        reply_tx.send(body);
                        return task;
                    }
                    ApiCommand::Auth(cmd) => {
                        let body = self.handle_auth_api(cmd);
                        reply_tx.send(body);
                    }
                    #[cfg(feature = "e2e-mock")]
                    ApiCommand::Test(cmd) => {
                        let (body, task) = self.handle_test_api(cmd);
                        reply_tx.send(body);
                        return task;
                    }
                }
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        // ログインウィンドウのビュー
        if Some(id) == self.login_window {
            return self.login_screen.view().map(Message::Login);
        }

        let dashboard = self.active_dashboard();
        let sidebar_pos = self.sidebar.position();

        let tickers_table = &self.sidebar.tickers_table;

        let content = if id == self.main_window.id {
            let sidebar_view = self
                .sidebar
                .view(self.audio_stream.volume())
                .map(Message::Sidebar);

            let dashboard_view = dashboard
                .view(
                    &self.main_window,
                    tickers_table,
                    self.timezone,
                    self.replay.is_replay(),
                )
                .map(move |msg| Message::Dashboard {
                    layout_id: None,
                    event: msg,
                });

            let header_title = {
                #[cfg(target_os = "macos")]
                {
                    iced::widget::center(
                        text("FLOWSURFACE")
                            .font(iced::Font {
                                weight: iced::font::Weight::Bold,
                                ..Default::default()
                            })
                            .size(16)
                            .style(style::title_text),
                    )
                    .height(20)
                    .align_y(Alignment::Center)
                    .padding(padding::top(4))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    column![]
                }
            };

            let replay_header = self.view_replay_header();

            let base = column![
                header_title,
                replay_header,
                match sidebar_pos {
                    sidebar::Position::Left => row![sidebar_view, dashboard_view,],
                    sidebar::Position::Right => row![dashboard_view, sidebar_view],
                }
                .spacing(4)
                .padding(8),
            ];

            if let Some(menu) = self.sidebar.active_menu() {
                self.view_with_modal(base.into(), dashboard, menu)
            } else {
                base.into()
            }
        } else {
            container(
                dashboard
                    .view_window(id, &self.main_window, tickers_table, self.timezone)
                    .map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    }),
            )
            .padding(padding::top(style::TITLE_PADDING_TOP))
            .into()
        };

        toast::Manager::new(
            content,
            self.notifications.toasts(),
            match sidebar_pos {
                sidebar::Position::Left => Alignment::Start,
                sidebar::Position::Right => Alignment::End,
            },
            Message::RemoveNotification,
        )
        .into()
    }

    fn view_replay_header(&self) -> Element<'_, Message> {
        let time_display = text(replay::format_current_time(&self.replay, self.timezone))
            .font(style::AZERET_MONO)
            .size(12);

        let is_replay = self.replay.is_replay();

        let mode_label = if is_replay { "REPLAY" } else { "LIVE" };
        let mode_toggle = button(text(mode_label).size(11))
            .on_press(Message::Replay(ReplayMessage::ToggleMode))
            .style(move |theme, status| style::button::bordered_toggle(theme, status, is_replay))
            .padding(padding::all(2).left(6).right(6));

        let mut start_input = text_input("Start", &self.replay.range_input.start).size(11);
        let mut end_input = text_input("End", &self.replay.range_input.end).size(11);
        if is_replay {
            start_input =
                start_input.on_input(|s| Message::Replay(ReplayMessage::StartTimeChanged(s)));
            end_input = end_input.on_input(|s| Message::Replay(ReplayMessage::EndTimeChanged(s)));
        }

        let is_loading = self.replay.is_loading();
        let is_playing = self.replay.is_playing();
        let is_paused = self.replay.is_paused();
        let has_clock = self.replay.clock.is_some();

        let play_pause_label = if is_playing { "\u{23F8}" } else { "\u{25B6}" };
        let mut play_pause_btn =
            button(text(play_pause_label).size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay && !is_loading {
            play_pause_btn = play_pause_btn.on_press(if is_playing {
                Message::Replay(ReplayMessage::Pause)
            } else if is_paused {
                Message::Replay(ReplayMessage::Resume)
            } else {
                Message::Replay(ReplayMessage::Play)
            });
        }

        // ⏮ StepBackward
        let mut step_back_btn =
            button(text("\u{23EE}").size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay && has_clock && !is_loading {
            step_back_btn = step_back_btn.on_press(Message::Replay(ReplayMessage::StepBackward));
        }

        // ⏭ StepForward
        let mut step_fwd_btn =
            button(text("\u{23ED}").size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay && !is_loading {
            step_fwd_btn = step_fwd_btn.on_press(Message::Replay(ReplayMessage::StepForward));
        }

        // Speed button
        let speed_label = self.replay.speed_label();
        let mut speed_btn =
            button(text(speed_label).size(11)).padding(padding::all(2).left(4).right(4));
        if is_replay && has_clock && !is_loading {
            speed_btn = speed_btn.on_press(Message::Replay(ReplayMessage::CycleSpeed));
        }
        let speed_tooltip: Element<'_, Message> = iced::widget::tooltip(
            speed_btn,
            container(
                text("M30 以下: 実時間連動 × speed / H1 以上: 1 バー/秒 × speed").size(11),
            )
            .style(style::tooltip)
            .padding(6),
            TooltipPosition::Top,
        )
        .into();

        let controls = row![step_back_btn, play_pause_btn, step_fwd_btn, speed_tooltip].spacing(4);

        let mut header = row![
            time_display,
            mode_toggle,
            start_input.width(140),
            text("~").size(11),
            end_input.width(140),
            controls,
        ];

        if is_loading {
            header = header.push(text("Loading...").size(11));
        }

        header
            .spacing(8)
            .padding(padding::all(4))
            .align_y(Alignment::Center)
            .into()
    }

    fn theme(&self, _window: window::Id) -> iced_core::Theme {
        self.theme.clone().into()
    }

    fn title(&self, window: window::Id) -> String {
        if Some(window) == self.login_window {
            return "kabu STATION ログイン".to_string();
        }
        if let Some(id) = self.layout_manager.active_layout_id() {
            format!("Flowsurface [{}]", id.name)
        } else {
            "Flowsurface".to_string()
        }
    }

    fn scale_factor(&self, _window: window::Id) -> f32 {
        self.ui_scale_factor.into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let window_events = window::events().map(Message::WindowEvent);
        let sidebar = self.sidebar.subscription().map(Message::Sidebar);
        let replay_api = Subscription::run(replay_api::subscription).map(Message::ReplayApi);

        // ログイン画面中はexchangeストリームを購読しない
        if self.login_window.is_some() {
            return Subscription::batch(vec![window_events, sidebar, replay_api]);
        }

        let tick = iced::window::frames().map(Message::Tick);

        let hotkeys = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed { key, .. } = event else {
                return None;
            };
            match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::GoBack),
                keyboard::Key::Named(keyboard::key::Named::F5) => {
                    Some(Message::Replay(ReplayMessage::ToggleMode))
                }
                _ => None,
            }
        });

        // リプレイモード中は WebSocket ストリームを購読しない
        if self.replay.is_replay() {
            return Subscription::batch(vec![window_events, sidebar, tick, hotkeys, replay_api]);
        }

        log::info!("[e2e-live] Live mode: building market subscriptions");
        let exchange_streams = self
            .active_dashboard()
            .market_subscriptions()
            .map(Message::MarketWsEvent);

        Subscription::batch(vec![
            exchange_streams,
            sidebar,
            window_events,
            tick,
            hotkeys,
            replay_api,
        ])
    }

    fn active_dashboard(&self) -> &Dashboard {
        let active_layout = self
            .layout_manager
            .active_layout_id()
            .expect("No active layout");
        self.layout_manager
            .get(active_layout.unique)
            .map(|layout| &layout.dashboard)
            .expect("No active dashboard")
    }

    fn active_dashboard_mut(&mut self) -> &mut Dashboard {
        let active_layout = self
            .layout_manager
            .active_layout_id()
            .expect("No active layout");
        self.layout_manager
            .get_mut(active_layout.unique)
            .map(|layout| &mut layout.dashboard)
            .expect("No active dashboard")
    }

    /// 認証状態確認 API コマンドを処理する（本番ビルドにも含まれる）。
    fn handle_auth_api(&self, cmd: replay_api::AuthCommand) -> String {
        use replay_api::AuthCommand;
        match cmd {
            AuthCommand::TachibanaSessionStatus => {
                let present = connector::auth::get_session().is_some();
                serde_json::json!({
                    "session": if present { "present" } else { "none" }
                })
                .to_string()
            }
        }
    }

    /// Pane CRUD API コマンドを処理する。
    /// 返り値: (JSON レスポンス, 続行する Task)。
    /// E2E テスト用 fixture 注入コマンドを処理する。
    /// `e2e-mock` feature でのみコンパイルされ、本番ビルドには含まれない。
    /// 詳細: docs/plan/tachibana_e2e_phase_t1.md
    #[cfg(feature = "e2e-mock")]
    fn handle_test_api(
        &mut self,
        cmd: replay_api::TestCommand,
    ) -> (String, Task<Message>) {
        use exchange::adapter::tachibana;
        use replay_api::TestCommand;

        match cmd {
            TestCommand::TachibanaInjectSession => {
                connector::auth::inject_dummy_session();
                (
                    serde_json::json!({"ok": true, "action": "inject-session"}).to_string(),
                    Task::none(),
                )
            }
            TestCommand::TachibanaInjectMaster { raw_body } => {
                // body 形式: {"records": [{"sIssueCode":"7203", ...}, ...]}
                let err_body = |msg: String| -> (String, Task<Message>) {
                    (format!(r#"{{"error":"{}"}}"#, msg.replace('"', "'")), Task::none())
                };
                let parsed: serde_json::Value = match serde_json::from_str(&raw_body) {
                    Ok(v) => v,
                    Err(e) => return err_body(format!("invalid JSON body: {e}")),
                };
                let records_value = match parsed.get("records") {
                    Some(v) => v.clone(),
                    None => return err_body("missing 'records' field".to_string()),
                };
                let records: Vec<tachibana::MasterRecord> =
                    match serde_json::from_value(records_value) {
                        Ok(r) => r,
                        Err(e) => return err_body(format!("failed to parse records: {e}")),
                    };

                // MasterRecord のデフォルト sCLMID は空。master_record_to_ticker_info が
                // "CLMIssueMstKabu" でフィルタするため、空のままだと無視されてしまう。
                // テスト注入では issue_code があるレコードを Kabu 扱いにする。
                let count = records.len();
                let normalized: Vec<tachibana::MasterRecord> = records
                    .into_iter()
                    .map(|mut r| {
                        if r.clm_id.is_empty() {
                            r.clm_id = "CLMIssueMstKabu".to_string();
                        }
                        r
                    })
                    .collect();

                tachibana::e2e_mock::inject_master_cache(normalized);

                // ISSUE_MASTER_CACHE は埋まったが、Sidebar 側の `tickers_info` は
                // `Message::Sidebar(TickersTable(UpdateMetadata))` で更新される。
                // `cached_ticker_metadata()` は内部的に std::sync::RwLock を読むだけなので、
                // Task::perform 経由で async ブリッジしつつ UpdateMetadata を発火する。
                use exchange::adapter::Venue;
                let task = Task::perform(
                    async { exchange::adapter::tachibana::cached_ticker_metadata().await },
                    |metadata| {
                        Message::Sidebar(dashboard::sidebar::Message::TickersTable(
                            dashboard::tickers_table::Message::UpdateMetadata(
                                Venue::Tachibana,
                                metadata,
                            ),
                        ))
                    },
                );

                let body = serde_json::json!({
                    "ok": true,
                    "action": "inject-master",
                    "count": count,
                })
                .to_string();
                (body, task)
            }
            TestCommand::TachibanaInjectDailyHistory { raw_body } => {
                // body 形式: {"issue_code":"7203","klines":[{"time":...,"open":...,"high":...,"low":...,"close":...,"volume":...}, ...]}
                let err_body = |msg: String| -> (String, Task<Message>) {
                    (format!(r#"{{"error":"{}"}}"#, msg.replace('"', "'")), Task::none())
                };
                let parsed: serde_json::Value = match serde_json::from_str(&raw_body) {
                    Ok(v) => v,
                    Err(e) => return err_body(format!("invalid JSON body: {e}")),
                };
                let Some(issue_code) = parsed
                    .get("issue_code")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                else {
                    return err_body("missing 'issue_code' field".to_string());
                };
                let Some(klines_arr) = parsed.get("klines").and_then(|v| v.as_array()) else {
                    return err_body("missing 'klines' field or not an array".to_string());
                };

                let mut klines: Vec<exchange::Kline> = Vec::with_capacity(klines_arr.len());
                for item in klines_arr.iter() {
                    let Some(time) = item.get("time").and_then(|v| v.as_u64()) else {
                        return err_body("kline missing 'time' (u64)".to_string());
                    };
                    let Some(open) = item.get("open").and_then(|v| v.as_f64()) else {
                        return err_body("kline missing 'open' (number)".to_string());
                    };
                    let Some(high) = item.get("high").and_then(|v| v.as_f64()) else {
                        return err_body("kline missing 'high' (number)".to_string());
                    };
                    let Some(low) = item.get("low").and_then(|v| v.as_f64()) else {
                        return err_body("kline missing 'low' (number)".to_string());
                    };
                    let Some(close) = item.get("close").and_then(|v| v.as_f64()) else {
                        return err_body("kline missing 'close' (number)".to_string());
                    };
                    let Some(volume) = item.get("volume").and_then(|v| v.as_f64()) else {
                        return err_body("kline missing 'volume' (number)".to_string());
                    };

                    // 日本株は整数円なので min_ticksize = 10^0 = 1（daily_record_to_kline と同じ前提）
                    let min_ticksize = exchange::unit::MinTicksize::new(0);
                    let qty = exchange::unit::qty::Qty::from_f32(volume as f32);
                    klines.push(exchange::Kline::new(
                        time,
                        open as f32,
                        high as f32,
                        low as f32,
                        close as f32,
                        exchange::Volume::TotalOnly(qty),
                        min_ticksize,
                    ));
                }

                let count = klines.len();
                tachibana::e2e_mock::inject_daily_klines(issue_code.clone(), klines);

                let body = serde_json::json!({
                    "ok": true,
                    "action": "inject-daily-history",
                    "issue_code": issue_code,
                    "count": count,
                })
                .to_string();
                (body, Task::none())
            }

            // ── Phase T2: inject-market-price ──────────────────────────────
            TestCommand::TachibanaInjectMarketPrice { raw_body } => {
                // body 形式: {"records": [{"sIssueCode":"7203","pDPP":"3000.0",...}, ...]}
                let err_body = |msg: String| -> (String, Task<Message>) {
                    (format!(r#"{{"error":"{}"}}"#, msg.replace('"', "'")), Task::none())
                };
                let parsed: serde_json::Value = match serde_json::from_str(&raw_body) {
                    Ok(v) => v,
                    Err(e) => return err_body(format!("invalid JSON body: {e}")),
                };
                let records_value = match parsed.get("records") {
                    Some(v) => v.clone(),
                    None => return err_body("missing 'records' field".to_string()),
                };
                let records: Vec<tachibana::MarketPriceRecord> =
                    match serde_json::from_value(records_value) {
                        Ok(r) => r,
                        Err(e) => return err_body(format!("failed to parse records: {e}")),
                    };
                let count = records.len();
                tachibana::e2e_mock::inject_market_prices(records);

                let body = serde_json::json!({
                    "ok": true,
                    "action": "inject-market-price",
                    "count": count,
                })
                .to_string();
                (body, Task::none())
            }

            // ── Phase T3: keyring 永続化テスト ──────────────────────────────
            TestCommand::TachibanaInjectPersistSession => {
                connector::auth::persist_injected_session();
                (
                    serde_json::json!({"ok": true, "action": "persist-session"}).to_string(),
                    Task::none(),
                )
            }
            TestCommand::TachibanaDeletePersistedSession => {
                connector::auth::delete_all_sessions();
                (
                    serde_json::json!({"ok": true, "action": "delete-persisted-session"})
                        .to_string(),
                    Task::none(),
                )
            }
        }
    }

    fn handle_pane_api(
        &mut self,
        cmd: replay_api::PaneCommand,
    ) -> (String, Task<Message>) {
        use replay_api::PaneCommand;

        match cmd {
            PaneCommand::ListPanes => {
                let json = self.build_pane_list_json();
                (json, Task::none())
            }
            PaneCommand::Split { pane_id, axis } => self.pane_api_split(pane_id, &axis),
            PaneCommand::Close { pane_id } => self.pane_api_close(pane_id),
            PaneCommand::SetTicker { pane_id, ticker } => {
                self.pane_api_set_ticker(pane_id, &ticker)
            }
            PaneCommand::SetTimeframe { pane_id, timeframe } => {
                self.pane_api_set_timeframe(pane_id, &timeframe)
            }
            PaneCommand::SidebarSelectTicker {
                pane_id,
                ticker,
                kind,
            } => self.pane_api_sidebar_select_ticker(pane_id, &ticker, kind.as_deref()),
            PaneCommand::ListNotifications => {
                let json = self.build_notification_list_json();
                (json, Task::none())
            }
        }
    }

    /// 現在の通知一覧を JSON シリアライズする。
    fn build_notification_list_json(&self) -> String {
        use widget::toast::Status;
        let items: Vec<serde_json::Value> = self
            .notifications
            .toasts()
            .iter()
            .map(|t| {
                let level = match t.status() {
                    Status::Danger => "error",
                    Status::Warning => "warning",
                    Status::Success => "success",
                    Status::Primary => "info",
                    Status::Secondary => "info",
                };
                serde_json::json!({
                    "title": t.title(),
                    "body": t.body(),
                    "level": level,
                })
            })
            .collect();
        let body = serde_json::json!({ "notifications": items });
        serde_json::to_string(&body).unwrap_or_else(|_| {
            r#"{"error":"failed to serialize notifications"}"#.to_string()
        })
    }

    /// 現在のアクティブレイアウトの全ペインを JSON シリアライズする。
    fn build_pane_list_json(&self) -> String {
        let main_window_id = self.main_window.id;
        let dashboard = self.active_dashboard();
        let pending_streams: Vec<String> = Vec::new();
        let trade_buffer_streams: Vec<String> = self
            .replay
            .active_streams
            .iter()
            .map(|s| format!("{s:?}"))
            .collect();

        let panes: Vec<serde_json::Value> = dashboard
            .iter_all_panes(main_window_id)
            .map(|(window_id, _pg_pane, state)| {
                let kind = state.content.kind().to_string();

                // ticker / timeframe の抽出（Ready → Waiting の順にフォールバック）
                let (ticker, timeframe) = extract_pane_ticker_timeframe(&state.streams);

                serde_json::json!({
                    "id": state.unique_id().to_string(),
                    "window_id": format!("{window_id:?}"),
                    "type": kind,
                    "ticker": ticker,
                    "timeframe": timeframe,
                    "link_group": state.link_group.map(|g| format!("{g:?}")),
                })
            })
            .collect();

        let body = serde_json::json!({
            "panes": panes,
            "pending_trade_streams": pending_streams,
            "trade_buffer_streams": trade_buffer_streams,
        });
        serde_json::to_string(&body).unwrap_or_else(|_| {
            r#"{"error":"failed to serialize pane list"}"#.to_string()
        })
    }

    /// uuid から (window_id, pane_grid::Pane) を検索する。
    fn find_pane_handle(
        &self,
        pane_id: uuid::Uuid,
    ) -> Option<(window::Id, pane_grid::Pane)> {
        let main_window_id = self.main_window.id;
        self.active_dashboard()
            .iter_all_panes(main_window_id)
            .find(|(_, _, state)| state.unique_id() == pane_id)
            .map(|(win, pg, _)| (win, pg))
    }

    fn pane_api_split(&mut self, pane_id: uuid::Uuid, axis_str: &str) -> (String, Task<Message>) {
        let axis = match axis_str {
            "Vertical" | "vertical" => pane_grid::Axis::Vertical,
            "Horizontal" | "horizontal" => pane_grid::Axis::Horizontal,
            _ => {
                return (
                    format!(
                        r#"{{"error":"invalid axis: {axis_str} (expected Vertical or Horizontal)"}}"#
                    ),
                    Task::none(),
                );
            }
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let task = self.update(Message::Dashboard {
            layout_id: None,
            event: dashboard::Message::Pane(
                window_id,
                dashboard::pane::Message::SplitPane(axis, pg_pane),
            ),
        });
        let ok = serde_json::json!({"ok": true, "action": "split", "pane_id": pane_id.to_string()});
        (ok.to_string(), task)
    }

    fn pane_api_close(&mut self, pane_id: uuid::Uuid) -> (String, Task<Message>) {
        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let task = self.update(Message::Dashboard {
            layout_id: None,
            event: dashboard::Message::Pane(
                window_id,
                dashboard::pane::Message::ClosePane(pg_pane),
            ),
        });
        let ok = serde_json::json!({"ok": true, "action": "close", "pane_id": pane_id.to_string()});
        (ok.to_string(), task)
    }

    /// "BinanceLinear:BTCUSDT" を (Exchange, Ticker) にパースする。
    fn parse_ser_ticker(s: &str) -> Result<exchange::Ticker, String> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "invalid ticker format: expected 'Exchange:Ticker', got '{s}'"
            ));
        }
        let exchange_str = parts[0];
        let normalized = ["Linear", "Inverse", "Spot"]
            .into_iter()
            .find_map(|suffix| {
                exchange_str
                    .strip_suffix(suffix)
                    .map(|prefix| format!("{prefix} {suffix}"))
            })
            .unwrap_or_else(|| exchange_str.to_owned());
        let exchange: exchange::adapter::Exchange = normalized
            .parse()
            .map_err(|_| format!("unknown exchange: {exchange_str}"))?;
        let ticker = exchange::Ticker::new(parts[1], exchange);
        Ok(ticker)
    }

    fn pane_api_set_ticker(
        &mut self,
        pane_id: uuid::Uuid,
        ticker_str: &str,
    ) -> (String, Task<Message>) {
        let ticker = match Self::parse_ser_ticker(ticker_str) {
            Ok(t) => t,
            Err(err) => {
                return (
                    format!(r#"{{"error":"{}"}}"#, err.replace('"', "'")),
                    Task::none(),
                );
            }
        };

        let ticker_info = self
            .sidebar
            .tickers_info()
            .get(&ticker)
            .and_then(|opt| *opt);
        let Some(ticker_info) = ticker_info else {
            return (
                format!(
                    r#"{{"error":"ticker info not loaded yet: {ticker_str} (wait for metadata fetch)"}}"#
                ),
                Task::none(),
            );
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let main_window_id = self.main_window.id;
        let dashboard = self.active_dashboard_mut();

        // 既存コンテンツの kind を保ったまま ticker を差し替える。
        // init_focused_pane と同じ副作用を得るため、一時的に focus を対象ペインに移す。
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        // Starter pane (split 直後など) は set_content_and_streams で unreachable!() に当たるため
        // CandlestickChart にフォールバックする（UI で ticker_table 経由で選んだ時の既定と同じ挙動）。
        let kind = dashboard
            .iter_all_panes(main_window_id)
            .find(|(_, p, _)| *p == pg_pane)
            .map(|(_, _, state)| state.content.kind())
            .map(|k| match k {
                data::layout::pane::ContentKind::Starter => {
                    data::layout::pane::ContentKind::CandlestickChart
                }
                other => other,
            })
            .unwrap_or(data::layout::pane::ContentKind::CandlestickChart);

        let task = dashboard
            .init_focused_pane(main_window_id, ticker_info, kind)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            })
            .chain(Task::done(Message::Replay(
                ReplayMessage::SyncReplayBuffers,
            )));

        // focus を元に戻す
        let dashboard = self.active_dashboard_mut();
        dashboard.focus = prev_focus;

        let ok = serde_json::json!({
            "ok": true,
            "action": "set-ticker",
            "pane_id": pane_id.to_string(),
            "ticker": ticker_str,
        });
        (ok.to_string(), task)
    }

    fn parse_timeframe(s: &str) -> Option<exchange::Timeframe> {
        match s {
            "MS100" => Some(exchange::Timeframe::MS100),
            "MS200" => Some(exchange::Timeframe::MS200),
            "MS300" => Some(exchange::Timeframe::MS300),
            "MS500" => Some(exchange::Timeframe::MS500),
            "MS1000" | "S1" => Some(exchange::Timeframe::MS1000),
            "M1" => Some(exchange::Timeframe::M1),
            "M3" => Some(exchange::Timeframe::M3),
            "M5" => Some(exchange::Timeframe::M5),
            "M15" => Some(exchange::Timeframe::M15),
            "M30" => Some(exchange::Timeframe::M30),
            "H1" => Some(exchange::Timeframe::H1),
            "H2" => Some(exchange::Timeframe::H2),
            "H4" => Some(exchange::Timeframe::H4),
            "H12" => Some(exchange::Timeframe::H12),
            "D1" => Some(exchange::Timeframe::D1),
            _ => None,
        }
    }

    fn pane_api_set_timeframe(
        &mut self,
        pane_id: uuid::Uuid,
        tf_str: &str,
    ) -> (String, Task<Message>) {
        let tf = match Self::parse_timeframe(tf_str) {
            Some(tf) => tf,
            None => {
                return (
                    format!(r#"{{"error":"invalid timeframe: {tf_str}"}}"#),
                    Task::none(),
                );
            }
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        // 対象ペインの現在の ticker_info と kind を取得
        let main_window_id = self.main_window.id;
        let (ticker_info, kind) = {
            let dashboard = self.active_dashboard();
            let Some((_, _, state)) = dashboard
                .iter_all_panes(main_window_id)
                .find(|(_, p, _)| *p == pg_pane)
            else {
                return (
                    format!(r#"{{"error":"pane not found in dashboard: {pane_id}"}}"#),
                    Task::none(),
                );
            };
            let Some(ti) = state.stream_pair() else {
                return (
                    format!(
                        r#"{{"error":"pane has no active ticker to rebase timeframe: {pane_id}"}}"#
                    ),
                    Task::none(),
                );
            };
            (ti, state.content.kind())
        };

        // settings.selected_basis を書き換えてから init_focused_pane を呼ぶ。
        // これは BasisSelected 経路と同等の effect (stream 再構築 + refresh_streams) を得るための近道。
        let dashboard = self.active_dashboard_mut();
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        if let Some(state) = dashboard
            .iter_all_panes_mut(main_window_id)
            .find(|(_, p, _)| *p == pg_pane)
            .map(|(_, _, s)| s)
        {
            state.settings.selected_basis =
                Some(data::chart::Basis::Time(tf));
        }

        let task = dashboard
            .init_focused_pane(main_window_id, ticker_info, kind)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            })
            .chain(Task::done(Message::Replay(
                ReplayMessage::SyncReplayBuffers,
            )));

        let dashboard = self.active_dashboard_mut();
        dashboard.focus = prev_focus;

        let ok = serde_json::json!({
            "ok": true,
            "action": "set-timeframe",
            "pane_id": pane_id.to_string(),
            "timeframe": tf_str,
        });
        (ok.to_string(), task)
    }

    /// "CandlestickChart" / "HeatmapChart" / "ShaderHeatmap" ... を ContentKind にパースする。
    fn parse_content_kind(s: &str) -> Option<data::layout::pane::ContentKind> {
        use data::layout::pane::ContentKind;
        match s {
            "CandlestickChart" | "Candlestick Chart" => Some(ContentKind::CandlestickChart),
            "HeatmapChart" | "Heatmap Chart" => Some(ContentKind::HeatmapChart),
            "ShaderHeatmap" | "Shader Heatmap" => Some(ContentKind::ShaderHeatmap),
            "FootprintChart" | "Footprint Chart" => Some(ContentKind::FootprintChart),
            "ComparisonChart" | "Comparison Chart" => Some(ContentKind::ComparisonChart),
            "TimeAndSales" | "Time&Sales" => Some(ContentKind::TimeAndSales),
            "Ladder" => Some(ContentKind::Ladder),
            "Starter" | "Starter Pane" => Some(ContentKind::Starter),
            _ => None,
        }
    }

    /// Sidebar::TickerSelected 経路のハンドラ（Phase 8 Fix 4 検証用）。
    /// `main.rs::Message::Sidebar` ハンドラと同じタスク構成を踏む:
    /// - `kind == None` → `switch_tickers_in_group`
    /// - `kind == Some(kind)` → `init_focused_pane`
    /// - 最後に `SyncReplayBuffers` を chain（heatmap-only で init_focused_pane が Task::none() を
    ///   返す経路でも sync が発火することを保証）
    fn pane_api_sidebar_select_ticker(
        &mut self,
        pane_id: uuid::Uuid,
        ticker_str: &str,
        kind_str: Option<&str>,
    ) -> (String, Task<Message>) {
        let ticker = match Self::parse_ser_ticker(ticker_str) {
            Ok(t) => t,
            Err(err) => {
                return (
                    format!(r#"{{"error":"{}"}}"#, err.replace('"', "'")),
                    Task::none(),
                );
            }
        };

        let ticker_info = self
            .sidebar
            .tickers_info()
            .get(&ticker)
            .and_then(|opt| *opt);
        let Some(ticker_info) = ticker_info else {
            return (
                format!(
                    r#"{{"error":"ticker info not loaded yet: {ticker_str} (wait for metadata fetch)"}}"#
                ),
                Task::none(),
            );
        };

        let kind = match kind_str {
            Some(s) => match Self::parse_content_kind(s) {
                Some(k) => Some(k),
                None => {
                    return (
                        format!(r#"{{"error":"invalid kind: {s}"}}"#),
                        Task::none(),
                    );
                }
            },
            None => None,
        };

        let Some((window_id, pg_pane)) = self.find_pane_handle(pane_id) else {
            return (
                format!(r#"{{"error":"pane not found: {pane_id}"}}"#),
                Task::none(),
            );
        };

        let main_window_id = self.main_window.id;
        let dashboard = self.active_dashboard_mut();
        let prev_focus = dashboard.focus;
        dashboard.focus = Some((window_id, pg_pane));

        // main.rs::Message::Sidebar と同じ分岐
        let task = if let Some(kind) = kind {
            dashboard.init_focused_pane(main_window_id, ticker_info, kind)
        } else {
            dashboard.switch_tickers_in_group(main_window_id, ticker_info)
        };

        let task = task
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            })
            .chain(Task::done(Message::Replay(
                ReplayMessage::SyncReplayBuffers,
            )));

        let dashboard = self.active_dashboard_mut();
        dashboard.focus = prev_focus;

        let ok = serde_json::json!({
            "ok": true,
            "action": "sidebar-select-ticker",
            "pane_id": pane_id.to_string(),
            "ticker": ticker_str,
            "kind": kind_str,
        });
        (ok.to_string(), task)
    }

    /// ログインウィンドウを閉じてメインウィンドウ（ダッシュボード）を開く。
    /// LoginCompleted(Ok) と SessionRestoreResult(Some) の共通処理。
    fn transition_to_dashboard(&mut self) -> Task<Message> {
        let login_win = self.login_window.take();

        let (main_window_id, open_main) = {
            let (position, size) = (
                window::Position::Centered,
                self.saved_main_window_spec
                    .map_or_else(crate::window::default_size, |w| w.size()),
            );
            window::open(window::Settings {
                size,
                position,
                exit_on_close_request: false,
                ..window::settings()
            })
        };

        self.main_window = window::Window::new(main_window_id);

        let active_layout_id = self.layout_manager.active_layout_id().unwrap_or(
            &self
                .layout_manager
                .layouts
                .first()
                .expect("No layouts available")
                .id,
        );
        let load_layout = self.load_layout(active_layout_id.unique, main_window_id);

        let close_login = login_win
            .map(window::close::<Message>)
            .unwrap_or_else(Task::none);

        close_login
            .chain(open_main.discard())
            .chain(iced::window::maximize(main_window_id, true))
            .chain(load_layout)
    }

    /// 銘柄マスタをバックグラウンドでダウンロードし、完了後に TickersTable へ反映する。
    fn start_master_download(
        session: exchange::adapter::tachibana::TachibanaSession,
    ) -> Task<Message> {
        use exchange::adapter::Venue;

        Task::perform(
            async move {
                let client = reqwest::Client::new();
                exchange::adapter::tachibana::init_issue_master(&client, &session).await?;
                Ok(exchange::adapter::tachibana::cached_ticker_metadata().await)
            },
            |result: Result<_, exchange::adapter::tachibana::TachibanaError>| {
                let venue = Venue::Tachibana;
                match result {
                    Ok(metadata) => Message::Sidebar(dashboard::sidebar::Message::TickersTable(
                        dashboard::tickers_table::Message::UpdateMetadata(venue, metadata),
                    )),
                    Err(e) => {
                        log::error!("Tachibana master download failed: {e}");
                        Message::Sidebar(dashboard::sidebar::Message::TickersTable(
                            dashboard::tickers_table::Message::MetadataFetchFailed(
                                venue,
                                data::InternalError::Fetch(format!("Tachibana: {e}")),
                            ),
                        ))
                    }
                }
            },
        )
    }

    fn load_layout(&mut self, layout_uid: uuid::Uuid, main_window: window::Id) -> Task<Message> {
        if let Err(err) = self.layout_manager.set_active_layout(layout_uid) {
            log::error!("Failed to set active layout: {}", err);
            return Task::none();
        }

        self.layout_manager
            .park_inactive_layouts(layout_uid, main_window);

        self.layout_manager
            .get_mut(layout_uid)
            .map(|layout| {
                layout
                    .dashboard
                    .load_layout(main_window)
                    .map(move |msg| Message::Dashboard {
                        layout_id: Some(layout_uid),
                        event: msg,
                    })
            })
            .unwrap_or_else(|| {
                log::error!("Active layout missing after selection: {}", layout_uid);
                Task::none()
            })
    }

    fn view_with_modal<'a>(
        &'a self,
        base: Element<'a, Message>,
        dashboard: &'a Dashboard,
        menu: sidebar::Menu,
    ) -> Element<'a, Message> {
        let sidebar_pos = self.sidebar.position();

        match menu {
            sidebar::Menu::Settings => {
                let settings_modal = {
                    let theme_picklist = {
                        let mut themes: Vec<iced::Theme> = iced_core::Theme::ALL.to_vec();

                        let default_theme = iced_core::Theme::Custom(default_theme().into());
                        themes.push(default_theme);

                        if let Some(custom_theme) = &self.theme_editor.custom_theme {
                            themes.push(custom_theme.clone());
                        }

                        pick_list(themes, Some(self.theme.0.clone()), |theme| {
                            Message::ThemeSelected(theme)
                        })
                    };

                    let toggle_theme_editor = button(text("Theme editor")).on_press(
                        Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(Some(
                            sidebar::Menu::ThemeEditor,
                        ))),
                    );

                    let toggle_network_editor = button(text("Network")).on_press(Message::Sidebar(
                        dashboard::sidebar::Message::ToggleSidebarMenu(Some(
                            sidebar::Menu::Network,
                        )),
                    ));

                    let timezone_picklist = pick_list(
                        [data::UserTimezone::Utc, data::UserTimezone::Local],
                        Some(self.timezone),
                        Message::SetTimezone,
                    );

                    let size_in_quote_currency_checkbox = {
                        let is_active = match self.volume_size_unit {
                            exchange::SizeUnit::Quote => true,
                            exchange::SizeUnit::Base => false,
                        };

                        let checkbox = iced::widget::checkbox(is_active)
                            .label("Size in quote currency")
                            .on_toggle(|checked| {
                                let on_dialog_confirm = Message::ApplyVolumeSizeUnit(if checked {
                                    exchange::SizeUnit::Quote
                                } else {
                                    exchange::SizeUnit::Base
                                });

                                let confirm_dialog = screen::ConfirmDialog::new(
                                    "Changing size display currency requires application restart"
                                        .to_string(),
                                    Box::new(on_dialog_confirm.clone()),
                                )
                                .with_confirm_btn_text("Restart now".to_string());

                                Message::ToggleDialogModal(Some(confirm_dialog))
                            });

                        tooltip(
                            checkbox,
                            Some(
                                "Display sizes/volumes in quote currency (USD)\nHas no effect on inverse perps or open interest",
                            ),
                            TooltipPosition::Top,
                        )
                    };

                    let sidebar_pos_picklist = pick_list(
                        [sidebar::Position::Left, sidebar::Position::Right],
                        Some(sidebar_pos),
                        |pos| {
                            Message::Sidebar(dashboard::sidebar::Message::SetSidebarPosition(pos))
                        },
                    );

                    let scale_factor = {
                        let current_value: f32 = self.ui_scale_factor.into();

                        let decrease_btn = if current_value > data::config::MIN_SCALE {
                            button(text("-"))
                                .on_press(Message::ScaleFactorChanged((current_value - 0.1).into()))
                        } else {
                            button(text("-"))
                        };

                        let increase_btn = if current_value < data::config::MAX_SCALE {
                            button(text("+"))
                                .on_press(Message::ScaleFactorChanged((current_value + 0.1).into()))
                        } else {
                            button(text("+"))
                        };

                        container(
                            row![
                                decrease_btn,
                                text(format!("{:.0}%", current_value * 100.0)).size(14),
                                increase_btn,
                            ]
                            .align_y(Alignment::Center)
                            .spacing(8)
                            .padding(4),
                        )
                        .style(style::modal_container)
                    };

                    let trade_fetch_checkbox = {
                        let is_active = connector::fetcher::is_trade_fetch_enabled();

                        let checkbox = iced::widget::checkbox(is_active)
                            .label("Fetch trades (Binance)")
                            .on_toggle(|checked| {
                                if checked {
                                    let confirm_dialog = screen::ConfirmDialog::new(
                                        "This might be unreliable and take some time to complete. Proceed?"
                                            .to_string(),
                                        Box::new(Message::ToggleTradeFetch(true)),
                                    );
                                    Message::ToggleDialogModal(Some(confirm_dialog))
                                } else {
                                    Message::ToggleTradeFetch(false)
                                }
                            });

                        tooltip(
                            checkbox,
                            Some("Try to fetch trades for footprint charts"),
                            TooltipPosition::Top,
                        )
                    };

                    let open_data_folder = {
                        let button =
                            button(text("Open data folder")).on_press(Message::DataFolderRequested);

                        tooltip(
                            button,
                            Some("Open the folder where the data & config is stored"),
                            TooltipPosition::Top,
                        )
                    };

                    let version_info = {
                        let (version_label, commit_label) = version::app_build_version_parts();

                        let github_link_button = button(text(version_label).size(13))
                            .padding(0)
                            .style(style::button::text_link)
                            .on_press(Message::OpenUrlRequested(Cow::Borrowed(
                                version::GITHUB_REPOSITORY_URL,
                            )));

                        let github_button: Element<'_, Message> = iced::widget::tooltip(
                            github_link_button,
                            container(
                                row![
                                    text("GitHub"),
                                    style::icon_text(style::Icon::ExternalLink, 12),
                                ]
                                .spacing(4)
                                .align_y(Alignment::Center),
                            )
                            .style(style::tooltip)
                            .padding(8),
                            TooltipPosition::Top,
                        )
                        .into();

                        if let (Some(commit_label), Some(commit_url)) =
                            (commit_label, version::build_commit_url())
                        {
                            let commit_button = button(text(commit_label).size(11))
                                .padding(0)
                                .style(style::button::text_link_secondary)
                                .on_press(Message::OpenUrlRequested(Cow::Owned(commit_url)));

                            column![github_button, commit_button]
                                .spacing(2)
                                .align_x(Alignment::End)
                                .into()
                        } else {
                            github_button
                        }
                    };

                    let footer = column![
                        container(version_info)
                            .width(iced::Length::Fill)
                            .align_x(Alignment::End),
                    ]
                    .spacing(8);

                    let column_content = split_column![
                        column![open_data_folder,].spacing(8),
                        column![text("Sidebar position").size(14), sidebar_pos_picklist,].spacing(12),
                        column![text("Time zone").size(14), timezone_picklist,].spacing(12),
                        column![text("Market data").size(14), size_in_quote_currency_checkbox,].spacing(12),
                        column![text("Theme").size(14), theme_picklist,].spacing(12),
                        column![text("Interface scale").size(14), scale_factor,].spacing(12),
                        column![
                            text("Experimental").size(14),
                            column![trade_fetch_checkbox, toggle_theme_editor, toggle_network_editor].spacing(8),
                        ]
                        .spacing(12),
                        footer,
                        ; spacing = 16, align_x = Alignment::Start
                    ];

                    let content = scrollable::Scrollable::with_direction(
                        column_content,
                        scrollable::Direction::Vertical(
                            scrollable::Scrollbar::new().width(8).scroller_width(6),
                        ),
                    );

                    container(content)
                        .align_x(Alignment::Start)
                        .max_width(240)
                        .padding(24)
                        .style(style::dashboard_modal)
                };

                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                let base_content = dashboard_modal(
                    base,
                    settings_modal,
                    Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                );

                if let Some(dialog) = &self.confirm_dialog {
                    let dialog_content =
                        confirm_dialog_container(dialog.clone(), Message::ToggleDialogModal(None));

                    main_dialog_modal(
                        base_content,
                        dialog_content,
                        Message::ToggleDialogModal(None),
                    )
                } else {
                    base_content
                }
            }
            sidebar::Menu::Layout => {
                let main_window = self.main_window.id;

                let manage_pane = if let Some((window_id, pane_id)) = dashboard.focus {
                    let selected_pane_str =
                        if let Some(state) = dashboard.get_pane(main_window, window_id, pane_id) {
                            let link_group_name: String =
                                state.link_group.as_ref().map_or_else(String::new, |g| {
                                    " - Group ".to_string() + &g.to_string()
                                });

                            state.content.to_string() + &link_group_name
                        } else {
                            "".to_string()
                        };

                    let is_main_window = window_id == main_window;

                    let reset_pane_button = {
                        let btn = button(text("Reset").align_x(Alignment::Center))
                            .width(iced::Length::Fill);
                        if is_main_window {
                            let dashboard_msg = Message::Dashboard {
                                layout_id: None,
                                event: dashboard::Message::Pane(
                                    main_window,
                                    dashboard::pane::Message::ReplacePane(pane_id),
                                ),
                            };

                            btn.on_press(dashboard_msg)
                        } else {
                            btn
                        }
                    };
                    let split_pane_button = {
                        let btn = button(text("Split").align_x(Alignment::Center))
                            .width(iced::Length::Fill);
                        if is_main_window {
                            let dashboard_msg = Message::Dashboard {
                                layout_id: None,
                                event: dashboard::Message::Pane(
                                    main_window,
                                    dashboard::pane::Message::SplitPane(
                                        pane_grid::Axis::Horizontal,
                                        pane_id,
                                    ),
                                ),
                            };
                            btn.on_press(dashboard_msg)
                        } else {
                            btn
                        }
                    };

                    column![
                        text(selected_pane_str),
                        row![
                            tooltip(
                                reset_pane_button,
                                if is_main_window {
                                    Some("Reset selected pane")
                                } else {
                                    None
                                },
                                TooltipPosition::Top,
                            ),
                            tooltip(
                                split_pane_button,
                                if is_main_window {
                                    Some("Split selected pane horizontally")
                                } else {
                                    None
                                },
                                TooltipPosition::Top,
                            ),
                        ]
                        .spacing(8)
                    ]
                    .spacing(8)
                } else {
                    column![text("No pane selected"),].spacing(8)
                };

                let manage_layout_modal = {
                    let col = column![
                        manage_pane,
                        rule::horizontal(1.0).style(style::split_ruler),
                        self.layout_manager.view().map(Message::Layouts)
                    ];

                    container(col.align_x(Alignment::Center).spacing(20))
                        .width(260)
                        .padding(24)
                        .style(style::dashboard_modal)
                };

                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).top(40)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).top(40)),
                };

                dashboard_modal(
                    base,
                    manage_layout_modal,
                    Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::Start,
                    align_x,
                )
            }
            sidebar::Menu::Audio => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).top(76)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).top(76)),
                };

                let trade_streams_list = dashboard.streams.trade_streams(None);

                dashboard_modal(
                    base,
                    self.audio_stream
                        .view(trade_streams_list)
                        .map(Message::AudioStream),
                    Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::Start,
                    align_x,
                )
            }
            sidebar::Menu::ThemeEditor => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                dashboard_modal(
                    base,
                    self.theme_editor
                        .view(&self.theme.0)
                        .map(Message::ThemeEditor),
                    Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                )
            }
            sidebar::Menu::Network => {
                let (align_x, padding) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };

                dashboard_modal(
                    base,
                    self.network.view().map(Message::NetworkManager),
                    Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding,
                    Alignment::End,
                    align_x,
                )
            }
        }
    }

    fn save_state_to_disk(&mut self, windows: &HashMap<window::Id, WindowSpec>) {
        self.active_dashboard_mut()
            .popout
            .iter_mut()
            .for_each(|(id, (_, window_spec))| {
                if let Some(new_window_spec) = windows.get(id) {
                    *window_spec = *new_window_spec;
                }
            });

        self.sidebar.sync_tickers_table_settings();

        let mut ser_layouts = vec![];
        for layout in &self.layout_manager.layouts {
            if let Some(layout) = self.layout_manager.get(layout.id.unique) {
                let serialized_dashboard = data::Dashboard::from(&layout.dashboard);
                ser_layouts.push(data::Layout {
                    name: layout.id.name.clone(),
                    dashboard: serialized_dashboard,
                });
            }
        }

        let layouts = data::Layouts {
            layouts: ser_layouts,
            active_layout: self
                .layout_manager
                .active_layout_id()
                .map(|layout| layout.name.to_string())
                .clone(),
        };

        let main_window_spec = windows
            .iter()
            .find(|(id, _)| **id == self.main_window.id)
            .map(|(_, spec)| *spec);

        let audio_cfg = data::AudioStream::from(&self.audio_stream);

        let proxy_cfg_persisted = self.network.proxy_cfg().map(|mut p| {
            p.auth = None;
            p
        });

        let replay_cfg = data::ReplayConfig {
            mode: match self.replay.mode {
                replay::ReplayMode::Live => "live".into(),
                replay::ReplayMode::Replay => "replay".into(),
            },
            range_start: self.replay.range_input.start.clone(),
            range_end: self.replay.range_input.end.clone(),
        };

        let state = data::State::from_parts(
            layouts,
            self.theme.clone(),
            self.theme_editor.custom_theme.clone().map(data::Theme),
            main_window_spec,
            self.timezone,
            self.sidebar.state.clone(),
            self.ui_scale_factor,
            audio_cfg,
            connector::fetcher::is_trade_fetch_enabled(),
            self.volume_size_unit,
            proxy_cfg_persisted,
            replay_cfg,
        );

        match serde_json::to_string(&state) {
            Ok(layout_str) => {
                let file_name = data::SAVED_STATE_PATH;
                if let Err(e) = data::write_json_to_file(&layout_str, file_name) {
                    log::error!("Failed to write layout state to file: {}", e);
                } else {
                    log::info!("Persisted state to {file_name}");
                }
            }
            Err(e) => log::error!("Failed to serialize layout: {}", e),
        }
    }

    fn restart(&mut self) -> Task<Message> {
        let mut windows_to_close: Vec<window::Id> =
            self.active_dashboard().popout.keys().copied().collect();
        windows_to_close.push(self.main_window.id);

        let close_windows = Task::batch(
            windows_to_close
                .into_iter()
                .map(window::close)
                .collect::<Vec<_>>(),
        );

        let (new_state, init_task) = Flowsurface::new();
        *self = new_state;

        close_windows.chain(init_task)
    }
}
