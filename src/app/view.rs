use crate::modal::dashboard_modal;
use crate::replay::{ReplayMessage, ReplayUserMessage};
use crate::screen;
use crate::screen::dashboard::Dashboard;
use crate::style;
use crate::{Flowsurface, Message};
use data::sidebar;
use iced::{
    Alignment, Element, padding,
    widget::{button, row, text, text_input},
};

impl Flowsurface {
    pub(crate) fn view_replay_header(&self) -> Element<'_, Message> {
        let time_display = text(self.replay.format_current_time(self.timezone))
            .font(style::AZERET_MONO)
            .size(12);

        let is_replay = self.replay.is_replay();
        let is_playing = self.replay.is_playing();

        let is_highlighted = if is_replay { is_playing } else { true };
        let mode_label = if is_replay {
            if is_playing { "● REPLAY" } else { "REPLAY" }
        } else {
            "● LIVE"
        };
        let mode_toggle = button(text(mode_label).size(11))
            .on_press(Message::Replay(ReplayMessage::User(
                ReplayUserMessage::ToggleMode,
            )))
            .style(move |theme, status| {
                style::button::bordered_toggle_highlighted(theme, status, is_replay, is_highlighted)
            })
            .padding(padding::all(2).left(6).right(6));

        let mut header = row![time_display, mode_toggle];

        if is_replay {
            let start_input = text_input("Start", self.replay.range_input_start())
                .size(11)
                .on_input(|s| {
                    Message::Replay(ReplayMessage::User(ReplayUserMessage::StartTimeChanged(s)))
                });
            let end_input = text_input("End", self.replay.range_input_end())
                .size(11)
                .on_input(|s| {
                    Message::Replay(ReplayMessage::User(ReplayUserMessage::EndTimeChanged(s)))
                });

            let is_loading = self.replay.is_loading();
            let is_playing = self.replay.is_playing();
            let is_paused = self.replay.is_paused();
            let has_clock = self.replay.has_clock();
            let is_at_end = self.replay.is_at_end();

            let play_pause_label = if is_playing { "\u{23F8}" } else { "\u{25B6}" };
            let mut play_pause_btn =
                button(text(play_pause_label).size(12)).padding(padding::all(2).left(4).right(4));
            if !is_loading {
                play_pause_btn = play_pause_btn.on_press(if is_playing {
                    Message::Replay(ReplayMessage::User(ReplayUserMessage::Pause))
                } else if is_paused && !is_at_end {
                    Message::Replay(ReplayMessage::User(ReplayUserMessage::Resume))
                } else {
                    Message::Replay(ReplayMessage::User(ReplayUserMessage::Play))
                });
            }

            let mut step_back_btn =
                button(text("\u{23EE}").size(12)).padding(padding::all(2).left(4).right(4));
            if has_clock && !is_loading {
                step_back_btn = step_back_btn.on_press(Message::Replay(ReplayMessage::User(
                    ReplayUserMessage::StepBackward,
                )));
            }

            let mut step_fwd_btn =
                button(text("\u{23ED}").size(12)).padding(padding::all(2).left(4).right(4));
            if !is_loading {
                step_fwd_btn = step_fwd_btn.on_press(Message::Replay(ReplayMessage::User(
                    ReplayUserMessage::StepForward,
                )));
            }

            let speed_label = self.replay.speed_label();
            let mut speed_btn =
                button(text(speed_label).size(11)).padding(padding::all(2).left(4).right(4));
            if has_clock && !is_loading {
                speed_btn = speed_btn.on_press(Message::Replay(ReplayMessage::User(
                    ReplayUserMessage::CycleSpeed,
                )));
            }
            let controls = row![step_back_btn, play_pause_btn, step_fwd_btn, speed_btn].spacing(4);

            header = header
                .push(start_input.width(140))
                .push(text("~").size(11))
                .push(end_input.width(140))
                .push(controls);

            if is_loading {
                header = header.push(text("Loading...").size(11));
            }
        }

        header
            .spacing(8)
            .padding(padding::all(4))
            .align_y(Alignment::Center)
            .into()
    }

    pub(crate) fn view_with_modal<'a>(
        &'a self,
        base: Element<'a, Message>,
        dashboard: &'a Dashboard,
        menu: sidebar::Menu,
    ) -> Element<'a, Message> {
        let sidebar_pos = self.sidebar.position();

        match menu {
            sidebar::Menu::Settings => self.build_settings_modal_content(sidebar_pos, base),
            sidebar::Menu::Layout => self.build_layout_modal_content(sidebar_pos, base, dashboard),
            sidebar::Menu::Audio => {
                let (align_x, padding_val) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).top(76)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).top(76)),
                };
                let trade_streams_list = dashboard.streams.trade_streams(None);
                dashboard_modal(
                    base,
                    self.audio_stream
                        .view(trade_streams_list)
                        .map(Message::AudioStream),
                    Message::Sidebar(screen::dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding_val,
                    Alignment::Start,
                    align_x,
                )
            }
            sidebar::Menu::ThemeEditor => {
                let (align_x, padding_val) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };
                dashboard_modal(
                    base,
                    self.theme_editor
                        .view(&self.theme.0)
                        .map(Message::ThemeEditor),
                    Message::Sidebar(screen::dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding_val,
                    Alignment::End,
                    align_x,
                )
            }
            sidebar::Menu::Network => {
                let (align_x, padding_val) = match sidebar_pos {
                    sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
                    sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
                };
                dashboard_modal(
                    base,
                    self.network.view().map(Message::NetworkManager),
                    Message::Sidebar(screen::dashboard::sidebar::Message::ToggleSidebarMenu(None)),
                    padding_val,
                    Alignment::End,
                    align_x,
                )
            }
            sidebar::Menu::Order => base,
        }
    }
}
