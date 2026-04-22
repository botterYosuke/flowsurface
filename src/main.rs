#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod audio;
mod chart;
mod connector;
mod headless;
mod layout;
mod logger;
mod modal;
mod narrative;
mod notify;
mod replay;
mod replay_api;
mod screen;
mod style;
mod version;
mod widget;
mod window;

pub(crate) use iced::widget::tooltip::Position as TooltipPosition;
pub(crate) use widget::tooltip;

use data::{layout::WindowSpec, sidebar};
use modal::{LayoutManager, ThemeEditor, audio::AudioStream, network_manager::NetworkManager};
use notify::Notifications;
use replay::{ReplayMessage, ReplayUserMessage, controller::ReplayController};
use screen::dashboard::{self};
use screen::login::{self, LoginScreen};
use widget::toast::{self, Toast};

use iced::{
    Alignment, Element, Subscription, Task, keyboard, padding,
    widget::{column, container, row},
};
use std::{borrow::Cow, collections::HashMap};

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
    replay: ReplayController,
    virtual_engine: Option<replay::virtual_exchange::VirtualExchangeEngine>,
    narrative_store: std::sync::Arc<narrative::store::NarrativeStore>,
    snapshot_store: narrative::snapshot_store::SnapshotStore,
    is_headless: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Login(login::Message),
    Sidebar(dashboard::sidebar::Message),
    MarketWsEvent(exchange::Event),
    Dashboard {
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
    BuyingPowerApiResult {
        reply: replay_api::ReplySender,
        result: Result<
            (
                exchange::adapter::tachibana::BuyingPowerResponse,
                exchange::adapter::tachibana::MarginPowerResponse,
            ),
            String,
        >,
    },
    TachibanaOrderApiResult {
        reply: replay_api::ReplySender,
        result: Result<exchange::adapter::tachibana::NewOrderResponse, String>,
    },
    FetchOrdersApiResult {
        reply: replay_api::ReplySender,
        result: Result<Vec<exchange::adapter::tachibana::OrderRecord>, String>,
    },
    FetchOrderDetailApiResult {
        reply: replay_api::ReplySender,
        result: Result<Vec<exchange::adapter::tachibana::ExecutionRecord>, String>,
    },
    ModifyOrderApiResult {
        reply: replay_api::ReplySender,
        result: Result<exchange::adapter::tachibana::ModifyOrderResponse, String>,
    },
    FetchHoldingsApiResult {
        reply: replay_api::ReplySender,
        result: Result<u64, String>,
    },
    NarrativeApiReply {
        reply: replay_api::ReplySender,
        status: u16,
        body: String,
    },
    /// アクティブダッシュボード上の全 KlineChart に最新ナラティブマーカーを配信する。
    /// POST /api/agent/narrative 成功時と FillEvent 発火時にトリガーされる。
    SetNarrativeMarkers(Vec<narrative::marker::NarrativeMarker>),
    Replay(ReplayMessage),
    ReplayApi(replay_api::ApiMessage),
    /// 完了は気にしないファイア・アンド・フォーゲットの async 結果用。
    /// 例: ナラティブ outcome の非同期更新（失敗時はログで充分）。
    Noop,
}

impl Flowsurface {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Login(msg) => return self.handle_login(msg),
            Message::LoginCompleted(result) => return self.handle_login_completed(result),
            Message::SessionRestoreResult(result) => {
                return self.handle_session_restore_result(result);
            }
            Message::BuyingPowerApiResult { reply, result } => {
                self.handle_api_buying_power(reply, result);
            }
            Message::TachibanaOrderApiResult { reply, result } => {
                self.handle_api_tachibana_order(reply, result);
            }
            Message::FetchOrdersApiResult { reply, result } => {
                self.handle_api_fetch_orders(reply, result);
            }
            Message::FetchOrderDetailApiResult { reply, result } => {
                self.handle_api_fetch_order_detail(reply, result);
            }
            Message::ModifyOrderApiResult { reply, result } => {
                self.handle_api_modify_order(reply, result);
            }
            Message::FetchHoldingsApiResult { reply, result } => {
                self.handle_api_fetch_holdings(reply, result);
            }
            Message::NarrativeApiReply {
                reply,
                status,
                body,
            } => {
                return self.handle_narrative_api_reply(reply, status, body);
            }
            Message::SetNarrativeMarkers(markers) => {
                self.handle_set_narrative_markers(markers);
            }
            Message::Noop => {}
            Message::MarketWsEvent(event) => return self.handle_market_ws_event(event),
            Message::Tick(now) => return self.handle_tick(now),
            Message::WindowEvent(event) => return self.handle_window_event(event),
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
            Message::GoBack => self.handle_go_back(),
            Message::ThemeSelected(theme) => self.handle_theme_selected(theme),
            Message::Dashboard {
                layout_id: id,
                event: msg,
            } => return self.handle_dashboard_message(id, msg),
            Message::RemoveNotification(index) => {
                self.notifications.remove(index);
            }
            Message::SetTimezone(tz) => {
                self.timezone = tz;
            }
            Message::ScaleFactorChanged(value) => {
                self.ui_scale_factor = value;
            }
            Message::ToggleTradeFetch(checked) => self.handle_toggle_trade_fetch(checked),
            Message::ToggleDialogModal(dialog) => {
                self.confirm_dialog = dialog;
            }
            Message::Layouts(message) => return self.handle_layouts(message),
            Message::AudioStream(message) => self.handle_audio_stream(message),
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
            Message::ThemeEditor(msg) => self.handle_theme_editor(msg),
            Message::NetworkManager(msg) => return self.handle_network_manager(msg),
            Message::Sidebar(message) => return self.handle_sidebar(message),
            Message::ApplyVolumeSizeUnit(pref) => {
                self.volume_size_unit = pref;
                self.confirm_dialog = None;

                let mut active_windows: Vec<window::Id> = self
                    .active_dashboard()
                    .map(|d| d.popout.keys().copied().collect())
                    .unwrap_or_default();
                active_windows.push(self.main_window.id);

                return window::collect_window_specs(active_windows, Message::RestartRequested);
            }
            Message::Replay(msg) => return self.handle_replay(msg),
            Message::ReplayApi((command, reply_tx)) => {
                return self.handle_replay_api(command, reply_tx);
            }
        }
        Task::none()
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        if Some(id) == self.login_window {
            return self.login_screen.view().map(Message::Login);
        }

        let Some(dashboard) = self.active_dashboard() else {
            return iced::widget::text("").into();
        };
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
                    &self.theme.0,
                )
                .map(move |msg| Message::Dashboard {
                    layout_id: None,
                    event: msg,
                });

            let header_title = {
                #[cfg(target_os = "macos")]
                {
                    iced::widget::center(
                        iced::widget::text("FLOWSURFACE")
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
                    .view_window(
                        id,
                        &self.main_window,
                        tickers_table,
                        self.timezone,
                        &self.theme.0,
                    )
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
                keyboard::Key::Named(keyboard::key::Named::F5) => Some(Message::Replay(
                    ReplayMessage::User(ReplayUserMessage::ToggleMode),
                )),
                _ => None,
            }
        });

        if self.replay.is_replay() {
            // headless 環境（CI=true）では window::frames() が発火しないためタイマーを使用。
            let replay_tick = if self.is_headless {
                iced::time::every(std::time::Duration::from_millis(100)).map(Message::Tick)
            } else {
                tick
            };
            return Subscription::batch(vec![
                window_events,
                sidebar,
                replay_tick,
                hotkeys,
                replay_api,
            ]);
        }

        let exchange_streams = self
            .active_dashboard()
            .map(|d| d.market_subscriptions().map(Message::MarketWsEvent))
            .unwrap_or_else(Subscription::none);

        Subscription::batch(vec![
            exchange_streams,
            sidebar,
            window_events,
            tick,
            hotkeys,
            replay_api,
        ])
    }
}

fn main() {
    #[cfg(debug_assertions)]
    dotenvy::dotenv().ok();

    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--headless".to_string()) {
        logger::setup(true).expect("Failed to initialize logger");
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(headless::run(&args));
        return;
    }

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
