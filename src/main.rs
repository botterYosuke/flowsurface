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
                    playback: None,
                    last_tick: None,
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

                // リプレイ再生中の場合はフレームごとに時間を進めて Trades を注入
                let elapsed_ms = self
                    .replay
                    .last_tick
                    .map(|prev| now.duration_since(prev).as_secs_f64() * 1000.0)
                    .unwrap_or(16.0);
                self.replay.last_tick = Some(now);

                let replay_trades = if let Some(pb) = &mut self.replay.playback {
                    if pb.status == replay::PlaybackStatus::Playing {
                        let current_time = pb.advance_time(elapsed_ms);

                        // 各ストリームの TradeBuffer から current_time 以前を収集
                        let streams: Vec<_> = pb.trade_buffers.keys().copied().collect();
                        let mut collected = Vec::new();

                        for stream in streams {
                            if let Some(buffer) = pb.trade_buffers.get_mut(&stream) {
                                let drained = buffer.drain_until(current_time);
                                if !drained.is_empty() {
                                    let update_t = drained.last().map_or(current_time, |t| t.time);
                                    collected.push((stream, drained.to_vec(), update_t));
                                }
                            }
                        }

                        // current_time >= end_time なら自動停止
                        if pb.current_time >= pb.end_time {
                            pb.status = replay::PlaybackStatus::Paused;
                        }

                        Some(collected)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let mut all_tasks: Vec<Task<Message>> = Vec::new();

                if let Some(collected) = replay_trades {
                    for (stream, trades, update_t) in &collected {
                        let task = self
                            .active_dashboard_mut()
                            .ingest_trades(stream, trades, *update_t, main_window_id)
                            .map(move |msg| Message::Dashboard {
                                layout_id: None,
                                event: msg,
                            });
                        all_tasks.push(task);
                    }
                }

                // リプレイ中も tick() を呼んでチャートのアニメーション更新を維持する
                let tick_task = self
                    .active_dashboard_mut()
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
                                    log::debug!("{err}");
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

                    return main_task
                        .map(move |msg| Message::Dashboard {
                            layout_id: Some(layout_id),
                            event: msg,
                        })
                        .chain(additional_task);
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

                        return task.map(move |msg| Message::Dashboard {
                            layout_id: None,
                            event: msg,
                        });
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
                        if was_replay && !self.replay.is_replay() {
                            let main_window_id = self.main_window.id;
                            let dashboard = self.active_dashboard_mut();
                            dashboard.prepare_replay(main_window_id);
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

                        // PlaybackState を初期化（Loading）
                        self.replay.playback = Some(replay::PlaybackState {
                            start_time: start_ms,
                            end_time: end_ms,
                            current_time: start_ms,
                            status: replay::PlaybackStatus::Loading,
                            speed: 1.0,
                            trade_buffers: std::collections::HashMap::new(),
                        });

                        let main_window_id = self.main_window.id;
                        let layout_id = self
                            .layout_manager
                            .active_layout_id()
                            .expect("No active layout")
                            .unique;

                        // ペインの content をクリアし、kline ストリームを収集
                        let dashboard = self.active_dashboard_mut();
                        let kline_targets = dashboard.prepare_replay(main_window_id);

                        // trades ストリームも収集（Binance のみ）
                        let trade_targets: Vec<_> = dashboard
                            .collect_trade_streams(main_window_id)
                            .into_iter()
                            .filter(|stream| {
                                let exchange = stream.ticker_info().exchange();
                                matches!(exchange.venue(), exchange::adapter::Venue::Binance)
                            })
                            .collect();

                        // 各 kline ストリームに対して fetch_klines を発行
                        let mut all_tasks: Vec<Task<Message>> = kline_targets
                            .into_iter()
                            .map(|(pane_id, stream)| {
                                let req_id = uuid::Uuid::new_v4();
                                connector::fetcher::kline_fetch_task(
                                    layout_id,
                                    pane_id,
                                    stream,
                                    Some(req_id),
                                    Some((start_ms, end_ms)),
                                )
                                .map(move |update| {
                                    Message::Dashboard {
                                        layout_id: Some(layout_id),
                                        event: update.into(),
                                    }
                                })
                            })
                            .collect();

                        // Binance trades のフェッチ
                        for stream in &trade_targets {
                            let ticker_info = stream.ticker_info();
                            let stream_kind = *stream;
                            let data_path = data::data_path(Some("market_data/binance/"));

                            let (task, _handle) = Task::sip(
                                connector::fetcher::fetch_trades_batched(
                                    ticker_info,
                                    start_ms,
                                    end_ms,
                                    data_path,
                                ),
                                move |batch| {
                                    Message::Replay(ReplayMessage::TradesBatchReceived(
                                        stream_kind,
                                        batch,
                                    ))
                                },
                                move |result| match result {
                                    Ok(()) => Message::Replay(ReplayMessage::TradesFetchCompleted(
                                        stream_kind,
                                    )),
                                    Err(err) => Message::Replay(ReplayMessage::DataLoadFailed(
                                        err.ui_message(),
                                    )),
                                },
                            )
                            .abortable();
                            all_tasks.push(task);
                        }

                        if all_tasks.is_empty() {
                            if let Some(pb) = &mut self.replay.playback {
                                pb.status = replay::PlaybackStatus::Playing;
                            }
                        } else {
                            // 全 fetch が完了したら DataLoaded を発行
                            let data_loaded =
                                Task::done(Message::Replay(ReplayMessage::DataLoaded));
                            return Task::batch(all_tasks).chain(data_loaded);
                        }
                    }
                    ReplayMessage::Pause => {
                        if let Some(pb) = &mut self.replay.playback {
                            pb.status = replay::PlaybackStatus::Paused;
                        }
                    }
                    ReplayMessage::StepForward => {
                        // 1分早送り: current_time を 60秒先にジャンプし、その区間の Trades を一括注入
                        let collected = if let Some(pb) = &mut self.replay.playback {
                            let step_ms = 60_000; // 1分
                            let new_time = (pb.current_time + step_ms).min(pb.end_time);
                            pb.current_time = new_time;

                            let streams: Vec<_> = pb.trade_buffers.keys().copied().collect();
                            let mut result = Vec::new();

                            for stream in streams {
                                if let Some(buffer) = pb.trade_buffers.get_mut(&stream) {
                                    let drained = buffer.drain_until(new_time);
                                    if !drained.is_empty() {
                                        let update_t = drained.last().map_or(new_time, |t| t.time);
                                        result.push((stream, drained.to_vec(), update_t));
                                    }
                                }
                            }

                            if pb.current_time >= pb.end_time {
                                pb.status = replay::PlaybackStatus::Paused;
                            }

                            result
                        } else {
                            Vec::new()
                        };

                        let main_window_id = self.main_window.id;
                        let mut tasks = Vec::new();
                        for (stream, trades, update_t) in &collected {
                            let task = self
                                .active_dashboard_mut()
                                .ingest_trades(stream, trades, *update_t, main_window_id)
                                .map(move |msg| Message::Dashboard {
                                    layout_id: None,
                                    event: msg,
                                });
                            tasks.push(task);
                        }
                        if !tasks.is_empty() {
                            return Task::batch(tasks);
                        }
                    }
                    ReplayMessage::CycleSpeed => {
                        if let Some(pb) = &mut self.replay.playback {
                            pb.cycle_speed();
                        }
                    }
                    ReplayMessage::StepBackward => {
                        // 巻き戻し: チャートリセット → Kline 再挿入 → start からの Trades 再注入
                        // current_time を 1 分前に戻す（最小は start_time）
                        if let Some(pb) = &mut self.replay.playback {
                            let step_ms = 60_000u64;
                            let new_time =
                                pb.current_time.saturating_sub(step_ms).max(pb.start_time);
                            pb.current_time = new_time;

                            // TradeBuffer のカーソルをリセットし、new_time まで早送り
                            for buffer in pb.trade_buffers.values_mut() {
                                buffer.cursor = 0;
                                buffer.drain_until(new_time);
                            }
                        }

                        // ペインの content をリビルドして Kline を再挿入
                        let main_window_id = self.main_window.id;
                        let layout_id = self
                            .layout_manager
                            .active_layout_id()
                            .expect("No active layout")
                            .unique;
                        let dashboard = self.active_dashboard_mut();
                        let kline_targets = dashboard.prepare_replay(main_window_id);

                        let start_ms = self.replay.playback.as_ref().map_or(0, |pb| pb.start_time);
                        let end_ms = self.replay.playback.as_ref().map_or(0, |pb| pb.end_time);

                        let fetch_tasks: Vec<_> = kline_targets
                            .into_iter()
                            .map(|(pane_id, stream)| {
                                let req_id = uuid::Uuid::new_v4();
                                connector::fetcher::kline_fetch_task(
                                    layout_id,
                                    pane_id,
                                    stream,
                                    Some(req_id),
                                    Some((start_ms, end_ms)),
                                )
                                .map(move |update| {
                                    Message::Dashboard {
                                        layout_id: Some(layout_id),
                                        event: update.into(),
                                    }
                                })
                            })
                            .collect();

                        if !fetch_tasks.is_empty() {
                            return Task::batch(fetch_tasks);
                        }
                    }
                    ReplayMessage::TradesBatchReceived(stream, batch) => {
                        if let Some(pb) = &mut self.replay.playback {
                            let buffer = pb.trade_buffers.entry(stream).or_insert_with(|| {
                                replay::TradeBuffer {
                                    trades: Vec::new(),
                                    cursor: 0,
                                }
                            });
                            buffer.trades.extend(batch);
                        }
                    }
                    ReplayMessage::TradesFetchCompleted(_stream) => {
                        // trades フェッチ完了（個別ストリーム）。
                        // DataLoaded は全タスク完了後に .chain() で発行される。
                    }
                    ReplayMessage::DataLoaded => {
                        if let Some(pb) = &mut self.replay.playback {
                            pb.status = replay::PlaybackStatus::Playing;
                        }
                    }
                    ReplayMessage::DataLoadFailed(err) => {
                        self.notifications
                            .push(Toast::error(format!("Replay data load failed: {err}")));
                        // リプレイモードをリセット
                        self.replay.playback = None;
                    }
                }
            }
            Message::ReplayApi((command, reply_tx)) => {
                use replay::ReplayCommand;

                match command {
                    ReplayCommand::GetStatus => {
                        reply_tx.send(self.replay.to_status());
                    }
                    ReplayCommand::Toggle => {
                        let task = self.update(Message::Replay(ReplayMessage::ToggleMode));
                        reply_tx.send(self.replay.to_status());
                        return task;
                    }
                    ReplayCommand::Play { start, end } => {
                        self.replay.range_input.start = start;
                        self.replay.range_input.end = end;
                        let task = self.update(Message::Replay(ReplayMessage::Play));
                        reply_tx.send(self.replay.to_status());
                        return task;
                    }
                    ReplayCommand::Pause => {
                        let task = self.update(Message::Replay(ReplayMessage::Pause));
                        reply_tx.send(self.replay.to_status());
                        return task;
                    }
                    ReplayCommand::Resume => {
                        if let Some(pb) = &mut self.replay.playback {
                            pb.status = replay::PlaybackStatus::Playing;
                        }
                        reply_tx.send(self.replay.to_status());
                    }
                    ReplayCommand::StepForward => {
                        let task = self.update(Message::Replay(ReplayMessage::StepForward));
                        reply_tx.send(self.replay.to_status());
                        return task;
                    }
                    ReplayCommand::StepBackward => {
                        let task = self.update(Message::Replay(ReplayMessage::StepBackward));
                        reply_tx.send(self.replay.to_status());
                        return task;
                    }
                    ReplayCommand::CycleSpeed => {
                        let task = self.update(Message::Replay(ReplayMessage::CycleSpeed));
                        reply_tx.send(self.replay.to_status());
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

        let is_playing = self
            .replay
            .playback
            .as_ref()
            .is_some_and(|pb| pb.status == replay::PlaybackStatus::Playing);

        let play_pause_label = if is_playing { "\u{23F8}" } else { "\u{25B6}" };
        let mut play_pause_btn =
            button(text(play_pause_label).size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay {
            play_pause_btn = play_pause_btn.on_press(if is_playing {
                Message::Replay(ReplayMessage::Pause)
            } else {
                Message::Replay(ReplayMessage::Play)
            });
        }

        // ⏮ StepBackward
        let mut step_back_btn =
            button(text("\u{23EE}").size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay && self.replay.playback.is_some() {
            step_back_btn = step_back_btn.on_press(Message::Replay(ReplayMessage::StepBackward));
        }

        // ⏭ StepForward
        let mut step_fwd_btn =
            button(text("\u{23ED}").size(12)).padding(padding::all(2).left(4).right(4));
        if is_replay {
            step_fwd_btn = step_fwd_btn.on_press(Message::Replay(ReplayMessage::StepForward));
        }

        // Speed button
        let speed_label = self
            .replay
            .playback
            .as_ref()
            .map_or("1x".to_string(), |pb| pb.speed_label());
        let mut speed_btn =
            button(text(speed_label).size(11)).padding(padding::all(2).left(4).right(4));
        if is_replay && self.replay.playback.is_some() {
            speed_btn = speed_btn.on_press(Message::Replay(ReplayMessage::CycleSpeed));
        }

        let is_loading = self
            .replay
            .playback
            .as_ref()
            .is_some_and(|pb| pb.status == replay::PlaybackStatus::Loading);

        let controls = row![step_back_btn, play_pause_btn, step_fwd_btn, speed_btn].spacing(4);

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
            .map(|id| window::close::<Message>(id))
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
