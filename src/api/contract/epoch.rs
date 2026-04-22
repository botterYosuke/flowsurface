//! `EpochMs` — agent API 境界で扱う Unix ミリ秒タイムスタンプ。
//!
//! 既存 `u64 ms` / `chrono::DateTime` への暗黙キャスト（特に `as i64`）による
//! silent overflow を封じるための newtype。

use serde::{Deserialize, Serialize};
use std::num::TryFromIntError;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EpochMs(pub u64);

impl EpochMs {
    pub const fn new(ms: u64) -> Self {
        Self(ms)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Narrative ストア等 `i64` を要求する境界への変換。
    /// `u64::MAX > i64::MAX` 領域では `TryFromIntError` を返し、silent な
    /// 負値化（`as i64` の振る舞い）を回避する。
    pub fn try_as_i64(self) -> Result<i64, TryFromIntError> {
        i64::try_from(self.0)
    }
}

impl From<u64> for EpochMs {
    fn from(ms: u64) -> Self {
        Self(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_u64_returns_inner_value() {
        let t = EpochMs::new(1_704_067_200_000);
        assert_eq!(t.as_u64(), 1_704_067_200_000);
    }

    #[test]
    fn try_as_i64_ok_for_values_within_i64_range() {
        let t = EpochMs::new(1_704_067_200_000);
        assert_eq!(t.try_as_i64().unwrap(), 1_704_067_200_000_i64);
    }

    #[test]
    fn try_as_i64_errors_on_values_beyond_i64_max() {
        let t = EpochMs::new(u64::MAX);
        assert!(t.try_as_i64().is_err());
    }

    #[test]
    fn try_as_i64_ok_at_i64_max_boundary() {
        let t = EpochMs::new(i64::MAX as u64);
        assert_eq!(t.try_as_i64().unwrap(), i64::MAX);
    }

    #[test]
    fn serde_roundtrip_as_transparent_integer() {
        let original = EpochMs::new(1_704_067_200_000);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "1704067200000"); // transparent: 裸の整数
        let restored: EpochMs = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn deserializes_from_raw_json_integer() {
        let t: EpochMs = serde_json::from_str("1704067200000").unwrap();
        assert_eq!(t, EpochMs::new(1_704_067_200_000));
    }

    #[test]
    fn rejects_negative_json_integer() {
        let result: Result<EpochMs, _> = serde_json::from_str("-1");
        assert!(
            result.is_err(),
            "negative values must not deserialize to u64"
        );
    }

    #[test]
    fn ordering_and_equality_reflect_inner_u64() {
        assert!(EpochMs::new(1) < EpochMs::new(2));
        assert_eq!(EpochMs::new(42), EpochMs::new(42));
    }

    #[test]
    fn from_u64_conversion() {
        let t: EpochMs = 100_u64.into();
        assert_eq!(t.as_u64(), 100);
    }
}
