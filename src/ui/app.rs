use crate::ui::menu::{menu_bar, MenuMessage};
use iced::widget::{column, container, text};
use iced::{Element, Length};

#[derive(Debug, Default)]
pub struct AppState {
    status_message: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Menu(MenuMessage),
}

fn update(state: &mut AppState, message: Message) {
    match message {
        Message::Menu(menu_msg) => {
            state.status_message = match menu_msg {
                MenuMessage::FileNew => "File > New clicked".to_string(),
                MenuMessage::FileOpen => "File > Open clicked".to_string(),
                MenuMessage::EditAddNote => "Edit > Add Note clicked".to_string(),
                MenuMessage::EditDeleteNote => "Edit > Delete Note clicked".to_string(),
                MenuMessage::HelpAbout => "Help > About clicked".to_string(),
            };
        }
    }
}

fn view(state: &AppState) -> Element<Message> {
    let menu = menu_bar().map(Message::Menu);

    let content = container(text(&state.status_message))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    column![menu, content].into()
}

pub fn run() -> iced::Result {
    iced::application("Krillnotes", update, view).run()
}
