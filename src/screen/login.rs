use iced::{
    Alignment, Border, Color, Element, Length, Shadow,
    widget::{button, center, column, container, row, text, text_input},
};

#[derive(Debug, Clone)]
pub enum Message {
    PasswordChanged(String),
    LoginSubmit,
}

pub struct LoginScreen {
    pub password: String,
}

impl LoginScreen {
    pub fn new() -> Self {
        Self {
            password: String::new(),
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::PasswordChanged(pw) => {
                self.password = pw;
            }
            Message::LoginSubmit => {
                // handled by parent
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        // --- Left panel ---
        let title = text("kabu STATION")
            .size(36)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..Default::default()
            })
            .style(|_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(Color::WHITE),
            });

        let title_row = container(title)
            .align_x(Alignment::Center)
            .width(Length::Fill)
            .padding(24);

        let alert_box = container(
            column![
                text("4/6【重要】約定後の画面反映遅延について")
                    .size(13)
                    .style(|_theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(Color::from_rgb(0.9, 0.2, 0.2)),
                    }),
                text("kabuステーションにおいて約定後の画面反映に時間を要する場合があります。")
                    .size(13)
                    .style(|_theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(Color::from_rgb(0.9, 0.2, 0.2)),
                    }),
                text("現在、本事象の解消に向けて対応中です。")
                    .size(13)
                    .style(|_theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(Color::from_rgb(0.9, 0.2, 0.2)),
                    }),
            ]
            .spacing(4),
        )
        .style(|_theme: &iced::Theme| container::Style {
            border: Border {
                color: Color::from_rgb(0.9, 0.2, 0.2),
                width: 1.5,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .padding(12);

        let news_text = text("【次回バージョンアップのお知らせ】")
            .size(13)
            .style(|_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(Color::from_rgb(1.0, 1.0, 0.0)),
            });

        let news_detail = text("2026/4/9 (木) 夜  Ver.5.39.1.0をリリース予定")
            .size(13)
            .style(|_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(Color::WHITE),
            });

        let left_panel = column![title_row, alert_box, news_text, news_detail]
            .spacing(16)
            .padding(40)
            .width(Length::FillPortion(3));

        // --- Right card (login form) ---
        let hint = text("パスワードを入力してログインしてください。")
            .size(14)
            .style(|_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(Color::from_rgb(0.3, 0.3, 0.3)),
            });

        let pw_input = text_input("パスワードを入力", &self.password)
            .on_input(Message::PasswordChanged)
            .on_submit(Message::LoginSubmit)
            .secure(true)
            .padding(14)
            .size(16)
            .style(|_theme: &iced::Theme, _status| iced::widget::text_input::Style {
                background: iced::Background::Color(Color::WHITE),
                border: Border {
                    color: Color::from_rgb(0.8, 0.5, 0.0),
                    width: 1.5,
                    radius: 6.0.into(),
                },
                icon: Color::from_rgb(0.5, 0.5, 0.5),
                placeholder: Color::from_rgb(0.6, 0.6, 0.6),
                value: Color::BLACK,
                selection: Color::from_rgb(1.0, 0.6, 0.0),
            });

        let login_btn = button(
            center(
                text("ログイン")
                    .size(20)
                    .font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..Default::default()
                    })
                    .style(|_theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(Color::WHITE),
                    }),
            )
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(56)
        .style(|_theme: &iced::Theme, status| iced::widget::button::Style {
            background: Some(iced::Background::Color(match status {
                iced::widget::button::Status::Hovered => Color::from_rgb(1.0, 0.5, 0.1),
                iced::widget::button::Status::Pressed => Color::from_rgb(0.8, 0.3, 0.0),
                _ => Color::from_rgb(1.0, 0.4, 0.0),
            })),
            border: Border {
                radius: 28.0.into(),
                ..Default::default()
            },
            text_color: Color::WHITE,
            ..Default::default()
        })
        .on_press(Message::LoginSubmit);

        let form = container(
            column![hint, pw_input, login_btn]
                .spacing(20)
                .align_x(Alignment::Center),
        )
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgb(0.95, 0.95, 0.95))),
            border: Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color { a: 0.3, ..Color::BLACK },
                offset: iced::Vector { x: 0.0, y: 4.0 },
                blur_radius: 16.0,
            },
            ..Default::default()
        })
        .padding(40)
        .width(Length::FillPortion(2));

        let right_panel = center(form).width(Length::FillPortion(2));

        // --- Root layout ---
        let content = row![left_panel, right_panel]
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .height(Length::Fill);

        container(content)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.05, 0.12, 0.22))),
                ..Default::default()
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
