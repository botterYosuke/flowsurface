//! GUI モードのナラティブ API ハンドラー。
//!
//! `Flowsurface` の `narrative_store` / `snapshot_store` を使い、
//! `narrative::service` にロジックを委譲する。各エンドポイントは
//! `Task::perform` で async 実行し、完了時に `Message::NarrativeApiReply`
//! 経由で `ReplySender::send_status` を呼び出す。

use iced::Task;

use crate::narrative::service;
use crate::replay_api::{self, NarrativeCommand, ReplySender};
use crate::{Flowsurface, Message};

impl Flowsurface {
    pub(crate) fn handle_narrative_api(
        &mut self,
        cmd: NarrativeCommand,
        reply: ReplySender,
    ) -> Task<Message> {
        let store = self.narrative_store.clone();
        let snapshot_store = self.snapshot_store.clone();
        let data_root = data::data_path(None);
        let now_ms = self.replay.current_time_ms().map(|t| t as i64).unwrap_or(0);
        let created_at_ms = chrono::Utc::now().timestamp_millis();

        match cmd {
            NarrativeCommand::Create(req) => Task::perform(
                async move {
                    service::create_narrative(&store, &snapshot_store, *req, now_ms, created_at_ms)
                        .await
                },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::List(q) => Task::perform(
                async move { service::list_narratives(&store, q).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::Get { id } => Task::perform(
                async move { service::get_narrative(&store, id).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::GetSnapshot { id } => Task::perform(
                async move { service::get_narrative_snapshot(&store, &snapshot_store, id).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::Patch { id, public } => Task::perform(
                async move { service::patch_narrative(&store, id, public).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::StorageStats => Task::perform(
                async move { service::storage_stats(&store).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
            NarrativeCommand::Orphans => Task::perform(
                async move { service::orphans(&store, data_root).await },
                move |(status, body)| Message::NarrativeApiReply {
                    reply: reply.clone(),
                    status,
                    body,
                },
            ),
        }
    }

    pub(crate) fn handle_narrative_api_reply(
        &self,
        reply: replay_api::ReplySender,
        status: u16,
        body: String,
    ) -> Task<Message> {
        reply.send_status(status, body);
        // 作成成功時はチャートマーカーを refresh する。
        if status == 201 {
            self.refresh_narrative_markers_task()
        } else {
            Task::none()
        }
    }

    /// ナラティブストアから最新一覧を取得し、マーカーに変換して `SetNarrativeMarkers`
    /// メッセージを発行する Task を返す。
    pub(crate) fn refresh_narrative_markers_task(&self) -> Task<Message> {
        use crate::narrative::marker::NarrativeMarker;
        use crate::narrative::store::ListFilter;
        let store = self.narrative_store.clone();
        Task::perform(
            async move {
                match store
                    .list(ListFilter {
                        limit: Some(1000),
                        ..Default::default()
                    })
                    .await
                {
                    Ok(narratives) => narratives
                        .iter()
                        .flat_map(NarrativeMarker::from_narrative)
                        .collect::<Vec<_>>(),
                    Err(e) => {
                        log::warn!("narrative marker refresh failed: {e}");
                        Vec::new()
                    }
                }
            },
            Message::SetNarrativeMarkers,
        )
    }

    pub(crate) fn handle_set_narrative_markers(
        &mut self,
        markers: Vec<crate::narrative::marker::NarrativeMarker>,
    ) {
        let main_window = self.main_window.id;
        let Some(dashboard) = self.active_dashboard_mut() else {
            return;
        };
        for (_, _, state) in dashboard.iter_all_panes_mut(main_window) {
            if let crate::screen::dashboard::pane::Content::Kline {
                chart: Some(chart), ..
            } = &mut state.content
            {
                chart.set_narrative_markers(markers.clone());
            }
        }
    }
}
