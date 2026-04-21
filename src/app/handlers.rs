use crate::connector;
use crate::modal::{self, layout_manager, network_manager};
use crate::screen::{dashboard, login};
use crate::widget::toast::Toast;
use crate::window;
use crate::{Flowsurface, Message};
use iced::Task;

impl Flowsurface {
    pub(crate) fn handle_go_back(&mut self) {
        let main_window = self.main_window.id;

        if self.confirm_dialog.is_some() {
            self.confirm_dialog = None;
        } else if self.sidebar.active_menu().is_some() {
            self.sidebar.set_menu(None);
        } else if let Some(dashboard) = self.active_dashboard_mut()
            && !dashboard.go_back(main_window)
        {
            if dashboard.focus.is_some() {
                dashboard.focus = None;
            } else {
                self.sidebar.hide_tickers_table();
            }
        }
    }

    pub(crate) fn handle_theme_selected(&mut self, theme: iced_core::Theme) {
        self.theme = data::Theme(theme.clone());
        let main_window = self.main_window.id;
        if let Some(d) = self.active_dashboard_mut() {
            d.theme_updated(main_window, &theme);
        }
    }

    pub(crate) fn handle_toggle_trade_fetch(&mut self, checked: bool) {
        self.layout_manager
            .iter_dashboards_mut()
            .for_each(|dashboard| {
                dashboard.toggle_trade_fetch(checked, &self.main_window);
            });

        if checked {
            self.confirm_dialog = None;
        }
    }

    pub(crate) fn handle_layouts(&mut self, message: layout_manager::Message) -> Task<Message> {
        let action = self.layout_manager.update(message);

        match action {
            Some(layout_manager::Action::Select(layout)) => {
                let active_popout_keys = self
                    .active_dashboard()
                    .map(|d| d.popout.keys().copied().collect::<Vec<_>>())
                    .unwrap_or_default();

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
            Some(layout_manager::Action::Clone(id)) => {
                self.handle_clone_layout(id);
            }
            None => {}
        }
        Task::none()
    }

    pub(crate) fn handle_audio_stream(&mut self, message: modal::audio::Message) {
        if let Some(event) = self.audio_stream.update(message)
            && !self.is_headless
        {
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

    pub(crate) fn handle_theme_editor(&mut self, msg: modal::theme_editor::Message) {
        let action = self.theme_editor.update(msg, &self.theme.0);

        match action {
            Some(modal::theme_editor::Action::Exit) => {
                self.sidebar.set_menu(Some(data::sidebar::Menu::Settings));
            }
            Some(modal::theme_editor::Action::UpdateTheme(theme)) => {
                self.theme = data::Theme(theme.clone());
                let main_window = self.main_window.id;
                if let Some(d) = self.active_dashboard_mut() {
                    d.theme_updated(main_window, &theme);
                }
            }
            None => {}
        }
    }

    pub(crate) fn handle_network_manager(
        &mut self,
        msg: network_manager::Message,
    ) -> Task<Message> {
        let action = self.network.update(msg);

        match action {
            Some(network_manager::Action::ApplyProxy) => {
                if let Some(proxy) = self.network.proxy_cfg() {
                    data::config::proxy::save_proxy_auth(&proxy);
                }

                let main_window = self.main_window.id;
                let Some(dashboard) = self.active_dashboard_mut() else {
                    return Task::none();
                };

                let mut active_windows = dashboard
                    .popout
                    .keys()
                    .copied()
                    .collect::<Vec<window::Id>>();
                active_windows.push(main_window);

                return window::collect_window_specs(active_windows, Message::SaveStateRequested);
            }
            Some(network_manager::Action::Exit) => {
                self.sidebar.set_menu(Some(data::sidebar::Menu::Settings));
            }
            None => {}
        }
        Task::none()
    }

    pub(crate) fn handle_market_ws_event(&mut self, event: exchange::Event) -> Task<Message> {
        let main_window_id = self.main_window.id;
        let Some(dashboard) = self.active_dashboard_mut() else {
            return Task::none();
        };

        match event {
            exchange::Event::Connected(exchange) => {
                log::info!("a stream connected to {exchange} WS");
            }
            exchange::Event::Disconnected(exchange, reason) => {
                log::info!("a stream disconnected from {exchange} WS: {reason:?}");
            }
            exchange::Event::DepthReceived(stream, depth_update_t, depth) => {
                return dashboard
                    .ingest_depth(&stream, depth_update_t, &depth, main_window_id)
                    .map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    });
            }
            exchange::Event::TradesReceived(stream, update_t, buffer) => {
                let task = dashboard
                    .ingest_trades(&stream, &buffer, update_t, main_window_id)
                    .map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    });

                if let Some(msg) = self.audio_stream.try_play_sound(&stream, &buffer)
                    && !self.is_headless
                {
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
        Task::none()
    }

    pub(crate) fn handle_tick(&mut self, now: std::time::Instant) -> Task<Message> {
        let main_window_id = self.main_window.id;
        let mut all_tasks: Vec<Task<Message>> = Vec::new();

        if self.replay.is_replay() {
            let Some(active_id) = self.layout_manager.active_layout_id().map(|l| l.unique) else {
                return Task::none();
            };
            let Some(dashboard) = self
                .layout_manager
                .get_mut(active_id)
                .map(|l| &mut l.dashboard)
            else {
                return Task::none();
            };
            let outcome = self.replay.tick(now, dashboard, main_window_id);

            for (stream, trades, update_t) in outcome.trade_events {
                if let Some(engine) = &mut self.virtual_engine {
                    let ticker = stream.ticker_info().ticker.to_string();
                    let clock_ms = self.replay.current_time_ms().unwrap_or(0);
                    let fills = engine.on_tick(&ticker, &trades, clock_ms);
                    if !fills.is_empty() {
                        // Phase 4a D-1: outcome 反映後にマーカー再描画を依頼。
                        all_tasks.push(self.refresh_narrative_markers_task());
                    }
                    for fill in fills {
                        // ナラティブ outcome 自動更新（Phase 4a C-1）
                        let narrative_store = self.narrative_store.clone();
                        let order_id = fill.order_id.clone();
                        let fill_price = fill.fill_price;
                        let fill_time_ms = fill.fill_time_ms as i64;
                        let side_hint = match fill.side {
                            crate::replay::virtual_exchange::PositionSide::Long => {
                                Some(crate::narrative::model::NarrativeSide::Buy)
                            }
                            crate::replay::virtual_exchange::PositionSide::Short => {
                                Some(crate::narrative::model::NarrativeSide::Sell)
                            }
                        };
                        all_tasks.push(Task::perform(
                            async move {
                                if let Err(e) =
                                    crate::narrative::service::update_outcome_from_fill(
                                        &narrative_store,
                                        &order_id,
                                        fill_price,
                                        fill_time_ms,
                                        side_hint,
                                    )
                                    .await
                                {
                                    log::warn!(
                                        "failed to update narrative outcome for order {order_id}: {e}"
                                    );
                                }
                            },
                            |()| Message::Noop,
                        ));

                        let fill_msg = dashboard::Message::VirtualOrderFilled(fill);
                        all_tasks.push(Task::done(Message::Dashboard {
                            layout_id: None,
                            event: fill_msg,
                        }));
                    }
                }

                if let Some(d) = self.active_dashboard_mut() {
                    let task = d
                        .ingest_trades(&stream, &trades, update_t, main_window_id)
                        .map(move |msg| Message::Dashboard {
                            layout_id: None,
                            event: msg,
                        });
                    all_tasks.push(task);
                }
            }

            if outcome.reached_end {
                self.notifications.push(Toast::info("Replay reached end"));
            }
        }

        if let Some(d) = self.active_dashboard_mut() {
            let tick_task = d
                .tick(now, main_window_id)
                .map(move |msg| Message::Dashboard {
                    layout_id: None,
                    event: msg,
                });
            all_tasks.push(tick_task);
        }

        Task::batch(all_tasks)
    }

    pub(crate) fn handle_window_event(&mut self, event: window::Event) -> Task<Message> {
        match event {
            window::Event::CloseRequested(window) => {
                if Some(window) == self.login_window {
                    return iced::exit();
                }

                let main_window = self.main_window.id;
                let Some(dashboard) = self.active_dashboard_mut() else {
                    return Task::none();
                };

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

                window::collect_window_specs(active_windows, Message::ExitRequested)
            }
        }
    }

    pub(crate) fn handle_login(&mut self, msg: login::Message) -> Task<Message> {
        match msg {
            login::Message::LoginSubmit => {
                self.login_screen.set_error(None);
                let user_id = self.login_screen.user_id.clone();
                let password = self.login_screen.password.clone();
                let is_demo = self.login_screen.is_demo;
                Task::perform(
                    connector::auth::perform_login(user_id, password, is_demo),
                    Message::LoginCompleted,
                )
            }
            other => {
                self.login_screen.update(other);
                Task::none()
            }
        }
    }

    pub(crate) fn handle_login_completed(
        &mut self,
        result: Result<exchange::adapter::tachibana::TachibanaSession, String>,
    ) -> Task<Message> {
        match result {
            Ok(session) => {
                connector::auth::store_session(session.clone());
                connector::auth::persist_session(&session);
                let dashboard_task = self.transition_to_dashboard();
                let master_task = Self::start_master_download(session);
                let disk_cache_task = Self::make_disk_cache_task();
                Task::batch([dashboard_task, disk_cache_task, master_task])
            }
            Err(error_msg) => {
                log::warn!("Login failed: {error_msg}");
                self.login_screen.set_error(Some(error_msg));
                Task::none()
            }
        }
    }

    pub(crate) fn handle_session_restore_result(
        &mut self,
        result: Option<exchange::adapter::tachibana::TachibanaSession>,
    ) -> Task<Message> {
        if let Some(session) = result {
            connector::auth::store_session(session.clone());
            let dashboard_task = self.transition_to_dashboard();
            let master_task = Self::start_master_download(session);
            let disk_cache_task = Self::make_disk_cache_task();
            return Task::batch([dashboard_task, disk_cache_task, master_task]);
        }
        let main_window_id = self.main_window.id;
        if self.replay.is_auto_play_pending()
            && self
                .active_dashboard()
                .is_some_and(|d| d.has_tachibana_stream_pane(main_window_id))
        {
            self.replay.on_session_unavailable();
            log::info!(
                "[auto-play] session unavailable — auto-play deferred (Tachibana login required)"
            );
            self.notifications.push(Toast::info(
                "Replay auto-play was deferred: please log in to resume",
            ));
        }
        let (login_window_id, open_login_window) = window::open(window::Settings {
            size: iced::Size::new(900.0, 560.0),
            position: window::Position::Centered,
            resizable: false,
            exit_on_close_request: true,
            ..Default::default()
        });
        self.login_window = Some(login_window_id);
        #[cfg(debug_assertions)]
        {
            if !self.login_screen.user_id.is_empty() {
                return Task::batch([
                    open_login_window.discard(),
                    Task::done(Message::Login(login::Message::LoginSubmit)),
                ]);
            }
        }
        #[cfg(not(debug_assertions))]
        return open_login_window.discard();
        #[cfg(debug_assertions)]
        open_login_window.discard()
    }
}
