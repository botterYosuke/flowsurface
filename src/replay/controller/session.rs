use iced::Task;

use crate::screen::dashboard::Dashboard;
use crate::widget::toast::Toast;

use super::super::{
    ReplayLoadEvent, ReplayMessage, ReplaySession, ReplayUserMessage, store::LoadedData,
};
use super::ReplayController;

impl ReplayController {
    /// UI 操作を処理する。非同期タスクを起動する可能性がある（Play 時に kline ロードタスクを発行）。
    pub fn handle_user_message(
        &mut self,
        msg: ReplayUserMessage,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> (Task<ReplayMessage>, Option<Toast>) {
        match msg {
            ReplayUserMessage::ToggleMode => {
                let was_replay = self.state.is_replay();
                self.state.toggle_mode();
                if was_replay && !self.state.is_replay() {
                    // Replay → Live: ペイン content を再構築して WS を自動復帰させる
                    dashboard.rebuild_for_live(main_window_id);
                } else if !was_replay && self.state.is_replay() {
                    // Live → Replay: replay_mode=true で再構築してフェッチループを抑制する
                    dashboard.clear_chart_for_replay(main_window_id);
                }
                (Task::none(), None)
            }

            ReplayUserMessage::StartTimeChanged(s) => {
                self.state.range_input.start = s;
                (Task::none(), None)
            }

            ReplayUserMessage::EndTimeChanged(s) => {
                self.state.range_input.end = s;
                (Task::none(), None)
            }
        }
    }

    /// 非同期ロードイベントを処理する。
    /// KlinesLoadCompleted も DataLoadFailed もタスクを起動しないため、
    /// Task を返す必要がない。これを型で表現する。
    pub fn handle_load_event(
        &mut self,
        event: ReplayLoadEvent,
        dashboard: &mut Dashboard,
        main_window_id: iced::window::Id,
    ) -> Option<Toast> {
        match event {
            ReplayLoadEvent::KlinesLoadCompleted(stream, range, klines) => {
                // 空 klines でも EventStore に登録してストリームをロード済みとマークする。
                // klines が空 = データなし（市場休場・範囲外）であり「ロード未完了」ではない。
                // Idle なら DataLoadFailed 後の遅延 KlinesLoadCompleted → サイレントドロップ。

                // Step 1: ミュータブルボローで内部を更新し、遷移すべきかを bool で返す
                let should_activate = if let ReplaySession::Loading {
                    pending_count,
                    store,
                    ..
                } = &mut self.state.session
                {
                    store.ingest_loaded(
                        stream,
                        range,
                        LoadedData {
                            klines: klines.clone(),
                            trades: vec![],
                        },
                    );
                    *pending_count = pending_count.saturating_sub(1);
                    *pending_count == 0
                } else {
                    // Idle: DataLoadFailed 後の遅延 KlinesLoadCompleted → 無視
                    false
                };

                // Step 2: ボローが解放されてから mem::replace で Loading → Active に遷移
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

                // Start 時刻より前のバーのみを注入する（pre_start_history バー）。
                // Start 以降のバーは dispatch_tick が逐次注入するため、ここで注入すると
                // dedup で無視されてバーが増えなくなる。
                let start_ms = match &self.state.session {
                    ReplaySession::Loading { clock, .. } | ReplaySession::Active { clock, .. } => {
                        clock.full_range().start
                    }
                    ReplaySession::Idle => 0,
                };
                let history_klines = super::super::pre_start_history(&klines, start_ms);
                if !history_klines.is_empty() {
                    dashboard.ingest_replay_klines(&stream, &history_klines, main_window_id);
                }
                None
            }

            ReplayLoadEvent::DataLoadFailed(err) => {
                // session をリセットして残留状態を除去する。
                // これがないと次回 Play 時に古いデータが混入する可能性がある。
                self.reset_session();
                Some(Toast::error(format!("Replay data load failed: {err}")))
            }
        }
    }

    /// session を Idle にリセットする。
    /// `DataLoadFailed` 時に呼ぶことで次回 Play 時に残留状態が混入しないようにする。
    fn reset_session(&mut self) {
        self.state.session = ReplaySession::Idle;
        self.state.resume_pending = false;
    }
}
