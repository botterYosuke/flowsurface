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
        // ADR-0001 §2: 自動再生機構の全廃に伴い、ここでの `self.replay.tick(...)` による
        // wall-clock 駆動 replay 前進処理は削除。Replay の進行は
        // `/api/agent/session/:id/{step,advance,rewind-to-start}` からのみ発火する。
        // このハンドラは iced::window::frames() による dashboard 描画アニメーション tick
        // の配送のみを担う。
        let main_window_id = self.main_window.id;

        let Some(d) = self.active_dashboard_mut() else {
            return Task::none();
        };
        d.tick(now, main_window_id)
            .map(move |msg| Message::Dashboard {
                layout_id: None,
                event: msg,
            })
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
        // ADR-0001 §8: 起動時 fixture 自動 Play は廃止済みなので、ここでの
        // auto-play deferred ハンドリング (Tachibana ログイン要求時の Toast 通知) も削除。
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

    pub(crate) fn handle_agent(&mut self, msg: crate::replay::AgentMessage) -> Task<Message> {
        let main_window_id = self.main_window.id;
        let layout_id = match self.layout_manager.active_layout_id() {
            Some(l) => l.unique,
            None => return Task::none(),
        };

        let dashboard = match self.layout_manager.get_mut(layout_id) {
            Some(l) => &mut l.dashboard,
            None => return Task::none(),
        };

        let mut virtual_fills = Vec::new();
        let mut all_tasks = Vec::new();
        let mut needs_marker_refresh = false;

        match msg {
            crate::replay::AgentMessage::Step => {
                if let Some((current_time, trade_events)) =
                    self.replay.agent_step(dashboard, main_window_id)
                {
                    if let Some(ve) = &mut self.virtual_engine {
                        for (stream, trades) in trade_events {
                            let ticker_str = stream.ticker_info().ticker.to_string();
                            let fills = ve.on_tick(&ticker_str, &trades, current_time);
                            if !fills.is_empty() {
                                needs_marker_refresh = true;
                            }
                            virtual_fills.extend(fills);
                        }
                    }
                } else {
                    log::warn!("agent_step called but session is not Active.");
                }
            }
            crate::replay::AgentMessage::Advance => {
                if let Some((current_time, trade_events)) = self.replay.agent_advance(
                    dashboard,
                    main_window_id,
                    crate::replay::controller::ReplayController::UI_ADVANCE_CAP_MS,
                ) {
                    if let Some(ve) = &mut self.virtual_engine {
                        for (stream, trades) in trade_events {
                            let ticker_str = stream.ticker_info().ticker.to_string();
                            let fills = ve.on_tick(&ticker_str, &trades, current_time);
                            if !fills.is_empty() {
                                needs_marker_refresh = true;
                            }
                            virtual_fills.extend(fills);
                        }
                    }
                } else {
                    log::warn!("agent_advance called but session is not Active.");
                }
            }
            crate::replay::AgentMessage::RewindToStart => {
                self.replay.agent_rewind(dashboard, main_window_id);
                // ADR-0001 §4 Reset 不変条件の部分実装:
                // ここでは `VirtualExchange` のローカル reset（open orders キャンセル・
                // fills 履歴破棄・仮想残高リセット）のみを呼ぶ。
                // 未実装: SessionLifecycleEvent::Reset 発火、client_order_id UNIQUE map
                // クリア、NarrativeState::Reset、UI チャートの「新 session 扱い」再描画。
                // 未実装項目は専用サブフェーズで対応予定（計画書参照）。
                if let Some(ve) = &mut self.virtual_engine {
                    ve.reset();
                }
            }
        }

        if needs_marker_refresh {
            all_tasks.push(self.refresh_narrative_markers_task());
        }

        for fill in virtual_fills {
            let narrative_store = self.narrative_store.clone();
            let order_id = fill.order_id.clone();
            let fill_price = fill.fill_price;
            let fill_time_ms =
                crate::api::contract::EpochMs::new(fill.fill_time_ms).saturating_to_i64();
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
                    if let Err(e) = crate::narrative::service::update_outcome_from_fill(
                        &narrative_store,
                        &order_id,
                        fill_price,
                        fill_time_ms,
                        side_hint,
                    )
                    .await
                    {
                        log::warn!("failed to update narrative outcome for order {order_id}: {e}");
                    }
                },
                |()| Message::Noop,
            ));

            all_tasks.push(Task::done(Message::Dashboard {
                layout_id: None,
                event: crate::screen::dashboard::Message::VirtualOrderFilled(fill),
            }));
        }

        Task::batch(all_tasks)
    }
}
