use iced::{
    Alignment, Border, Color, Element, Length, Shadow,
    widget::{button, center, column, container, row, text, text_input, toggler},
};

// ── テスト ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cycle B1: LoginScreen::new() の初期状態 ───────────────────────────────

    #[test]
    fn new_login_screen_has_empty_user_id_and_password() {
        let screen = LoginScreen::new();
        assert!(screen.user_id.is_empty(), "初期 user_id は空であるべき");
        assert!(screen.password.is_empty(), "初期 password は空であるべき");
    }

    // ── Cycle B2: Message::UserIdChanged でユーザーIDを更新 ───────────────────

    #[test]
    fn user_id_changed_message_updates_user_id() {
        let mut screen = LoginScreen::new();
        screen.update(Message::UserIdChanged("1234567".to_string()));
        assert_eq!(screen.user_id, "1234567");
    }

    // ── Cycle B3: is_demo フィールドのデフォルト値 ────────────────────────────

    #[test]
    fn new_login_screen_defaults_to_production_environment() {
        let screen = LoginScreen::new();
        assert!(!screen.is_demo, "デフォルトは本番環境 (is_demo = false) であるべき");
    }

    // ── Cycle B4: Message::IsDemoProd でデモ/本番を切り替え ──────────────────

    #[test]
    fn is_demo_message_toggles_demo_environment() {
        let mut screen = LoginScreen::new();
        screen.update(Message::IsDemoProd(true));
        assert!(screen.is_demo, "is_demo=true に設定できるべき");
        screen.update(Message::IsDemoProd(false));
        assert!(!screen.is_demo, "is_demo=false に戻せるべき");
    }

    // ── Cycle B5: エラーフィールドと set_error ────────────────────────────────

    #[test]
    fn new_login_screen_has_no_error() {
        let screen = LoginScreen::new();
        assert!(screen.error.is_none(), "初期エラーは None であるべき");
    }

    #[test]
    fn set_error_stores_error_message() {
        let mut screen = LoginScreen::new();
        screen.set_error(Some("ログインに失敗しました".to_string()));
        assert_eq!(screen.error.as_deref(), Some("ログインに失敗しました"));
        screen.set_error(None);
        assert!(screen.error.is_none());
    }

    // ── Cycle B6: Tachibana エラーコードのマッピング ──────────────────────────

    #[test]
    fn error_code_10001_maps_to_credential_error_message() {
        let msg = tachibana_error_message("10001");
        assert!(msg.contains("ユーザID") || msg.contains("パスワード"), "コード 10001 は認証情報エラーを示すべき: {msg}");
    }

    #[test]
    fn unread_notices_error_maps_to_guidance_message() {
        let msg = tachibana_error_message("UNREAD_NOTICES");
        assert!(msg.contains("書面") || msg.contains("未読"), "未読書面エラーは書面確認を促すメッセージであるべき: {msg}");
    }

    #[test]
    fn unknown_error_code_returns_generic_message() {
        let msg = tachibana_error_message("99999");
        assert!(!msg.is_empty(), "未知のコードでも空でないメッセージを返すべき");
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    UserIdChanged(String),
    PasswordChanged(String),
    IsDemoProd(bool),
    LoginSubmit,
}

pub struct LoginScreen {
    pub user_id: String,
    pub password: String,
    /// true = デモ環境, false = 本番環境
    pub is_demo: bool,
    /// 表示するエラーメッセージ
    pub error: Option<String>,
}

/// Tachibana エラーコードを日本語メッセージに変換する。
pub fn tachibana_error_message(code: &str) -> &'static str {
    match code {
        "10001" | "10002" | "10003" => {
            "ユーザIDまたはパスワードが正しくありません。"
        }
        "10004" => "アカウントがロックされています。サポートにお問い合わせください。",
        "UNREAD_NOTICES" => {
            "未読書面があります。立花証券の Web サイトで書面を確認してからログインしてください。"
        }
        _ => "ログインに失敗しました。しばらくしてから再度お試しください。",
    }
}

impl LoginScreen {
    pub fn new() -> Self {
        Self {
            user_id: String::new(),
            password: String::new(),
            is_demo: false,
            error: None,
        }
    }

    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::UserIdChanged(id) => {
                self.user_id = id;
            }
            Message::PasswordChanged(pw) => {
                self.password = pw;
            }
            Message::IsDemoProd(demo) => {
                self.is_demo = demo;
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
        let input_style = |_theme: &iced::Theme, _status| iced::widget::text_input::Style {
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
        };

        let hint = text("ユーザIDとパスワードを入力してください。")
            .size(14)
            .style(|_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(Color::from_rgb(0.3, 0.3, 0.3)),
            });

        // 電話認証案内
        let phone_auth_notice = container(
            text("※ ログイン前に立花証券への電話認証が必要です。")
                .size(12)
                .style(|_theme: &iced::Theme| iced::widget::text::Style {
                    color: Some(Color::from_rgb(0.4, 0.4, 0.9)),
                }),
        )
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(0.2, 0.3, 0.9, 0.08))),
            border: Border {
                color: Color::from_rgba(0.4, 0.4, 0.9, 0.4),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..Default::default()
        })
        .padding([6, 10]);

        let uid_input = text_input("ユーザID", &self.user_id)
            .on_input(Message::UserIdChanged)
            .on_submit(Message::LoginSubmit)
            .padding(14)
            .size(16)
            .style(input_style);

        let pw_input = text_input("パスワードを入力", &self.password)
            .on_input(Message::PasswordChanged)
            .on_submit(Message::LoginSubmit)
            .secure(true)
            .padding(14)
            .size(16)
            .style(input_style);

        // デモ/本番環境切り替えトグル
        let env_label = text(if self.is_demo { "デモ環境" } else { "本番環境" })
            .size(13)
            .style(move |_theme: &iced::Theme| iced::widget::text::Style {
                color: Some(if self.is_demo {
                    Color::from_rgb(0.1, 0.6, 0.4)
                } else {
                    Color::from_rgb(0.3, 0.3, 0.3)
                }),
            });

        let env_toggle = toggler(self.is_demo)
            .on_toggle(Message::IsDemoProd)
            .size(18);

        let env_row = row![env_toggle, env_label]
            .spacing(8)
            .align_y(Alignment::Center);

        // エラーメッセージ
        let error_area: Element<'_, Message> = if let Some(err) = &self.error {
            container(
                text(err.as_str())
                    .size(13)
                    .style(|_theme: &iced::Theme| iced::widget::text::Style {
                        color: Some(Color::from_rgb(0.85, 0.1, 0.1)),
                    }),
            )
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(Color::from_rgba(0.9, 0.1, 0.1, 0.07))),
                border: Border {
                    color: Color::from_rgb(0.85, 0.1, 0.1),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            })
            .padding([8, 12])
            .width(Length::Fill)
            .into()
        } else {
            iced::widget::Space::new().into()
        };

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
            column![
                hint,
                phone_auth_notice,
                uid_input,
                pw_input,
                env_row,
                error_area,
                login_btn,
            ]
            .spacing(16)
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
