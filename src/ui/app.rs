use iced::{
    widget::{column, text},
    Application, Command, Element, Settings, Theme,
};

pub struct KrillnotesApp {
    // Will add fields later
}

#[derive(Debug, Clone)]
pub enum Message {
    // Will add variants later
}

impl Application for KrillnotesApp {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        (Self {}, Command::none())
    }

    fn title(&self) -> String {
        "Krillnotes".to_string()
    }

    fn update(&mut self, _message: Self::Message) -> Command<Self::Message> {
        Command::none()
    }

    fn view(&self) -> Element<Self::Message> {
        column![text("Krillnotes").size(32),].into()
    }

    fn theme(&self) -> Self::Theme {
        Theme::Dark
    }
}

pub fn run() -> iced::Result {
    KrillnotesApp::run(Settings::default())
}
