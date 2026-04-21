use crate::replay::{self, ReplayMessage, ReplaySystemEvent, ReplayUserMessage};
use crate::screen::dashboard;
use crate::widget::toast::Toast;
use crate::{Flowsurface, Message};
use iced::Task;

impl Flowsurface {
    /// アクティブな kline ストリームを新しい ticker_info に切り替えるリロードタスクを生成する。
    /// リプレイ中でなければ SyncReplayBuffers のみ返す。
    pub(crate) fn make_kline_reload_task(
        &self,
        ticker_info: exchange::TickerInfo,
    ) -> Task<Message> {
        let old_kline_streams: Vec<exchange::adapter::StreamKind> = if self.replay.is_replay() {
            self.replay.active_kline_streams()
        } else {
            return Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::SyncReplayBuffers,
            )));
        };

        let reload_tasks: Vec<Task<Message>> = old_kline_streams
            .into_iter()
            .filter_map(|old| {
                old.as_kline_stream().map(|(_, tf)| {
                    Task::done(Message::Replay(ReplayMessage::System(
                        ReplaySystemEvent::ReloadKlineStream {
                            old_stream: Some(old),
                            new_stream: exchange::adapter::StreamKind::Kline {
                                ticker_info,
                                timeframe: tf,
                            },
                        },
                    )))
                })
            })
            .collect();

        if reload_tasks.is_empty() {
            Task::done(Message::Replay(ReplayMessage::System(
                ReplaySystemEvent::SyncReplayBuffers,
            )))
        } else {
            Task::batch(reload_tasks)
        }
    }

    pub(crate) fn handle_dashboard_message(
        &mut self,
        id: Option<uuid::Uuid>,
        msg: dashboard::Message,
    ) -> Task<Message> {
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

                    let has_any_ticker_info = tickers_info.values().any(|opt| opt.is_some());
                    log::info!(
                        "[e2e-live] ResolveStreams pane={pane_id} streams={} has_ticker_info={has_any_ticker_info}",
                        streams.len()
                    );
                    if !has_any_ticker_info {
                        log::debug!(
                            "Deferring persisted stream resolution for pane {pane_id}: ticker metadata not loaded yet"
                        );
                        return Task::none();
                    }

                    let resolved_streams =
                        streams.into_iter().try_fold(vec![], |mut acc, persist| {
                            let resolver =
                                |t: &exchange::Ticker| tickers_info.get(t).and_then(|opt| *opt);

                            match persist.into_stream_kinds(resolver) {
                                Ok(mut resolved) => {
                                    acc.append(&mut resolved);
                                    Ok(acc)
                                }
                                Err(err) => {
                                    Err(format!("Persisted stream still not resolvable: {err}"))
                                }
                            }
                        });

                    match resolved_streams {
                        Ok(resolved) => {
                            log::info!(
                                "[e2e-live] Streams resolved: {} streams for pane={pane_id}",
                                resolved.len()
                            );
                            if resolved.is_empty() {
                                return Task::none();
                            }

                            // (1) pane を Ready に昇格させる（別スコープで借用解放）
                            let resolve_task = {
                                let Some(dashboard) = self.active_dashboard_mut() else {
                                    return Task::none();
                                };
                                dashboard
                                    .resolve_streams(main_window.id, pane_id, resolved)
                                    .map(move |msg| Message::Dashboard {
                                        layout_id: None,
                                        event: msg,
                                    })
                            };

                            // (2) 全ペイン Ready かつ pending_auto_play が立っているか判定
                            if self.replay.is_auto_play_pending()
                                && self.replay.is_replay()
                                && self
                                    .active_dashboard()
                                    .is_some_and(|d| d.all_panes_have_ready_streams(main_window.id))
                            {
                                log::info!(
                                    "[auto-play] All panes ready — firing ReplayMessage::Play"
                                );
                                self.replay.clear_pending_auto_play();
                                let play_task = Task::done(Message::Replay(ReplayMessage::User(
                                    ReplayUserMessage::Play,
                                )));
                                return Task::batch([resolve_task, play_task]);
                            }

                            resolve_task
                        }
                        Err(err) => {
                            log::info!("[e2e-live] Stream resolution failed: {err}");
                            Task::none()
                        }
                    }
                }
                Some(dashboard::Event::RequestPalette) => {
                    let theme = self.theme.0.clone();
                    let main_window = self.main_window.id;
                    if let Some(d) = self.active_dashboard_mut() {
                        d.theme_updated(main_window, &theme);
                    }
                    Task::none()
                }
                Some(dashboard::Event::ReloadReplayKlines {
                    old_stream,
                    new_stream,
                }) => Task::done(Message::Replay(ReplayMessage::System(
                    ReplaySystemEvent::ReloadKlineStream {
                        old_stream,
                        new_stream,
                    },
                ))),
                Some(dashboard::Event::SwitchTickersInGroup { ticker_info }) => {
                    let is_replay = self.replay.is_replay();
                    let replay_task = self.make_kline_reload_task(ticker_info);

                    let Some(ad) = self.active_dashboard_mut() else {
                        return Task::none();
                    };
                    let switch_task = ad
                        .switch_tickers_in_group(main_window.id, ticker_info, is_replay)
                        .map(move |msg| Message::Dashboard {
                            layout_id: Some(layout_id),
                            event: msg,
                        });

                    // switch_task と replay_task は並列実行（.chain() は認証待ちでブロックするため）。
                    return Task::batch([switch_task, replay_task]);
                }
                Some(dashboard::Event::SubmitVirtualOrder(vo)) => {
                    if let Some(engine) = &mut self.virtual_engine {
                        let order_id = engine.place_order(vo);
                        log::debug!("仮想注文を受け付けました: order_id={order_id:?}");
                    } else {
                        log::warn!("仮想注文が来たが VirtualExchangeEngine が初期化されていません");
                    }
                    Task::none()
                }
                None => Task::none(),
            };

            // mid-replay で stream 構成変更の可能性があれば SyncReplayBuffers を chain する。
            return main_task
                .map(move |msg| Message::Dashboard {
                    layout_id: Some(layout_id),
                    event: msg,
                })
                .chain(additional_task)
                .chain(Task::done(Message::Replay(ReplayMessage::System(
                    ReplaySystemEvent::SyncReplayBuffers,
                ))));
        }
        Task::none()
    }

    pub(crate) fn handle_sidebar(&mut self, message: dashboard::sidebar::Message) -> Task<Message> {
        let is_metadata_update = matches!(
            &message,
            dashboard::sidebar::Message::TickersTable(
                dashboard::tickers_table::Message::UpdateMetadata(..)
            )
        );

        let (task, action) = self.sidebar.update(message);

        match action {
            Some(dashboard::sidebar::Action::TickerSelected(ticker_info, content)) => {
                let main_window_id = self.main_window.id;

                if let Some(kind) = content {
                    let Some(dashboard) = self.active_dashboard_mut() else {
                        return Task::none();
                    };
                    match dashboard.split_focused_and_init(main_window_id, ticker_info, kind) {
                        Some(task) => {
                            let replay_task = Task::done(Message::Replay(ReplayMessage::System(
                                ReplaySystemEvent::SyncReplayBuffers,
                            )));
                            return Task::batch([
                                task.map(move |msg| Message::Dashboard {
                                    layout_id: None,
                                    event: msg,
                                }),
                                replay_task,
                            ]);
                        }
                        None => {
                            self.sidebar.hide_tickers_table();
                            return Task::none();
                        }
                    }
                }

                // content = None: 銘柄のみ切り替え（既存フロー）
                let is_replay = self.replay.is_replay();
                let replay_task = self.make_kline_reload_task(ticker_info);

                let Some(dashboard) = self.active_dashboard_mut() else {
                    return Task::none();
                };
                let task =
                    dashboard.switch_tickers_in_group(main_window_id, ticker_info, is_replay);

                return Task::batch([
                    task.map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    }),
                    replay_task,
                ]);
            }
            Some(dashboard::sidebar::Action::OpenOrderPane(content_kind)) => {
                let main_window_id = self.main_window.id;
                let Some(d) = self.active_dashboard_mut() else {
                    return Task::none();
                };
                return d
                    .split_focused_and_init_order(main_window_id, content_kind)
                    .map(move |msg| Message::Dashboard {
                        layout_id: None,
                        event: msg,
                    });
            }
            Some(dashboard::sidebar::Action::ErrorOccurred(err)) => {
                if !self.is_headless {
                    self.notifications.push(Toast::error(err.to_string()));
                }
            }
            None => {}
        }

        if is_metadata_update {
            let main_window_id = self.main_window.id;
            if let Some(d) = self.active_dashboard_mut() {
                d.refresh_waiting_panes(main_window_id);
            }
            if self.replay.is_auto_play_pending() {
                log::info!(
                    "[auto-play] metadata updated — refreshed waiting panes for stream resolution"
                );
            }
        }

        task.map(Message::Sidebar)
    }

    pub(crate) fn handle_replay(&mut self, msg: ReplayMessage) -> Task<Message> {
        let main_window_id = self.main_window.id;
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
        let was_replay = self.replay.is_replay();
        let time_before = self.replay.current_time_ms();
        let is_step_forward = matches!(
            msg,
            replay::ReplayMessage::User(replay::ReplayUserMessage::StepForward)
        );
        let (task, toast) = self.replay.handle_message(msg, dashboard, main_window_id);
        let time_after = self.replay.current_time_ms();
        if let Some(t) = toast {
            self.notifications.push(t);
        }
        let is_replay_now = self.replay.is_replay();
        if let Some(d) = self.active_dashboard_mut() {
            d.is_replay = is_replay_now;
            d.sync_virtual_mode(main_window_id);
        }

        // 仮想エンジンのライフサイクル管理
        if !was_replay && is_replay_now {
            if self.virtual_engine.is_none() {
                self.virtual_engine = Some(replay::virtual_exchange::VirtualExchangeEngine::new(
                    1_000_000.0,
                ));
                log::info!("VirtualExchangeEngine を初期化しました（初期 cash: 1,000,000）");
            } else if let Some(engine) = &mut self.virtual_engine {
                engine.reset();
                log::info!("VirtualExchangeEngine をリセットしました（再プレイ）");
            }
        } else if was_replay && !is_replay_now {
            if self.virtual_engine.is_some() {
                self.virtual_engine = None;
                log::info!("VirtualExchangeEngine を破棄しました（Live へ切替）");
            }
        } else if is_replay_now
            && time_before != time_after
            && !is_step_forward
            && let Some(engine) = &mut self.virtual_engine
        {
            // StepBackward / range 変更 → 時刻が変化したためリセット
            engine.reset();
            log::info!(
                "VirtualExchangeEngine をリセットしました（seek: {:?} → {:?}）",
                time_before,
                time_after
            );
        }

        // StepForward: kline close から合成 Trade を生成して on_tick() を呼ぶ
        if is_replay_now && is_step_forward {
            let mut fill_tasks = Vec::new();
            if let Some(engine) = &mut self.virtual_engine {
                let clock_ms = self.replay.current_time_ms().unwrap_or(0);
                for (stream, trades) in self.replay.synthetic_trades_at_current_time() {
                    let ticker = stream.ticker_info().ticker.to_string();
                    let fills = engine.on_tick(&ticker, &trades, clock_ms);
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
                        fill_tasks.push(Task::perform(
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

                        let fill_msg = crate::screen::dashboard::Message::VirtualOrderFilled(fill);
                        fill_tasks.push(Task::done(Message::Dashboard {
                            layout_id: None,
                            event: fill_msg,
                        }));
                    }
                }
            }
            if fill_tasks.is_empty() {
                return task.map(Message::Replay);
            }
            fill_tasks.push(task.map(Message::Replay));
            return Task::batch(fill_tasks);
        }

        task.map(Message::Replay)
    }
}
