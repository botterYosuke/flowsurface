// Library target — exposes internal modules for integration tests in tests/rust/.
// The binary (main.rs) re-declares the same modules independently; this is the
// standard Rust "library + binary in one package" pattern.

pub(crate) use iced::Element;
pub(crate) use iced::widget::tooltip::Position as TooltipPosition;
pub(crate) use screen::dashboard;
pub(crate) use widget::tooltip;

pub mod audio;
pub mod chart;
pub mod connector;
pub mod headless;
pub mod layout;
pub mod logger;
pub mod modal;
pub mod narrative;
pub mod notify;
pub mod replay;
pub mod replay_api;
pub mod screen;
pub mod style;
pub mod version;
pub mod widget;
pub mod window;
