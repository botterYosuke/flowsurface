pub(crate) mod api;
mod dashboard;
mod handlers;
mod persistence;
mod sidebar_modals;
mod view;

use crate::modal::{ThemeEditor, audio::AudioStream, network_manager::NetworkManager};
use crate::notify::Notifications;
use crate::replay::controller::ReplayController;
use crate::screen::dashboard::Dashboard;
use crate::screen::login::LoginScreen;
use crate::widget::toast::Toast;
use crate::window;
use crate::{Flowsurface, Message};
use iced::Task;

impl Flowsurface {
    pub(crate) fn new() -> (Self, Task<Message>) {
        let saved_state = crate::layout::load_saved_state();

        let dummy_main_id = window::Id::unique();

        let (sidebar, launch_sidebar) = crate::screen::dashboard::Sidebar::new(&saved_state);
        let (audio_stream, audio_init_err) = AudioStream::new(saved_state.audio_cfg);
        let saved_main_window_spec = saved_state.main_window;

        let replay_mode = match saved_state.replay_config.mode.as_str() {
            "replay" => crate::replay::ReplayMode::Replay,
            _ => crate::replay::ReplayMode::Live,
        };
        let is_replay_mode = replay_mode == crate::replay::ReplayMode::Replay;

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
                let range_start = saved_state.replay_config.range_start;
                let range_end = saved_state.replay_config.range_end;
                let has_valid_range =
                    crate::replay::parse_replay_range(&range_start, &range_end).is_ok();
                let pending_auto_play = is_replay_mode && has_valid_range;
                ReplayController::from_saved(replay_mode, range_start, range_end, pending_auto_play)
            },
            virtual_engine: if is_replay_mode {
                Some(crate::replay::virtual_exchange::VirtualExchangeEngine::new(
                    1_000_000.0,
                ))
            } else {
                None
            },
            narrative_store: std::sync::Arc::new(
                crate::narrative::store::NarrativeStore::open_default()
                    .expect("failed to open narrative store"),
            ),
            snapshot_store: crate::narrative::snapshot_store::SnapshotStore::new(data::data_path(
                None,
            )),
            is_headless: std::env::var("CI").is_ok() || std::env::args().any(|a| a == "--headless"),
        };

        if let Some(err) = audio_init_err
            && !state.is_headless
        {
            state
                .notifications
                .push(Toast::error(format!("Audio disabled: {err}")));
        }

        let restore_task = Task::perform(
            crate::connector::auth::try_restore_session(),
            Message::SessionRestoreResult,
        );

        (
            state,
            Task::batch([launch_sidebar.map(Message::Sidebar), restore_task]),
        )
    }

    pub(crate) fn active_dashboard(&self) -> Option<&Dashboard> {
        let active_layout = self.layout_manager.active_layout_id()?;
        self.layout_manager
            .get(active_layout.unique)
            .map(|layout| &layout.dashboard)
    }

    pub(crate) fn active_dashboard_mut(&mut self) -> Option<&mut Dashboard> {
        let active_layout = self.layout_manager.active_layout_id()?;
        self.layout_manager
            .get_mut(active_layout.unique)
            .map(|layout| &mut layout.dashboard)
    }
}
