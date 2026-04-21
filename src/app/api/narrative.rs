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
    ) {
        reply.send_status(status, body);
    }
}
