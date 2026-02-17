use iced::widget::{button, row, text};
use iced::Element;

#[derive(Debug, Clone)]
pub enum MenuMessage {
    FileNew,
    FileOpen,
    EditAddNote,
    EditDeleteNote,
    HelpAbout,
}

pub fn menu_bar<'a>() -> Element<'a, MenuMessage> {
    row![
        button(text("File")).on_press(MenuMessage::FileNew),
        button(text("Edit")).on_press(MenuMessage::EditAddNote),
        button(text("View")),
        button(text("Help")).on_press(MenuMessage::HelpAbout),
    ]
    .spacing(10)
    .into()
}
