// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

// @name: Text Note
// @description: A simple text note with a body field.

schema("TextNote", #{
    version: 1,
    fields: [
        #{ name: "body", type: "textarea", required: false },
    ]
});
