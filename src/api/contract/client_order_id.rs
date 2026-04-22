//! `ClientOrderId` — agent が注文冪等性のために指定するクライアント採番 ID。
//!
//! 制約: 1..=64 文字、`[A-Za-z0-9_-]` のみ。regex を外部クレートに依存せず
//! 手書きで検証する（依存最小化）。

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ClientOrderId(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClientOrderIdError {
    #[error("client_order_id must be between 1 and 64 characters (got {0})")]
    InvalidLength(usize),
    #[error("client_order_id must match [A-Za-z0-9_-] (invalid char {0:?})")]
    InvalidCharacter(char),
}

impl ClientOrderId {
    pub fn new(value: impl Into<String>) -> Result<Self, ClientOrderIdError> {
        let value: String = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(value: &str) -> Result<(), ClientOrderIdError> {
        let len = value.len();
        if !(1..=64).contains(&len) {
            return Err(ClientOrderIdError::InvalidLength(len));
        }
        if let Some(ch) = value
            .chars()
            .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        {
            return Err(ClientOrderIdError::InvalidCharacter(ch));
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for ClientOrderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        ClientOrderId::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_minimum_length_one() {
        let id = ClientOrderId::new("a").unwrap();
        assert_eq!(id.as_str(), "a");
    }

    #[test]
    fn accepts_maximum_length_sixty_four() {
        let s = "a".repeat(64);
        let id = ClientOrderId::new(&s).unwrap();
        assert_eq!(id.as_str().len(), 64);
    }

    #[test]
    fn accepts_allowed_character_set() {
        let id = ClientOrderId::new("abc_DEF-012").unwrap();
        assert_eq!(id.as_str(), "abc_DEF-012");
    }

    #[test]
    fn rejects_empty_string() {
        let err = ClientOrderId::new("").unwrap_err();
        assert_eq!(err, ClientOrderIdError::InvalidLength(0));
    }

    #[test]
    fn rejects_length_sixty_five() {
        let s = "a".repeat(65);
        let err = ClientOrderId::new(&s).unwrap_err();
        assert_eq!(err, ClientOrderIdError::InvalidLength(65));
    }

    #[test]
    fn rejects_space() {
        let err = ClientOrderId::new("foo bar").unwrap_err();
        assert_eq!(err, ClientOrderIdError::InvalidCharacter(' '));
    }

    #[test]
    fn rejects_slash() {
        let err = ClientOrderId::new("foo/bar").unwrap_err();
        assert_eq!(err, ClientOrderIdError::InvalidCharacter('/'));
    }

    #[test]
    fn rejects_non_ascii() {
        let err = ClientOrderId::new("foo_é").unwrap_err();
        assert_eq!(err, ClientOrderIdError::InvalidCharacter('é'));
    }

    #[test]
    fn serde_serializes_as_bare_string() {
        let id = ClientOrderId::new("cli_42").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""cli_42""#);
    }

    #[test]
    fn serde_deserializes_valid_string() {
        let id: ClientOrderId = serde_json::from_str(r#""cli_42""#).unwrap();
        assert_eq!(id.as_str(), "cli_42");
    }

    #[test]
    fn serde_rejects_invalid_string_on_deserialize() {
        let result: Result<ClientOrderId, _> = serde_json::from_str(r#""foo bar""#);
        assert!(result.is_err(), "validation must run during deserialize");
    }

    #[test]
    fn serde_rejects_empty_string_on_deserialize() {
        let result: Result<ClientOrderId, _> = serde_json::from_str(r#""""#);
        assert!(result.is_err());
    }

    #[test]
    fn serde_rejects_too_long_string_on_deserialize() {
        let s = "a".repeat(65);
        let json = format!("\"{s}\"");
        let result: Result<ClientOrderId, _> = serde_json::from_str(&json);
        assert!(result.is_err());
    }
}
