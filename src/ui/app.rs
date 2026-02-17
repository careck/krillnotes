use crate::ui::menu::{menu_bar, MenuMessage};
use iced::{
    widget::{column, container, text},
    Application, Command, Element, Length, Settings, Theme,
};

pub struct KrillnotesApp {
    status_message: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Menu(MenuMessage),
}

impl Application for KrillnotesApp {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        (
            Self {
                status_message: "Welcome to Krillnotes".to_string(),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        "Krillnotes".to_string()
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::Menu(menu_msg) => {
                self.status_message = match menu_msg {
                    MenuMessage::FileNew => "File > New clicked".to_string(),
                    MenuMessage::FileOpen => "File > Open clicked".to_string(),
                    MenuMessage::EditAddNote => "Edit > Add Note clicked".to_string(),
                    MenuMessage::EditDeleteNote => "Edit > Delete Note clicked".to_string(),
                    MenuMessage::HelpAbout => "Help > About clicked".to_string(),
                };
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<Self::Message> {
        let menu = menu_bar().map(Message::Menu);

        let content = container(text(&self.status_message))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y();

        column![menu, content].into()
    }

    fn theme(&self) -> Self::Theme {
        Theme::Dark
    }
}

pub fn run() -> iced::Result {
    KrillnotesApp::run(Settings::default())
}
