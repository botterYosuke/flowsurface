use crate::connector;
use crate::layout::{self, configuration};
use crate::replay;
use crate::screen::{self, dashboard};
use crate::window;
use crate::{Flowsurface, Message};
use data::layout::WindowSpec;
use iced::Task;
use std::collections::HashMap;

impl Flowsurface {
    pub(crate) fn save_state_to_disk(&mut self, windows: &HashMap<window::Id, WindowSpec>) {
        if let Some(dashboard) = self.active_dashboard_mut() {
            dashboard
                .popout
                .iter_mut()
                .for_each(|(id, (_, window_spec))| {
                    if let Some(new_window_spec) = windows.get(id) {
                        *window_spec = *new_window_spec;
                    }
                });
        }

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
                .map(|layout| layout.name.to_string()),
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
            mode: match self.replay.mode() {
                replay::ReplayMode::Live => "live".into(),
                replay::ReplayMode::Replay => "replay".into(),
            },
            range_start: self.replay.range_input_start().to_string(),
            range_end: self.replay.range_input_end().to_string(),
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

    pub(crate) fn load_layout(
        &mut self,
        layout_uid: uuid::Uuid,
        main_window: window::Id,
    ) -> Task<Message> {
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

    pub(crate) fn restart(&mut self) -> Task<Message> {
        let mut windows_to_close: Vec<window::Id> = self
            .active_dashboard()
            .map(|d| d.popout.keys().copied().collect())
            .unwrap_or_default();
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

    /// ログインウィンドウを閉じてメインウィンドウ（ダッシュボード）を開く。
    pub(crate) fn transition_to_dashboard(&mut self) -> Task<Message> {
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

        let Some(active_layout_uid) = self.layout_manager.active_layout_id().map(|id| id.unique)
        else {
            log::error!("transition_to_dashboard: layout_manager が空です — 起動を中断します");
            return Task::none();
        };
        let initial_buying_power = self
            .layout_manager
            .get(active_layout_uid)
            .map(|layout| {
                layout
                    .dashboard
                    .initial_buying_power_fetch(main_window_id)
                    .map(|msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    })
            })
            .unwrap_or_else(Task::none);
        let initial_order_list = self
            .layout_manager
            .get(active_layout_uid)
            .map(|layout| {
                layout
                    .dashboard
                    .initial_order_list_fetch(main_window_id)
                    .map(|msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    })
            })
            .unwrap_or_else(Task::none);
        let load_layout = self.load_layout(active_layout_uid, main_window_id);

        let close_login = login_win
            .map(window::close::<Message>)
            .unwrap_or_else(Task::none);

        close_login
            .chain(open_main.discard())
            .chain(iced::window::maximize(main_window_id, true))
            .chain(load_layout)
            .chain(initial_buying_power)
            .chain(initial_order_list)
    }

    /// 銘柄マスタをバックグラウンドでダウンロードし、完了後に TickersTable へ反映する。
    pub(crate) fn start_master_download(
        session: exchange::adapter::tachibana::TachibanaSession,
    ) -> Task<Message> {
        use exchange::adapter::Venue;

        Task::perform(
            async move {
                let cache_path = data::data_path(Some("market_data/tachibana_master_cache.json"));
                let client = reqwest::Client::new();
                const MAX_RETRIES: u32 = 3;
                let mut last_err = None;
                for attempt in 0..MAX_RETRIES {
                    if attempt > 0 {
                        log::info!(
                            "Tachibana master download retry {attempt}/{} (previous: {:?})",
                            MAX_RETRIES - 1,
                            last_err
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    }
                    match exchange::adapter::tachibana::init_issue_master(
                        &client,
                        &session,
                        Some(&cache_path),
                    )
                    .await
                    {
                        Ok(()) => {
                            return Ok(exchange::adapter::tachibana::cached_ticker_metadata().await);
                        }
                        Err(e) => {
                            log::warn!("Tachibana master download attempt {attempt} failed: {e}");
                            last_err = Some(e);
                        }
                    }
                }
                Err(last_err.unwrap_or_else(|| unreachable!("MAX_RETRIES > 0")))
            },
            |result: Result<_, exchange::adapter::tachibana::TachibanaError>| {
                let venue = Venue::Tachibana;
                match result {
                    Ok(metadata) => Message::Sidebar(dashboard::sidebar::Message::TickersTable(
                        dashboard::tickers_table::Message::UpdateMetadata(venue, metadata),
                    )),
                    Err(e) => {
                        log::error!("Tachibana master download failed after retries: {e}");
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

    /// ディスクキャッシュから銘柄マスタを読み込み、TickersTable へ即座に反映する。
    pub(crate) fn make_disk_cache_task() -> Task<Message> {
        let path = data::data_path(Some("market_data/tachibana_master_cache.json"));
        match exchange::adapter::tachibana::load_master_from_disk(&path) {
            Some(metadata) => {
                Task::done(Message::Sidebar(dashboard::sidebar::Message::TickersTable(
                    dashboard::tickers_table::Message::UpdateMetadata(
                        exchange::adapter::Venue::Tachibana,
                        metadata,
                    ),
                )))
            }
            None => Task::none(),
        }
    }

    pub(crate) fn handle_clone_layout(&mut self, id: uuid::Uuid) {
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
            let new_layout = layout::LayoutId {
                unique: new_uid,
                name: manager.ensure_unique_name(&name, new_uid),
            };

            let mut popout_windows = Vec::new();

            for (pane, window_spec) in &ser_dashboard.popout {
                let cfg = configuration(pane.clone());
                popout_windows.push((cfg, *window_spec));
            }

            let dashboard = screen::dashboard::Dashboard::from_config(
                configuration(ser_dashboard.pane.clone()),
                popout_windows,
                old_id,
            );

            manager.insert_layout(new_layout.clone(), dashboard);
        }
    }
}
