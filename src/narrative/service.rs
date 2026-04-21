//! ナラティブ HTTP API の共有ロジック。
//!
//! GUI モード（`src/app/api/narrative.rs`）と headless モード（`src/headless.rs`）の
//! 両方から呼び出される、ストレージ側のサービス関数をまとめる。
//!
//! 返り値は `(status_code, body_json)` のタプルで、HTTP レスポンスに直接使える。

use std::sync::Arc;

use uuid::Uuid;

use super::model::{Narrative, NarrativeOutcome, NarrativeSide};
use super::snapshot_store::{SnapshotStore, SnapshotStoreError};
use super::store::{ListFilter, NarrativeStore, NarrativeStoreError};
use crate::replay_api::{NarrativeCreateRequest, NarrativeListQuery};

/// ナラティブ作成（`POST /api/agent/narrative`）。
///
/// `now_ms` は `timestamp_ms` が未指定のときに使うサーバー側の仮想時刻。
/// `created_at_ms` は常に実時間（監査用）。
pub async fn create_narrative(
    store: &Arc<NarrativeStore>,
    snapshot_store: &SnapshotStore,
    req: NarrativeCreateRequest,
    now_ms: i64,
    created_at_ms: i64,
) -> (u16, String) {
    let id = Uuid::new_v4();
    let timestamp_ms = req.timestamp_ms.unwrap_or(now_ms);

    let snapshot_ref = match snapshot_store.write(id, timestamp_ms, &req.observation_snapshot) {
        Ok(r) => r,
        Err(SnapshotStoreError::PayloadTooLargeUncompressed { .. })
        | Err(SnapshotStoreError::PayloadTooLargeCompressed { .. }) => {
            return (
                413,
                serde_json::json!({"error": "Payload Too Large"}).to_string(),
            );
        }
        Err(e) => {
            return (
                500,
                serde_json::json!({"error": format!("snapshot write failed: {e}")}).to_string(),
            );
        }
    };
    let snapshot_bytes = snapshot_ref.size_bytes;

    let narrative = Narrative {
        id,
        agent_id: req.agent_id,
        uagent_address: req.uagent_address,
        timestamp_ms,
        ticker: req.ticker,
        timeframe: req.timeframe,
        snapshot_ref,
        reasoning: req.reasoning,
        action: req.action,
        confidence: req.confidence,
        outcome: None,
        linked_order_id: req.linked_order_id,
        public: false,
        created_at_ms,
        idempotency_key: req.idempotency_key,
    };

    match store.insert(narrative).await {
        Ok(result) => {
            let body = serde_json::json!({
                "id": result.narrative.id,
                "snapshot_bytes": snapshot_bytes,
                "idempotent_replay": result.idempotent_replay,
            })
            .to_string();
            (201, body)
        }
        Err(e) => (
            500,
            serde_json::json!({"error": format!("db insert failed: {e}")}).to_string(),
        ),
    }
}

pub async fn list_narratives(
    store: &Arc<NarrativeStore>,
    query: NarrativeListQuery,
) -> (u16, String) {
    let filter = ListFilter {
        agent_id: query.agent_id,
        ticker: query.ticker,
        since_ms: query.since_ms,
        limit: query.limit,
    };
    match store.list(filter).await {
        Ok(narratives) => (
            200,
            serde_json::json!({ "narratives": narratives }).to_string(),
        ),
        Err(e) => (
            500,
            serde_json::json!({"error": format!("db list failed: {e}")}).to_string(),
        ),
    }
}

pub async fn get_narrative(store: &Arc<NarrativeStore>, id: Uuid) -> (u16, String) {
    match store.get(id).await {
        Ok(Some(n)) => (
            200,
            serde_json::to_string(&n)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
        ),
        Ok(None) => (404, serde_json::json!({"error": "not found"}).to_string()),
        Err(e) => (
            500,
            serde_json::json!({"error": format!("db get failed: {e}")}).to_string(),
        ),
    }
}

/// スナップショット本体を読み取って返す。sha256 不一致は 410 Gone。
pub async fn get_narrative_snapshot(
    store: &Arc<NarrativeStore>,
    snapshot_store: &SnapshotStore,
    id: Uuid,
) -> (u16, String) {
    let snapshot_ref = match store.snapshot_ref_of(id).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            return (404, serde_json::json!({"error": "not found"}).to_string());
        }
        Err(e) => {
            return (
                500,
                serde_json::json!({"error": format!("db get failed: {e}")}).to_string(),
            );
        }
    };
    let snapshot_store = snapshot_store.clone();
    let reference = snapshot_ref.clone();
    let result = tokio::task::spawn_blocking(move || snapshot_store.read(&reference)).await;
    match result {
        Ok(Ok(value)) => (
            200,
            serde_json::to_string(&value)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
        ),
        Ok(Err(SnapshotStoreError::IntegrityMismatch { .. })) => (
            410,
            serde_json::json!({"error": "snapshot integrity mismatch"}).to_string(),
        ),
        Ok(Err(SnapshotStoreError::Io(e))) if e.kind() == std::io::ErrorKind::NotFound => {
            (404, serde_json::json!({"error": "not found"}).to_string())
        }
        Ok(Err(e)) => (
            500,
            serde_json::json!({"error": format!("snapshot read failed: {e}")}).to_string(),
        ),
        Err(e) => (
            500,
            serde_json::json!({"error": format!("task panic: {e}")}).to_string(),
        ),
    }
}

pub async fn patch_narrative(store: &Arc<NarrativeStore>, id: Uuid, public: bool) -> (u16, String) {
    match store.set_public(id, public).await {
        Ok(n) => (
            200,
            serde_json::to_string(&n)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
        ),
        Err(NarrativeStoreError::NotFound(_)) => {
            (404, serde_json::json!({"error": "not found"}).to_string())
        }
        Err(e) => (
            500,
            serde_json::json!({"error": format!("db update failed: {e}")}).to_string(),
        ),
    }
}

pub async fn storage_stats(store: &Arc<NarrativeStore>) -> (u16, String) {
    match store.storage_stats().await {
        Ok(s) => (
            200,
            serde_json::to_string(&s)
                .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string()),
        ),
        Err(e) => (
            500,
            serde_json::json!({"error": format!("db stats failed: {e}")}).to_string(),
        ),
    }
}

pub async fn orphans(
    store: &Arc<NarrativeStore>,
    snapshots_root: std::path::PathBuf,
) -> (u16, String) {
    match store.gc_orphans(snapshots_root).await {
        Ok(paths) => {
            let strs: Vec<String> = paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            (200, serde_json::json!({ "orphan_files": strs }).to_string())
        }
        Err(e) => (
            500,
            serde_json::json!({"error": format!("gc orphans failed: {e}")}).to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narrative::model::{NarrativeAction, NarrativeSide};
    use crate::replay_api::NarrativeCreateRequest;

    fn tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!(
            "flowsurface-service-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn sample_request(agent: &str, idempotency: Option<&str>) -> NarrativeCreateRequest {
        NarrativeCreateRequest {
            agent_id: agent.to_string(),
            uagent_address: None,
            ticker: "BTCUSDT".to_string(),
            timeframe: "1h".to_string(),
            observation_snapshot: serde_json::json!({ "rsi": 28.3 }),
            reasoning: "RSI divergence".to_string(),
            action: NarrativeAction {
                side: NarrativeSide::Buy,
                qty: 0.1,
                price: 92_500.0,
            },
            confidence: 0.76,
            linked_order_id: Some("ord_1".to_string()),
            timestamp_ms: Some(1_000_000),
            idempotency_key: idempotency.map(|s| s.to_string()),
        }
    }

    #[test]
    fn create_then_get_then_patch_then_snapshot_roundtrip() {
        let rt = rt();
        rt.block_on(async {
            let root = tempdir();
            let store = Arc::new(NarrativeStore::open_in_memory().unwrap());
            let snapshot_store = SnapshotStore::new(&root);

            // Create
            let (status, body) =
                create_narrative(&store, &snapshot_store, sample_request("alpha", None), 0, 1)
                    .await;
            assert_eq!(status, 201);
            let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
            let id = Uuid::parse_str(resp["id"].as_str().unwrap()).unwrap();
            assert_eq!(resp["idempotent_replay"], serde_json::json!(false));

            // Get
            let (status, body) = get_narrative(&store, id).await;
            assert_eq!(status, 200);
            let parsed: Narrative = serde_json::from_str(&body).unwrap();
            assert_eq!(parsed.agent_id, "alpha");
            assert!(!parsed.public);

            // Patch public=true
            let (status, body) = patch_narrative(&store, id, true).await;
            assert_eq!(status, 200);
            let parsed: Narrative = serde_json::from_str(&body).unwrap();
            assert!(parsed.public);

            // Snapshot roundtrip
            let (status, body) = get_narrative_snapshot(&store, &snapshot_store, id).await;
            assert_eq!(status, 200);
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(parsed["rsi"], serde_json::json!(28.3));
        });
    }

    #[test]
    fn idempotent_create_returns_same_id() {
        let rt = rt();
        rt.block_on(async {
            let root = tempdir();
            let store = Arc::new(NarrativeStore::open_in_memory().unwrap());
            let snapshot_store = SnapshotStore::new(&root);

            let (_, body1) = create_narrative(
                &store,
                &snapshot_store,
                sample_request("alpha", Some("step_1")),
                0,
                1,
            )
            .await;
            let (status2, body2) = create_narrative(
                &store,
                &snapshot_store,
                sample_request("alpha", Some("step_1")),
                0,
                1,
            )
            .await;
            assert_eq!(status2, 201);
            let r1: serde_json::Value = serde_json::from_str(&body1).unwrap();
            let r2: serde_json::Value = serde_json::from_str(&body2).unwrap();
            assert_eq!(r1["id"], r2["id"]);
            assert_eq!(r2["idempotent_replay"], serde_json::json!(true));
        });
    }

    #[test]
    fn get_snapshot_returns_410_on_integrity_mismatch() {
        let rt = rt();
        rt.block_on(async {
            let root = tempdir();
            let store = Arc::new(NarrativeStore::open_in_memory().unwrap());
            let snapshot_store = SnapshotStore::new(&root);

            let (_, body) =
                create_narrative(&store, &snapshot_store, sample_request("alpha", None), 0, 1)
                    .await;
            let id = Uuid::parse_str(
                serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();

            // ファイルを破壊して sha256 不一致を起こす
            let narrative = store.get(id).await.unwrap().unwrap();
            let abs = root.join(&narrative.snapshot_ref.path);
            let mut bytes = std::fs::read(&abs).unwrap();
            bytes[0] ^= 0xff;
            std::fs::write(&abs, bytes).unwrap();

            let (status, _) = get_narrative_snapshot(&store, &snapshot_store, id).await;
            assert_eq!(status, 410);
        });
    }

    #[test]
    fn patch_returns_404_for_unknown_id() {
        let rt = rt();
        rt.block_on(async {
            let store = Arc::new(NarrativeStore::open_in_memory().unwrap());
            let (status, _) = patch_narrative(&store, Uuid::new_v4(), true).await;
            assert_eq!(status, 404);
        });
    }

    #[test]
    fn list_filters_by_agent() {
        let rt = rt();
        rt.block_on(async {
            let root = tempdir();
            let store = Arc::new(NarrativeStore::open_in_memory().unwrap());
            let snapshot_store = SnapshotStore::new(&root);

            create_narrative(&store, &snapshot_store, sample_request("alpha", None), 0, 1).await;
            create_narrative(&store, &snapshot_store, sample_request("beta", None), 0, 1).await;
            create_narrative(&store, &snapshot_store, sample_request("alpha", None), 0, 1).await;

            let (status, body) = list_narratives(
                &store,
                NarrativeListQuery {
                    agent_id: Some("alpha".to_string()),
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(status, 200);
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(parsed["narratives"].as_array().unwrap().len(), 2);
        });
    }
}

/// FillEvent 連携: order_id に紐付くナラティブの outcome を更新する。
///
/// Phase 2 の `PositionSide` は計算で使わない（side は action 側に既に記録済み）。
/// ログ用にのみ受け取る。
pub async fn update_outcome_from_fill(
    store: &Arc<NarrativeStore>,
    order_id: &str,
    fill_price: f64,
    fill_time_ms: i64,
    _side_hint: Option<NarrativeSide>,
) -> Result<usize, NarrativeStoreError> {
    let outcome = NarrativeOutcome {
        fill_price,
        fill_time_ms,
        closed_at_ms: None,
        realized_pnl: None,
    };
    store.update_outcome_by_order_id(order_id, outcome).await
}
