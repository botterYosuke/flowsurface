use super::ScaleFactor;
use super::sidebar::Sidebar;
use super::timezone::UserTimezone;
use crate::layout::WindowSpec;
use crate::{AudioStream, Layout, Theme};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayConfig {
    /// "live" or "replay"
    pub mode: String,
    /// 開始日時の入力文字列 (例: "2026-04-10 09:00")
    pub range_start: String,
    /// 終了日時の入力文字列 (例: "2026-04-10 15:00")
    pub range_end: String,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            mode: "live".into(),
            range_start: String::new(),
            range_end: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Layouts {
    pub layouts: Vec<Layout>,
    pub active_layout: Option<String>,
}

#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    pub layout_manager: Layouts,
    pub selected_theme: Theme,
    pub custom_theme: Option<Theme>,
    pub main_window: Option<WindowSpec>,
    pub timezone: UserTimezone,
    pub sidebar: Sidebar,
    pub scale_factor: ScaleFactor,
    pub audio_cfg: AudioStream,
    pub trade_fetch_enabled: bool,
    pub size_in_quote_ccy: exchange::SizeUnit,
    pub proxy_cfg: Option<exchange::proxy::Proxy>,
    pub replay: ReplayConfig,
}

impl State {
    pub fn from_parts(
        layout_manager: Layouts,
        selected_theme: Theme,
        custom_theme: Option<Theme>,
        main_window: Option<WindowSpec>,
        timezone: UserTimezone,
        sidebar: Sidebar,
        scale_factor: ScaleFactor,
        audio_cfg: AudioStream,
        trade_fetch_enabled: bool,
        volume_size_unit: exchange::SizeUnit,
        proxy_cfg: Option<exchange::proxy::Proxy>,
        replay: ReplayConfig,
    ) -> Self {
        State {
            layout_manager,
            selected_theme: Theme(selected_theme.0),
            custom_theme: custom_theme.map(|t| Theme(t.0)),
            main_window,
            timezone,
            sidebar,
            scale_factor,
            audio_cfg,
            trade_fetch_enabled,
            size_in_quote_ccy: volume_size_unit,
            proxy_cfg,
            replay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_config_default_is_live_with_empty_strings() {
        let cfg = ReplayConfig::default();
        assert_eq!(cfg.mode, "live");
        assert!(cfg.range_start.is_empty());
        assert!(cfg.range_end.is_empty());
    }

    #[test]
    fn replay_config_serializes_to_json() {
        let cfg = ReplayConfig {
            mode: "replay".into(),
            range_start: "2026-04-10 09:00".into(),
            range_end: "2026-04-10 15:00".into(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(json["mode"], "replay");
        assert_eq!(json["range_start"], "2026-04-10 09:00");
        assert_eq!(json["range_end"], "2026-04-10 15:00");
    }

    #[test]
    fn replay_config_deserializes_from_json() {
        let json =
            r#"{"mode":"replay","range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}"#;
        let cfg: ReplayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, "replay");
        assert_eq!(cfg.range_start, "2026-04-10 09:00");
        assert_eq!(cfg.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn state_without_replay_field_deserializes_with_default_replay() {
        // 後方互換: replay フィールドなしの JSON でも ReplayConfig::default() が使われる
        let json = r#"{"layout_manager":{"layouts":[],"active_layout":null}}"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.replay.mode, "live");
        assert!(state.replay.range_start.is_empty());
    }

    #[test]
    fn state_with_replay_field_deserializes_correctly() {
        let json = r#"{"replay":{"mode":"replay","range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}}"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.replay.mode, "replay");
        assert_eq!(state.replay.range_start, "2026-04-10 09:00");
        assert_eq!(state.replay.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn state_serializes_replay_field() {
        let state = State {
            replay: ReplayConfig {
                mode: "replay".into(),
                range_start: "2026-04-10 09:00".into(),
                range_end: "2026-04-10 15:00".into(),
            },
            ..State::default()
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["replay"]["mode"], "replay");
        assert_eq!(json["replay"]["range_start"], "2026-04-10 09:00");
    }

    // ── ReplayConfig ラウンドトリップ ──

    #[test]
    fn replay_config_roundtrip_serialize_deserialize() {
        let original = ReplayConfig {
            mode: "replay".into(),
            range_start: "2026-04-10 09:00".into(),
            range_end: "2026-04-10 15:00".into(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: ReplayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, original.mode);
        assert_eq!(restored.range_start, original.range_start);
        assert_eq!(restored.range_end, original.range_end);
    }

    #[test]
    fn replay_config_roundtrip_live_mode() {
        let original = ReplayConfig::default();
        let json = serde_json::to_string(&original).unwrap();
        let restored: ReplayConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, "live");
        assert!(restored.range_start.is_empty());
        assert!(restored.range_end.is_empty());
    }

    // ── 部分的・欠損フィールドのデシリアライズ ──

    #[test]
    fn replay_config_empty_object_uses_defaults() {
        let cfg: ReplayConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.mode, "live");
        assert!(cfg.range_start.is_empty());
        assert!(cfg.range_end.is_empty());
    }

    #[test]
    fn replay_config_mode_only_uses_default_for_ranges() {
        let json = r#"{"mode":"replay"}"#;
        let cfg: ReplayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, "replay");
        assert!(cfg.range_start.is_empty());
        assert!(cfg.range_end.is_empty());
    }

    #[test]
    fn replay_config_ranges_only_uses_default_mode() {
        let json = r#"{"range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}"#;
        let cfg: ReplayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, "live");
        assert_eq!(cfg.range_start, "2026-04-10 09:00");
        assert_eq!(cfg.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn replay_config_unknown_mode_preserved() {
        // 未知の mode 値はそのまま保持される（String なので制約なし）
        let json = r#"{"mode":"unknown_value","range_start":"","range_end":""}"#;
        let cfg: ReplayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, "unknown_value");
    }

    #[test]
    fn replay_config_extra_fields_ignored() {
        // 将来追加されるかもしれない未知フィールドを無視
        let json = r#"{"mode":"live","range_start":"","range_end":"","extra_field":42}"#;
        let cfg: ReplayConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.mode, "live");
    }

    // ── State ラウンドトリップ (replay 含む) ──

    #[test]
    fn state_roundtrip_preserves_replay_config() {
        let original = State {
            replay: ReplayConfig {
                mode: "replay".into(),
                range_start: "2026-04-10 09:00".into(),
                range_end: "2026-04-10 15:00".into(),
            },
            ..State::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: State = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.replay.mode, "replay");
        assert_eq!(restored.replay.range_start, "2026-04-10 09:00");
        assert_eq!(restored.replay.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn state_empty_json_deserializes_with_all_defaults() {
        let state: State = serde_json::from_str("{}").unwrap();
        assert_eq!(state.replay.mode, "live");
        assert!(state.replay.range_start.is_empty());
        assert!(state.replay.range_end.is_empty());
    }

    #[test]
    fn state_with_invalid_timezone_falls_back_to_default() {
        // IANA タイムゾーン名等の未対応値が入っても State 全体の deserialize は成功し、
        // timezone は default (Utc) にフォールバックする。
        let json = r#"{"timezone":"Asia/Tokyo"}"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.timezone, UserTimezone::default());
    }

    #[test]
    fn state_with_invalid_timezone_preserves_other_fields() {
        // timezone が壊れていても、replay など他フィールドが巻き添えでロストしない。
        let json = r#"{
            "timezone":"Asia/Tokyo",
            "replay":{
                "mode":"replay",
                "range_start":"2026-04-10 09:00",
                "range_end":"2026-04-10 15:00"
            }
        }"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.timezone, UserTimezone::default());
        assert_eq!(state.replay.mode, "replay");
        assert_eq!(state.replay.range_start, "2026-04-10 09:00");
        assert_eq!(state.replay.range_end, "2026-04-10 15:00");
    }

    #[test]
    fn state_replay_live_with_ranges_preserved() {
        // Live モードでも range が保存されているケース（ユーザーが入力後にモードを戻した場合等）
        let json = r#"{"replay":{"mode":"live","range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}}"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.replay.mode, "live");
        assert_eq!(state.replay.range_start, "2026-04-10 09:00");
        assert_eq!(state.replay.range_end, "2026-04-10 15:00");
    }
}
