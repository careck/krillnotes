use iced::widget::button;
use iced::Element;
use iced_aw::menu::{Item, Menu, MenuBar};

#[derive(Debug, Clone)]
pub enum MenuMessage {
    FileNew,
    FileOpen,
    EditAddNote,
    EditDeleteNote,
    HelpAbout,
}

pub fn menu_bar<'a>() -> Element<'a, MenuMessage> {
    // File menu items
    let file_items = vec![
        Item::new(button("New").on_press(MenuMessage::FileNew)),
        Item::new(button("Open").on_press(MenuMessage::FileOpen)),
    ];

    // Edit menu items
    let edit_items = vec![
        Item::new(button("Add Note").on_press(MenuMessage::EditAddNote)),
        Item::new(button("Delete Note").on_press(MenuMessage::EditDeleteNote)),
    ];

    // Help menu items
    let help_items = vec![
        Item::new(button("About").on_press(MenuMessage::HelpAbout)),
    ];

    // Create menus
    let file_menu = Item::with_menu(button("File"), Menu::new(file_items));
    let edit_menu = Item::with_menu(button("Edit"), Menu::new(edit_items));
    let help_menu = Item::with_menu(button("Help"), Menu::new(help_items));

    // Create menu bar
    MenuBar::new(vec![file_menu, edit_menu, help_menu]).into()
}
