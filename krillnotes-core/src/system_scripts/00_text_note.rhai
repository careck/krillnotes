// @name: Text Note
// @description: A simple text note with a body field.

schema("TextNote", #{
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});
