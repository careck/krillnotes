// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import type { Note, SchemaInfo } from '../types';

export type NotePosition = 'child' | 'sibling' | 'root';

/**
 * Returns the note types that are valid to create at a given position.
 * - 'root'    : referenceNoteId ignored; types with no allowedParentTypes restriction
 * - 'child'   : referenceNoteId is the intended parent
 * - 'sibling' : referenceNoteId is the intended sibling (its parent becomes the effective parent)
 */
export function getAvailableTypes(
  position: NotePosition,
  referenceNoteId: string | null,
  notes: Note[],
  schemas: Record<string, SchemaInfo>
): string[] {
  const allTypes = Object.keys(schemas);

  if (position === 'root' || referenceNoteId === null) {
    return allTypes.filter(t => (schemas[t]?.allowedParentTypes ?? []).length === 0);
  }

  const referenceNote = notes.find(n => n.id === referenceNoteId);
  if (!referenceNote) return allTypes;

  let effectiveParentType: string | null;
  if (position === 'child') {
    effectiveParentType = referenceNote.schema;
  } else {
    // sibling: effective parent is referenceNote's parent
    const parentNote = notes.find(n => n.id === referenceNote.parentId);
    effectiveParentType = parentNote ? parentNote.schema : null;
  }

  return allTypes.filter(type => {
    const apt = schemas[type]?.allowedParentTypes ?? [];
    if (apt.length > 0) {
      if (effectiveParentType === null) return false;
      if (!apt.includes(effectiveParentType)) return false;
    }
    if (effectiveParentType !== null) {
      const act = schemas[effectiveParentType]?.allowedChildrenTypes ?? [];
      if (act.length > 0 && !act.includes(type)) return false;
    }
    return true;
  });
}
