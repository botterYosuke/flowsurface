//! `NarrativeStore`: ナラティブのメタデータ永続化（SQLite）。
//!
//! 計画 §3.2 の方針に従い、メタデータのみ SQLite に保存し、
//! `observation_snapshot` 本体は [`crate::narrative::snapshot_store::SnapshotStore`]
//! で別ファイルに永続化する。
//!
//! # 並行性モデル
//!
//! 単一の `rusqlite::Connection` を `Arc<tokio::sync::Mutex<_>>` で共有する（§3.2）。
//! ブロッキング I/O になる書き込み・読み込みは、呼び出し側で
//! `tokio::task::spawn_blocking` に包むことを推奨する。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::model::{Narrative, NarrativeAction, NarrativeOutcome, SnapshotRef};

/// ナラティブストアのエラー。
#[derive(Debug, thiserror::Error)]
pub enum NarrativeStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serde_json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("narrative not found: {0}")]
    NotFound(Uuid),
}

/// `insert` の結果。
///
/// `idempotent_replay` が `true` の場合、`idempotency_key` が既に登録済みで
/// 新規 INSERT はされなかった（既存の Narrative をそのまま返した）ことを示す。
#[derive(Debug, Clone, PartialEq)]
pub struct InsertResult {
    pub narrative: Narrative,
    pub idempotent_replay: bool,
}

/// ストレージ統計（`GET /api/agent/narratives/storage` で返す）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StorageStats {
    pub total_count: u64,
    pub total_bytes: u64,
    /// 圧縮後サイズが WARN しきい値を超えるファイルの件数。
    pub warn_count: u64,
}

/// SQLite ベースのナラティブストア。
#[derive(Clone)]
pub struct NarrativeStore {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl NarrativeStore {
    /// 指定した SQLite ファイルパスでストアを開く（ファイルは自動作成）。
    pub fn open_at_path<P: AsRef<Path>>(path: P) -> Result<Self, NarrativeStoreError> {
        let db_path = path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        apply_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
        })
    }

    /// `data_path()/narratives.db` をデフォルトパスとしてストアを開く。
    pub fn open_default() -> Result<Self, NarrativeStoreError> {
        let path = data::data_path(Some("narratives.db"));
        Self::open_at_path(path)
    }

    /// テスト用: インメモリ SQLite でストアを開く。
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, NarrativeStoreError> {
        let conn = Connection::open_in_memory()?;
        apply_migrations(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: PathBuf::from(":memory:"),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// ナラティブを挿入する（メタデータのみ、スナップショット本体は事前に書き込み済みとする）。
    ///
    /// `idempotency_key` が指定されており、同一 `(agent_id, idempotency_key)` が
    /// 既に存在する場合は、新規 INSERT せず既存レコードを返す。
    pub async fn insert(&self, narrative: Narrative) -> Result<InsertResult, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.blocking_lock();
            insert_sync(&mut guard, narrative)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// ID で単一ナラティブを取得する（メタのみ、スナップショット本体は含まない）。
    pub async fn get(&self, id: Uuid) -> Result<Option<Narrative>, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            get_sync(&guard, id)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// 一覧を取得する（メタのみ、スナップショット本体は含まない）。
    pub async fn list(&self, filter: ListFilter) -> Result<Vec<Narrative>, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            list_sync(&guard, &filter)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// `outcome` を自動更新する（FillEvent 連携用）。
    ///
    /// `linked_order_id` に一致する全ナラティブの `outcome` を置き換える。
    /// 戻り値は更新された件数。
    pub async fn update_outcome_by_order_id(
        &self,
        order_id: &str,
        outcome: NarrativeOutcome,
    ) -> Result<usize, NarrativeStoreError> {
        let conn = self.conn.clone();
        let order_id = order_id.to_string();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            update_outcome_sync(&guard, &order_id, &outcome)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// `public` フラグを更新する（取消対応）。
    pub async fn set_public(
        &self,
        id: Uuid,
        public: bool,
    ) -> Result<Narrative, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            set_public_sync(&guard, id, public)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// ストレージ統計（件数・合計バイト数・WARN しきい値超過件数）。
    pub async fn storage_stats(&self) -> Result<StorageStats, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            storage_stats_sync(&guard)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// 孤児スナップショット（SQLite に登録されていないファイル）のパス一覧を返す。
    /// **自動削除はしない**（計画 §3.2 方針）。
    pub async fn gc_orphans(
        &self,
        snapshots_root: PathBuf,
    ) -> Result<Vec<PathBuf>, NarrativeStoreError> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            gc_orphans_sync(&guard, &snapshots_root)
        })
        .await
        .map_err(|e| NarrativeStoreError::Io(std::io::Error::other(e)))?
    }

    /// `snapshot_ref` だけを取り出す（ファイル本体を読み込む前の事前取得用）。
    pub async fn snapshot_ref_of(
        &self,
        id: Uuid,
    ) -> Result<Option<SnapshotRef>, NarrativeStoreError> {
        Ok(self.get(id).await?.map(|n| n.snapshot_ref))
    }
}

/// `list` のフィルタ。
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub agent_id: Option<String>,
    pub ticker: Option<String>,
    pub since_ms: Option<i64>,
    /// 返す最大件数。`None` は 100 件、上限は 1000 件に丸める。
    pub limit: Option<usize>,
}

impl ListFilter {
    pub const DEFAULT_LIMIT: usize = 100;
    pub const MAX_LIMIT: usize = 1000;

    fn effective_limit(&self) -> usize {
        self.limit
            .unwrap_or(Self::DEFAULT_LIMIT)
            .min(Self::MAX_LIMIT)
    }
}

// ── 内部同期関数（`spawn_blocking` 内から呼ばれる） ───────────────────────────

fn apply_migrations(conn: &Connection) -> Result<(), NarrativeStoreError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS narratives (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            uagent_address TEXT,
            timestamp_ms INTEGER NOT NULL,
            ticker TEXT NOT NULL,
            timeframe TEXT NOT NULL,
            snapshot_path TEXT NOT NULL,
            snapshot_size_bytes INTEGER NOT NULL,
            snapshot_sha256 TEXT NOT NULL,
            reasoning TEXT NOT NULL,
            action_json TEXT NOT NULL,
            confidence REAL NOT NULL,
            outcome_json TEXT,
            linked_order_id TEXT,
            public INTEGER NOT NULL DEFAULT 0,
            created_at_ms INTEGER NOT NULL,
            idempotency_key TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_narratives_agent_ticker
            ON narratives(agent_id, ticker);
        CREATE INDEX IF NOT EXISTS idx_narratives_timestamp
            ON narratives(timestamp_ms);
        CREATE INDEX IF NOT EXISTS idx_narratives_order
            ON narratives(linked_order_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_narratives_idempotency
            ON narratives(agent_id, idempotency_key)
            WHERE idempotency_key IS NOT NULL;
        ",
    )?;
    Ok(())
}

fn insert_sync(
    conn: &mut Connection,
    narrative: Narrative,
) -> Result<InsertResult, NarrativeStoreError> {
    if let Some(key) = &narrative.idempotency_key
        && let Some(existing) = lookup_by_idempotency(conn, &narrative.agent_id, key)?
    {
        return Ok(InsertResult {
            narrative: existing,
            idempotent_replay: true,
        });
    }

    let action_json = serde_json::to_string(&narrative.action)?;
    let outcome_json = match &narrative.outcome {
        Some(o) => Some(serde_json::to_string(o)?),
        None => None,
    };
    let snapshot_path = narrative.snapshot_ref.path.to_string_lossy().into_owned();

    conn.execute(
        "INSERT INTO narratives (
            id, agent_id, uagent_address, timestamp_ms, ticker, timeframe,
            snapshot_path, snapshot_size_bytes, snapshot_sha256,
            reasoning, action_json, confidence, outcome_json,
            linked_order_id, public, created_at_ms, idempotency_key
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            narrative.id.to_string(),
            narrative.agent_id,
            narrative.uagent_address,
            narrative.timestamp_ms,
            narrative.ticker,
            narrative.timeframe,
            snapshot_path,
            narrative.snapshot_ref.size_bytes as i64,
            narrative.snapshot_ref.sha256,
            narrative.reasoning,
            action_json,
            narrative.confidence,
            outcome_json,
            narrative.linked_order_id,
            narrative.public as i64,
            narrative.created_at_ms,
            narrative.idempotency_key,
        ],
    )?;

    Ok(InsertResult {
        narrative,
        idempotent_replay: false,
    })
}

fn lookup_by_idempotency(
    conn: &Connection,
    agent_id: &str,
    key: &str,
) -> Result<Option<Narrative>, NarrativeStoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, uagent_address, timestamp_ms, ticker, timeframe,
                snapshot_path, snapshot_size_bytes, snapshot_sha256,
                reasoning, action_json, confidence, outcome_json,
                linked_order_id, public, created_at_ms, idempotency_key
           FROM narratives
          WHERE agent_id = ?1 AND idempotency_key = ?2
          LIMIT 1",
    )?;
    let narrative = stmt
        .query_row(params![agent_id, key], row_to_narrative)
        .optional()?;
    Ok(narrative)
}

fn get_sync(conn: &Connection, id: Uuid) -> Result<Option<Narrative>, NarrativeStoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, uagent_address, timestamp_ms, ticker, timeframe,
                snapshot_path, snapshot_size_bytes, snapshot_sha256,
                reasoning, action_json, confidence, outcome_json,
                linked_order_id, public, created_at_ms, idempotency_key
           FROM narratives
          WHERE id = ?1",
    )?;
    let result = stmt
        .query_row(params![id.to_string()], row_to_narrative)
        .optional()?;
    Ok(result)
}

fn list_sync(
    conn: &Connection,
    filter: &ListFilter,
) -> Result<Vec<Narrative>, NarrativeStoreError> {
    let mut sql = String::from(
        "SELECT id, agent_id, uagent_address, timestamp_ms, ticker, timeframe,
                snapshot_path, snapshot_size_bytes, snapshot_sha256,
                reasoning, action_json, confidence, outcome_json,
                linked_order_id, public, created_at_ms, idempotency_key
           FROM narratives",
    );
    let mut clauses = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(agent_id) = &filter.agent_id {
        clauses.push("agent_id = ?");
        args.push(Box::new(agent_id.clone()));
    }
    if let Some(ticker) = &filter.ticker {
        clauses.push("ticker = ?");
        args.push(Box::new(ticker.clone()));
    }
    if let Some(since_ms) = filter.since_ms {
        clauses.push("timestamp_ms >= ?");
        args.push(Box::new(since_ms));
    }

    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY timestamp_ms DESC LIMIT ?");
    args.push(Box::new(filter.effective_limit() as i64));

    let mut stmt = conn.prepare(&sql)?;
    let args_refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(args_refs.as_slice(), row_to_narrative)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn update_outcome_sync(
    conn: &Connection,
    order_id: &str,
    outcome: &NarrativeOutcome,
) -> Result<usize, NarrativeStoreError> {
    let outcome_json = serde_json::to_string(outcome)?;
    let n = conn.execute(
        "UPDATE narratives SET outcome_json = ?1 WHERE linked_order_id = ?2",
        params![outcome_json, order_id],
    )?;
    Ok(n)
}

fn set_public_sync(
    conn: &Connection,
    id: Uuid,
    public: bool,
) -> Result<Narrative, NarrativeStoreError> {
    let n = conn.execute(
        "UPDATE narratives SET public = ?1 WHERE id = ?2",
        params![public as i64, id.to_string()],
    )?;
    if n == 0 {
        return Err(NarrativeStoreError::NotFound(id));
    }
    get_sync(conn, id)?.ok_or(NarrativeStoreError::NotFound(id))
}

fn storage_stats_sync(conn: &Connection) -> Result<StorageStats, NarrativeStoreError> {
    use crate::narrative::snapshot_store::WARN_COMPRESSED_BYTES;
    let mut stmt = conn.prepare(
        "SELECT
            COUNT(*) AS cnt,
            COALESCE(SUM(snapshot_size_bytes), 0) AS total,
            COALESCE(SUM(CASE WHEN snapshot_size_bytes > ?1 THEN 1 ELSE 0 END), 0) AS warn_cnt
           FROM narratives",
    )?;
    let row = stmt.query_row(params![WARN_COMPRESSED_BYTES as i64], |row| {
        Ok((
            row.get::<_, i64>(0)? as u64,
            row.get::<_, i64>(1)? as u64,
            row.get::<_, i64>(2)? as u64,
        ))
    })?;
    Ok(StorageStats {
        total_count: row.0,
        total_bytes: row.1,
        warn_count: row.2,
    })
}

fn gc_orphans_sync(
    conn: &Connection,
    snapshots_root: &Path,
) -> Result<Vec<PathBuf>, NarrativeStoreError> {
    let mut registered = std::collections::HashSet::<PathBuf>::new();
    let mut stmt = conn.prepare("SELECT snapshot_path FROM narratives")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for r in rows {
        registered.insert(PathBuf::from(r?));
    }

    let mut orphans = Vec::new();
    let snapshots_subdir = snapshots_root.join(crate::narrative::snapshot_store::SNAPSHOT_SUBDIR);
    if snapshots_subdir.exists() {
        walk_files(&snapshots_subdir, &mut |abs| {
            if let Ok(rel) = abs.strip_prefix(snapshots_root)
                && !registered.contains(rel)
            {
                orphans.push(rel.to_path_buf());
            }
        })?;
    }
    if !orphans.is_empty() {
        log::warn!(
            "narrative store detected {} orphan snapshot file(s) under {}",
            orphans.len(),
            snapshots_subdir.display()
        );
    }
    Ok(orphans)
}

fn walk_files(dir: &Path, visit: &mut impl FnMut(&Path)) -> Result<(), NarrativeStoreError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, visit)?;
        } else if path.is_file() {
            visit(&path);
        }
    }
    Ok(())
}

fn row_to_narrative(row: &rusqlite::Row<'_>) -> rusqlite::Result<Narrative> {
    let id_str: String = row.get(0)?;
    let id = Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let action_json: String = row.get(10)?;
    let action: NarrativeAction = serde_json::from_str(&action_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let outcome_json: Option<String> = row.get(12)?;
    let outcome = match outcome_json {
        Some(s) => Some(serde_json::from_str(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(12, rusqlite::types::Type::Text, Box::new(e))
        })?),
        None => None,
    };
    let snapshot_path: String = row.get(6)?;
    let snapshot_size_bytes: i64 = row.get(7)?;
    let snapshot_sha256: String = row.get(8)?;
    let snapshot_ref = SnapshotRef {
        path: PathBuf::from(snapshot_path),
        size_bytes: snapshot_size_bytes as u64,
        sha256: snapshot_sha256,
    };
    let public_int: i64 = row.get(14)?;

    Ok(Narrative {
        id,
        agent_id: row.get(1)?,
        uagent_address: row.get(2)?,
        timestamp_ms: row.get(3)?,
        ticker: row.get(4)?,
        timeframe: row.get(5)?,
        snapshot_ref,
        reasoning: row.get(9)?,
        action,
        confidence: row.get(11)?,
        outcome,
        linked_order_id: row.get(13)?,
        public: public_int != 0,
        created_at_ms: row.get(15)?,
        idempotency_key: row.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narrative::model::{NarrativeAction, NarrativeOutcome, NarrativeSide, SnapshotRef};

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = base.join(format!(
            "flowsurface-narrative-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("mkdir tmp");
        dir
    }

    fn sample_narrative(agent: &str, ticker: &str, ts: i64) -> Narrative {
        Narrative {
            id: Uuid::new_v4(),
            agent_id: agent.to_string(),
            uagent_address: None,
            timestamp_ms: ts,
            ticker: ticker.to_string(),
            timeframe: "1h".to_string(),
            snapshot_ref: SnapshotRef {
                path: PathBuf::from(format!(
                    "narratives/snapshots/2026/04/21/{}.json.gz",
                    Uuid::new_v4()
                )),
                size_bytes: 1024,
                sha256: "a".repeat(64),
            },
            reasoning: "test".to_string(),
            action: NarrativeAction {
                side: NarrativeSide::Buy,
                qty: 0.1,
                price: 100.0,
            },
            confidence: 0.5,
            outcome: None,
            linked_order_id: Some("ord_1".to_string()),
            public: false,
            created_at_ms: ts + 1,
            idempotency_key: None,
        }
    }

    #[test]
    fn opens_db_in_data_path() {
        let tmp = tempdir();
        let db_path = tmp.join("narratives.db");
        let store = NarrativeStore::open_at_path(&db_path).expect("open store");
        assert!(db_path.exists(), "sqlite file was not created");
        assert_eq!(store.db_path(), db_path);
    }

    #[test]
    fn creates_parent_directory_if_missing() {
        let tmp = tempdir();
        let db_path = tmp.join("nested/sub/narratives.db");
        let _ = NarrativeStore::open_at_path(&db_path).expect("open store");
        assert!(db_path.exists());
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn insert_and_get_roundtrip() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let narrative = sample_narrative("alpha", "BTCUSDT", 1000);
            let id = narrative.id;
            let result = store.insert(narrative.clone()).await.unwrap();
            assert!(!result.idempotent_replay);

            let fetched = store.get(id).await.unwrap().expect("found");
            assert_eq!(fetched, narrative);
        });
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let fetched = store.get(Uuid::new_v4()).await.unwrap();
            assert!(fetched.is_none());
        });
    }

    #[test]
    fn list_filters_by_agent_ticker_since() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            store
                .insert(sample_narrative("alpha", "BTCUSDT", 1000))
                .await
                .unwrap();
            store
                .insert(sample_narrative("alpha", "ETHUSDT", 2000))
                .await
                .unwrap();
            store
                .insert(sample_narrative("beta", "BTCUSDT", 3000))
                .await
                .unwrap();

            let all = store.list(ListFilter::default()).await.unwrap();
            assert_eq!(all.len(), 3);

            let by_agent = store
                .list(ListFilter {
                    agent_id: Some("alpha".to_string()),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(by_agent.len(), 2);

            let by_ticker = store
                .list(ListFilter {
                    ticker: Some("BTCUSDT".to_string()),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(by_ticker.len(), 2);

            let since = store
                .list(ListFilter {
                    since_ms: Some(2500),
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(since.len(), 1);
            assert_eq!(since[0].agent_id, "beta");
        });
    }

    #[test]
    fn list_limit_is_clamped_to_max() {
        let f = ListFilter {
            limit: Some(10_000),
            ..Default::default()
        };
        assert_eq!(f.effective_limit(), ListFilter::MAX_LIMIT);
    }

    #[test]
    fn idempotency_key_prevents_duplicate_insert() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let mut first = sample_narrative("alpha", "BTCUSDT", 1000);
            first.idempotency_key = Some("step_42".to_string());
            let id1 = first.id;
            store.insert(first.clone()).await.unwrap();

            let mut second = sample_narrative("alpha", "BTCUSDT", 2000);
            second.idempotency_key = Some("step_42".to_string());
            let result = store.insert(second).await.unwrap();
            assert!(result.idempotent_replay);
            assert_eq!(result.narrative.id, id1);

            let all = store.list(ListFilter::default()).await.unwrap();
            assert_eq!(all.len(), 1);
        });
    }

    #[test]
    fn update_outcome_by_order_id_sets_outcome() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let narrative = sample_narrative("alpha", "BTCUSDT", 1000);
            let id = narrative.id;
            store.insert(narrative).await.unwrap();

            let outcome = NarrativeOutcome {
                fill_price: 100.5,
                fill_time_ms: 1500,
                closed_at_ms: None,
                realized_pnl: None,
            };
            let n = store
                .update_outcome_by_order_id("ord_1", outcome.clone())
                .await
                .unwrap();
            assert_eq!(n, 1);

            let fetched = store.get(id).await.unwrap().expect("found");
            assert_eq!(fetched.outcome, Some(outcome));
        });
    }

    #[test]
    fn set_public_toggles_flag() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let narrative = sample_narrative("alpha", "BTCUSDT", 1000);
            let id = narrative.id;
            store.insert(narrative).await.unwrap();

            let updated = store.set_public(id, true).await.unwrap();
            assert!(updated.public);

            let updated = store.set_public(id, false).await.unwrap();
            assert!(!updated.public);
        });
    }

    #[test]
    fn set_public_returns_not_found_for_unknown_id() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let err = store.set_public(Uuid::new_v4(), true).await.unwrap_err();
            assert!(matches!(err, NarrativeStoreError::NotFound(_)));
        });
    }

    #[test]
    fn storage_stats_counts_bytes_and_entries() {
        let rt = rt();
        rt.block_on(async {
            let store = NarrativeStore::open_in_memory().unwrap();
            let mut small = sample_narrative("alpha", "BTCUSDT", 1);
            small.snapshot_ref.size_bytes = 100;
            let mut warn = sample_narrative("beta", "BTCUSDT", 2);
            warn.snapshot_ref.size_bytes = 512 * 1024;
            store.insert(small).await.unwrap();
            store.insert(warn).await.unwrap();

            let stats = store.storage_stats().await.unwrap();
            assert_eq!(stats.total_count, 2);
            assert_eq!(stats.total_bytes, 100 + 512 * 1024);
            assert_eq!(stats.warn_count, 1);
        });
    }

    #[test]
    fn gc_orphans_detects_files_without_rows() {
        let rt = rt();
        rt.block_on(async {
            let tmp = tempdir();
            let store = NarrativeStore::open_in_memory().unwrap();

            // 孤児ファイルを作る
            let orphan_rel = PathBuf::from("narratives/snapshots/2026/04/21/orphan-1.json.gz");
            let orphan_abs = tmp.join(&orphan_rel);
            std::fs::create_dir_all(orphan_abs.parent().unwrap()).unwrap();
            std::fs::write(&orphan_abs, b"dummy").unwrap();

            let orphans = store.gc_orphans(tmp.clone()).await.unwrap();
            assert_eq!(orphans.len(), 1);
            assert_eq!(orphans[0], orphan_rel);
        });
    }

    #[test]
    fn gc_orphans_ignores_registered_files() {
        let rt = rt();
        rt.block_on(async {
            let tmp = tempdir();
            let store = NarrativeStore::open_in_memory().unwrap();

            let mut narrative = sample_narrative("alpha", "BTCUSDT", 1000);
            let rel = PathBuf::from("narratives/snapshots/2026/04/21/registered.json.gz");
            narrative.snapshot_ref.path = rel.clone();
            store.insert(narrative).await.unwrap();

            let abs = tmp.join(&rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, b"registered").unwrap();

            let orphans = store.gc_orphans(tmp.clone()).await.unwrap();
            assert!(orphans.is_empty());
        });
    }
}
