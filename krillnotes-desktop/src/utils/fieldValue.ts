// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import type { FieldValue, FieldType } from '../types';

/** Return a sensible empty/default value for a given field type string. */
export function defaultValueForFieldType(fieldType: FieldType): FieldValue {
  switch (fieldType) {
    case 'boolean': return { Boolean: false };
    case 'number':  return { Number: 0 };
    case 'rating':  return { Number: 0 };
    case 'date':      return { Date: null };
    case 'email':     return { Email: '' };
    case 'note_link': return { NoteLink: null };
    case 'file':      return { File: null };
    default:          return { Text: '' }; // covers 'text', 'textarea', 'select'
  }
}

/** Check whether a FieldValue is effectively "empty" (blank text, null date, etc.). */
export function isEmptyFieldValue(value: FieldValue): boolean {
  if ('Text' in value)     return value.Text === '';
  if ('Email' in value)    return value.Email === '';
  if ('Date' in value)     return value.Date === null;
  if ('NoteLink' in value) return value.NoteLink === null;
  if ('File' in value)     return value.File === null;
  return false; // Number and Boolean are never empty
}
