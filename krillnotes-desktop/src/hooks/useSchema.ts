// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, SchemaInfo, ViewInfo } from '../types';

const emptySchemaInfo: SchemaInfo = {
  fields: [],
  titleCanView: true,
  titleCanEdit: true,
  childrenSort: 'none',
  allowedParentSchemas: [],
  allowedChildrenSchemas: [],
  isLeaf: false,
  showCheckbox: false,
  hasViews: false,
  hasHover: false,
  allowAttachments: false,
  attachmentTypes: [],
  fieldGroups: [],
};

export function useSchema(
  selectedNote: Note | null,
  isEditing: boolean,
  onSchemaLoaded: (schema: SchemaInfo) => void,
) {
  const [schemaInfo, setSchemaInfo] = useState<SchemaInfo>({
    fields: [],
    titleCanView: true,
    titleCanEdit: true,
    childrenSort: 'none',
    allowedParentSchemas: [],
    allowedChildrenSchemas: [],
    isLeaf: false,
    showCheckbox: false,
    hasViews: false,
    hasHover: false,
    allowAttachments: false,
    attachmentTypes: [],
    fieldGroups: [],
  });
  const [views, setViews] = useState<ViewInfo[]>([]);
  const [activeTab, setActiveTab] = useState<string>('fields');
  const [viewHtml, setViewHtml] = useState<Record<string, string>>({});
  const [previousTab, setPreviousTab] = useState<string | null>(null);

  // Tracks whether the schema fetch for the current note has already resolved.
  // Used by the requestEditMode effect to enter edit mode immediately when the
  // schema is already available, rather than waiting for a .then() that already ran.
  const schemaLoadedRef = useRef(false);

  // Tracks which note ID the current `views` array belongs to. Set to null at
  // the top of Effect 1 (synchronously) and restored after the async view fetch.
  // Effect 4 checks this to avoid calling render_view with stale views/tab from
  // a previous note — React effects fire in the same cycle, so state updates
  // from Effect 1 aren't visible to Effect 4 yet.
  const viewsForNoteRef = useRef<string | null>(null);

  // Stable ref so the schema effect can call the callback without listing it as a
  // dependency (which would re-run the fetch on every render).
  const onSchemaLoadedRef = useRef(onSchemaLoaded);
  onSchemaLoadedRef.current = onSchemaLoaded;

  // Effect 1: Schema & views fetch — re-runs when the selected note changes.
  // Intentionally has no cancellation flag to match the original InfoPanel behaviour:
  // stale-note callbacks are harmless because they only call setState setters, and
  // the most-recently-resolved fetch always wins. Adding a cancelled flag introduced
  // a regression in React StrictMode (double-invocation cancels the first fetch before
  // it can chain to get_views_for_type), leaving views=[] on first load.
  useEffect(() => {
    schemaLoadedRef.current = false;
    // Invalidate immediately so Effect 4 (same render cycle) won't call
    // render_view with stale views/tab from the previous note.
    viewsForNoteRef.current = null;
    setViewHtml({});
    setActiveTab('fields');
    if (!selectedNote) {
      setSchemaInfo(emptySchemaInfo);
      setViews([]);
      return;
    }

    const noteId = selectedNote.id;
    invoke<SchemaInfo>('get_schema_fields', { schema: selectedNote.schema })
      .then(info => {
        setSchemaInfo(info);
        schemaLoadedRef.current = true;
        onSchemaLoadedRef.current(info);
        // Fetch registered views for this note type
        invoke<ViewInfo[]>('get_views_for_type', { schemaName: selectedNote.schema })
          .then(v => {
            setViews(v);
            viewsForNoteRef.current = noteId;
            // Default tab: first displayFirst view, or first view, or "fields"
            const sorted = [...v].sort((a, b) =>
              (b.displayFirst ? 1 : 0) - (a.displayFirst ? 1 : 0)
            );
            setActiveTab(sorted.length > 0 ? sorted[0].label : 'fields');
          })
          .catch(err => {
            console.error('Failed to fetch views:', err);
            setViews([]);
            setActiveTab('fields');
          });
      })
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaInfo(emptySchemaInfo);
        setViews([]);
        setViewHtml({});
        setActiveTab('fields');
        schemaLoadedRef.current = true;
        onSchemaLoadedRef.current(emptySchemaInfo);
      });
  }, [selectedNote?.id]);

  // Effect 4: Render view HTML when the active tab changes.
  // Guard: viewsForNoteRef is set to null synchronously at the top of Effect 1
  // and only restored after the async view fetch completes for the new note.
  // This prevents render_view calls with stale views/tab during the transition.
  useEffect(() => {
    if (activeTab !== 'fields' && selectedNote && !isEditing
        && viewsForNoteRef.current === selectedNote.id) {
      invoke<string>('render_view', {
        noteId: selectedNote.id,
        viewLabel: activeTab,
      }).then(html => {
        setViewHtml(prev => ({ ...prev, [activeTab]: html }));
      }).catch(err => {
        console.error('Failed to render view:', err);
      });
    }
  }, [activeTab, selectedNote?.id, isEditing, views]);

  return {
    schemaInfo,
    views,
    activeTab,
    setActiveTab,
    viewHtml,
    setViewHtml,
    previousTab,
    setPreviousTab,
    schemaLoadedRef,
  };
}
