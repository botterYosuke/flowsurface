use crate::connector;
use crate::modal::{dashboard_modal, main_dialog_modal};
use crate::screen::{self, dashboard};
use crate::split_column;
use crate::style;
use crate::version;
use crate::widget::{confirm_dialog_container, tooltip};
use crate::{Flowsurface, Message};
use data::config::theme::default_theme;
use data::sidebar;
use iced::{
    Alignment, Element, padding,
    widget::tooltip::Position as TooltipPosition,
    widget::{button, column, container, pick_list, row, rule, scrollable, text},
};
use std::borrow::Cow;

impl Flowsurface {
    /// Settings メニューのモーダルコンテンツを構築する。
    /// `view_with_modal` の Settings アームから呼ばれる。
    pub(crate) fn build_settings_modal_content<'a>(
        &'a self,
        sidebar_pos: sidebar::Position,
        base: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let theme_picklist = {
            let mut themes: Vec<iced::Theme> = iced_core::Theme::ALL.to_vec();
            let default = iced_core::Theme::Custom(default_theme().into());
            themes.push(default);
            if let Some(custom_theme) = &self.theme_editor.custom_theme {
                themes.push(custom_theme.clone());
            }
            pick_list(themes, Some(self.theme.0.clone()), |theme| {
                Message::ThemeSelected(theme)
            })
        };

        let toggle_theme_editor = button(text("Theme editor")).on_press(Message::Sidebar(
            dashboard::sidebar::Message::ToggleSidebarMenu(Some(sidebar::Menu::ThemeEditor)),
        ));

        let toggle_network_editor = button(text("Network")).on_press(Message::Sidebar(
            dashboard::sidebar::Message::ToggleSidebarMenu(Some(sidebar::Menu::Network)),
        ));

        let timezone_picklist = pick_list(
            [data::UserTimezone::Utc, data::UserTimezone::Local],
            Some(self.timezone),
            Message::SetTimezone,
        );

        let size_in_quote_currency_checkbox = {
            let is_active = match self.volume_size_unit {
                exchange::SizeUnit::Quote => true,
                exchange::SizeUnit::Base => false,
            };
            let checkbox = iced::widget::checkbox(is_active)
                .label("Size in quote currency")
                .on_toggle(|checked| {
                    let on_dialog_confirm = Message::ApplyVolumeSizeUnit(if checked {
                        exchange::SizeUnit::Quote
                    } else {
                        exchange::SizeUnit::Base
                    });
                    let confirm_dialog = screen::ConfirmDialog::new(
                        "Changing size display currency requires application restart".to_string(),
                        Box::new(on_dialog_confirm.clone()),
                    )
                    .with_confirm_btn_text("Restart now".to_string());
                    Message::ToggleDialogModal(Some(confirm_dialog))
                });
            tooltip(
                checkbox,
                Some(
                    "Display sizes/volumes in quote currency (USD)\nHas no effect on inverse perps or open interest",
                ),
                TooltipPosition::Top,
            )
        };

        let sidebar_pos_picklist = pick_list(
            [sidebar::Position::Left, sidebar::Position::Right],
            Some(sidebar_pos),
            |pos| Message::Sidebar(dashboard::sidebar::Message::SetSidebarPosition(pos)),
        );

        let scale_factor = {
            let current_value: f32 = self.ui_scale_factor.into();
            let decrease_btn = if current_value > data::config::MIN_SCALE {
                button(text("-"))
                    .on_press(Message::ScaleFactorChanged((current_value - 0.1).into()))
            } else {
                button(text("-"))
            };
            let increase_btn = if current_value < data::config::MAX_SCALE {
                button(text("+"))
                    .on_press(Message::ScaleFactorChanged((current_value + 0.1).into()))
            } else {
                button(text("+"))
            };
            container(
                row![
                    decrease_btn,
                    text(format!("{:.0}%", current_value * 100.0)).size(14),
                    increase_btn,
                ]
                .align_y(Alignment::Center)
                .spacing(8)
                .padding(4),
            )
            .style(style::modal_container)
        };

        let trade_fetch_checkbox = {
            let is_active = connector::fetcher::is_trade_fetch_enabled();
            let checkbox = iced::widget::checkbox(is_active)
                .label("Fetch trades (Binance)")
                .on_toggle(|checked| {
                    if checked {
                        let confirm_dialog = screen::ConfirmDialog::new(
                            "This might be unreliable and take some time to complete. Proceed?"
                                .to_string(),
                            Box::new(Message::ToggleTradeFetch(true)),
                        );
                        Message::ToggleDialogModal(Some(confirm_dialog))
                    } else {
                        Message::ToggleTradeFetch(false)
                    }
                });
            tooltip(
                checkbox,
                Some("Try to fetch trades for footprint charts"),
                TooltipPosition::Top,
            )
        };

        let open_data_folder = {
            let btn = button(text("Open data folder")).on_press(Message::DataFolderRequested);
            tooltip(
                btn,
                Some("Open the folder where the data & config is stored"),
                TooltipPosition::Top,
            )
        };

        let version_info: Element<'_, Message> = {
            let (version_label, commit_label) = version::app_build_version_parts();
            let github_link_button = button(text(version_label).size(13))
                .padding(0)
                .style(style::button::text_link)
                .on_press(Message::OpenUrlRequested(Cow::Borrowed(
                    version::GITHUB_REPOSITORY_URL,
                )));
            let github_button: Element<'_, Message> = iced::widget::tooltip(
                github_link_button,
                container(
                    row![
                        text("GitHub"),
                        style::icon_text(style::Icon::ExternalLink, 12),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                )
                .style(style::tooltip)
                .padding(8),
                TooltipPosition::Top,
            )
            .into();

            if let (Some(commit_label), Some(commit_url)) =
                (commit_label, version::build_commit_url())
            {
                let commit_button = button(text(commit_label).size(11))
                    .padding(0)
                    .style(style::button::text_link_secondary)
                    .on_press(Message::OpenUrlRequested(Cow::Owned(commit_url)));
                column![github_button, commit_button]
                    .spacing(2)
                    .align_x(Alignment::End)
                    .into()
            } else {
                github_button
            }
        };

        let footer = column![
            container(version_info)
                .width(iced::Length::Fill)
                .align_x(Alignment::End),
        ]
        .spacing(8);

        let column_content = split_column![
            column![open_data_folder,].spacing(8),
            column![text("Sidebar position").size(14), sidebar_pos_picklist,].spacing(12),
            column![text("Time zone").size(14), timezone_picklist,].spacing(12),
            column![text("Market data").size(14), size_in_quote_currency_checkbox,].spacing(12),
            column![text("Theme").size(14), theme_picklist,].spacing(12),
            column![text("Interface scale").size(14), scale_factor,].spacing(12),
            column![
                text("Experimental").size(14),
                column![trade_fetch_checkbox, toggle_theme_editor, toggle_network_editor]
                    .spacing(8),
            ]
            .spacing(12),
            footer,
            ; spacing = 16, align_x = Alignment::Start
        ];

        let content = scrollable::Scrollable::with_direction(
            column_content,
            scrollable::Direction::Vertical(
                scrollable::Scrollbar::new().width(8).scroller_width(6),
            ),
        );

        let settings_modal = container(content)
            .align_x(Alignment::Start)
            .max_width(240)
            .padding(24)
            .style(style::dashboard_modal);

        let (align_x, padding_val) = match sidebar_pos {
            sidebar::Position::Left => (Alignment::Start, padding::left(44).bottom(4)),
            sidebar::Position::Right => (Alignment::End, padding::right(44).bottom(4)),
        };

        let base_content = dashboard_modal(
            base,
            settings_modal,
            Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
            padding_val,
            Alignment::End,
            align_x,
        );

        if let Some(dialog) = &self.confirm_dialog {
            let dialog_content =
                confirm_dialog_container(dialog.clone(), Message::ToggleDialogModal(None));
            main_dialog_modal(
                base_content,
                dialog_content,
                Message::ToggleDialogModal(None),
            )
        } else {
            base_content
        }
    }

    /// Layout メニューのモーダルコンテンツを構築する。
    /// `view_with_modal` の Layout アームから呼ばれる。
    pub(crate) fn build_layout_modal_content<'a>(
        &'a self,
        sidebar_pos: sidebar::Position,
        base: Element<'a, Message>,
        dashboard: &'a screen::dashboard::Dashboard,
    ) -> Element<'a, Message> {
        let main_window = self.main_window.id;

        let manage_pane = if let Some((window_id, pane_id)) = dashboard.focus {
            let selected_pane_str =
                if let Some(state) = dashboard.get_pane(main_window, window_id, pane_id) {
                    let link_group_name: String = state
                        .link_group
                        .as_ref()
                        .map_or_else(String::new, |g| " - Group ".to_string() + &g.to_string());
                    state.content.to_string() + &link_group_name
                } else {
                    "".to_string()
                };

            let is_main_window = window_id == main_window;

            let reset_pane_button = {
                let btn =
                    button(text("Reset").align_x(Alignment::Center)).width(iced::Length::Fill);
                if is_main_window {
                    let dashboard_msg = Message::Dashboard {
                        layout_id: None,
                        event: dashboard::Message::Pane(
                            main_window,
                            dashboard::pane::Message::ReplacePane(pane_id),
                        ),
                    };
                    btn.on_press(dashboard_msg)
                } else {
                    btn
                }
            };
            let split_pane_button = {
                let btn =
                    button(text("Split").align_x(Alignment::Center)).width(iced::Length::Fill);
                if is_main_window {
                    let dashboard_msg = Message::Dashboard {
                        layout_id: None,
                        event: dashboard::Message::Pane(
                            main_window,
                            dashboard::pane::Message::SplitPane(
                                iced::widget::pane_grid::Axis::Horizontal,
                                pane_id,
                            ),
                        ),
                    };
                    btn.on_press(dashboard_msg)
                } else {
                    btn
                }
            };

            column![
                text(selected_pane_str),
                row![
                    tooltip(
                        reset_pane_button,
                        if is_main_window {
                            Some("Reset selected pane")
                        } else {
                            None
                        },
                        TooltipPosition::Top,
                    ),
                    tooltip(
                        split_pane_button,
                        if is_main_window {
                            Some("Split selected pane horizontally")
                        } else {
                            None
                        },
                        TooltipPosition::Top,
                    ),
                ]
                .spacing(8)
            ]
            .spacing(8)
        } else {
            column![text("No pane selected"),].spacing(8)
        };

        let manage_layout_modal = {
            let col = column![
                manage_pane,
                rule::horizontal(1.0).style(style::split_ruler),
                self.layout_manager.view().map(Message::Layouts)
            ];
            container(col.align_x(Alignment::Center).spacing(20))
                .width(260)
                .padding(24)
                .style(style::dashboard_modal)
        };

        let (align_x, padding_val) = match sidebar_pos {
            sidebar::Position::Left => (Alignment::Start, padding::left(44).top(40)),
            sidebar::Position::Right => (Alignment::End, padding::right(44).top(40)),
        };

        dashboard_modal(
            base,
            manage_layout_modal,
            Message::Sidebar(dashboard::sidebar::Message::ToggleSidebarMenu(None)),
            padding_val,
            Alignment::Start,
            align_x,
        )
    }
}
