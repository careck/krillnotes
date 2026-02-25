// @name: Text Note
// @description: A simple text note with a body field.

schema("TextNote", #{
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});

// Example tree action: sort a TextNote's children alphabetically by title.
// Right-click any TextNote in the tree and choose "Sort Children A→Z".
add_tree_action("Sort Children A→Z", ["TextNote"], |note| {
    let children = get_children(note.id);
    children.sort_by(|a, b| a.title <= b.title);
    children.map(|c| c.id)
});

// Example tree action: add a new child TextNote with a placeholder title.
add_tree_action("Add Child Note", ["TextNote"], |note| {
    let child = create_note(note.id, "TextNote");
    child.title = "New note";
    update_note(child);
});
