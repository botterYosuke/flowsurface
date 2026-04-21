//! `SnapshotStore`: observation_snapshot の gzip + sha256 ファイル永続化。
//!
//! 計画 §3.2 に従い、各ナラティブの大容量スナップショットを
//! `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` に保存する。
//!
//! - **書き込み**: 先に圧縮前サイズをチェック → gzip 圧縮 → 圧縮後サイズ再チェック
//!   → 親ディレクトリ作成 → 書き込み → sha256 計算。
//! - **読み込み**: 指定パスを読み取り → sha256 照合 → 解凍 → `serde_json::Value`。
//! - サイズ上限: 圧縮前 10 MB / 圧縮後 2 MB。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, TimeZone, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::model::SnapshotRef;

pub const SNAPSHOT_SUBDIR: &str = "narratives/snapshots";

pub const MAX_UNCOMPRESSED_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_COMPRESSED_BYTES: u64 = 2 * 1024 * 1024;
pub const WARN_COMPRESSED_BYTES: u64 = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde_json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error(
        "payload too large: uncompressed {uncompressed} bytes exceeds limit {MAX_UNCOMPRESSED_BYTES}"
    )]
    PayloadTooLargeUncompressed { uncompressed: u64 },

    #[error(
        "payload too large: compressed {compressed} bytes exceeds limit {MAX_COMPRESSED_BYTES}"
    )]
    PayloadTooLargeCompressed { compressed: u64 },

    #[error("snapshot integrity check failed: expected sha256 {expected}, got {actual}")]
    IntegrityMismatch { expected: String, actual: String },
}

/// `observation_snapshot` ファイル群の入出力を担う。
///
/// ルートディレクトリ（`root`）配下に `narratives/snapshots/{yyyy}/{mm}/{dd}/` を
/// 動的に作成する。テストでは一時ディレクトリを、本番では `data_path()` を渡す。
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    root: PathBuf,
}

impl SnapshotStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// スナップショット本体（`serde_json::Value`）を書き込む。
    ///
    /// `timestamp_ms` は年月日ディレクトリ分割に使用する（UTC 基準）。
    /// 戻り値は [`SnapshotRef`]（相対パス・圧縮後サイズ・sha256）。
    pub fn write(
        &self,
        id: Uuid,
        timestamp_ms: i64,
        snapshot: &serde_json::Value,
    ) -> Result<SnapshotRef, SnapshotStoreError> {
        let uncompressed = serde_json::to_vec(snapshot)?;
        let uncompressed_len = uncompressed.len() as u64;
        if uncompressed_len > MAX_UNCOMPRESSED_BYTES {
            return Err(SnapshotStoreError::PayloadTooLargeUncompressed {
                uncompressed: uncompressed_len,
            });
        }

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&uncompressed)?;
        let compressed = encoder.finish()?;
        let compressed_len = compressed.len() as u64;
        if compressed_len > MAX_COMPRESSED_BYTES {
            return Err(SnapshotStoreError::PayloadTooLargeCompressed {
                compressed: compressed_len,
            });
        }
        if compressed_len > WARN_COMPRESSED_BYTES {
            log::warn!(
                "narrative snapshot {} compressed size {} exceeds warn threshold {}",
                id,
                compressed_len,
                WARN_COMPRESSED_BYTES
            );
        }

        let rel_path = relative_snapshot_path(id, timestamp_ms);
        let abs_path = self.root.join(&rel_path);
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&abs_path, &compressed)?;

        let sha256 = hex_digest(&compressed);

        Ok(SnapshotRef {
            path: rel_path,
            size_bytes: compressed_len,
            sha256,
        })
    }

    /// スナップショット本体を読み取り、sha256 を検証してから解凍する。
    pub fn read(&self, reference: &SnapshotRef) -> Result<serde_json::Value, SnapshotStoreError> {
        let abs_path = self.root.join(&reference.path);
        let compressed = std::fs::read(&abs_path)?;
        let actual = hex_digest(&compressed);
        if actual != reference.sha256 {
            return Err(SnapshotStoreError::IntegrityMismatch {
                expected: reference.sha256.clone(),
                actual,
            });
        }
        let mut decoder = GzDecoder::new(&compressed[..]);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded)?;
        let value: serde_json::Value = serde_json::from_slice(&decoded)?;
        Ok(value)
    }
}

/// `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` 形式の相対パスを作る。
pub fn relative_snapshot_path(id: Uuid, timestamp_ms: i64) -> PathBuf {
    let datetime = Utc
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("epoch"));
    PathBuf::from(SNAPSHOT_SUBDIR)
        .join(format!("{:04}", datetime.year()))
        .join(format!("{:02}", datetime.month()))
        .join(format!("{:02}", datetime.day()))
        .join(format!("{id}.json.gz"))
}

fn hex_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = base.join(format!(
            "flowsurface-snapshot-test-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("mkdir tmp");
        dir
    }

    #[test]
    fn write_then_read_roundtrip() {
        let store = SnapshotStore::new(tempdir());
        let id = Uuid::new_v4();
        let snapshot = json!({"ohlcv": [[1, 2, 3, 4, 5]], "rsi": 28.3});
        let reference = store
            .write(id, 1_704_067_200_000, &snapshot)
            .expect("write");
        assert!(reference.size_bytes > 0);
        assert_eq!(reference.sha256.len(), 64);
        assert!(reference.path.starts_with(SNAPSHOT_SUBDIR));

        let reloaded = store.read(&reference).expect("read");
        assert_eq!(reloaded, snapshot);
    }

    #[test]
    fn write_creates_year_month_day_subdirectories() {
        use chrono::TimeZone;
        let store = SnapshotStore::new(tempdir());
        let id = Uuid::new_v4();
        let ts = chrono::Utc
            .with_ymd_and_hms(2026, 4, 21, 12, 34, 56)
            .unwrap()
            .timestamp_millis();
        let reference = store.write(id, ts, &json!({})).expect("write");
        let parts: Vec<_> = reference
            .path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert!(parts.contains(&"2026".to_string()));
        assert!(parts.contains(&"04".to_string()));
        assert!(parts.contains(&"21".to_string()));
    }

    #[test]
    fn read_detects_sha256_mismatch() {
        let root = tempdir();
        let store = SnapshotStore::new(&root);
        let id = Uuid::new_v4();
        let reference = store.write(id, 0, &json!({"a": 1})).expect("write");

        // ファイルを破壊する
        let abs_path = root.join(&reference.path);
        let mut corrupted = std::fs::read(&abs_path).unwrap();
        corrupted[0] ^= 0xff;
        std::fs::write(&abs_path, corrupted).unwrap();

        let err = store.read(&reference).expect_err("should fail");
        assert!(matches!(err, SnapshotStoreError::IntegrityMismatch { .. }));
    }

    #[test]
    fn rejects_payload_over_uncompressed_limit() {
        let store = SnapshotStore::new(tempdir());
        // `10 MB + 1 byte` の文字列を JSON 化する（文字列リテラルはそれ以上に膨らむ）。
        let big = "x".repeat((MAX_UNCOMPRESSED_BYTES as usize) + 1);
        let snapshot = json!({ "big": big });
        let err = store
            .write(Uuid::new_v4(), 0, &snapshot)
            .expect_err("should reject");
        assert!(matches!(
            err,
            SnapshotStoreError::PayloadTooLargeUncompressed { .. }
        ));
    }
}
