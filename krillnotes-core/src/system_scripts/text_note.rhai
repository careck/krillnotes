//! This script is used to create a simple text note

schema("TextNote", #{
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});
