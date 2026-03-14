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
    if (!selectedNote) {
      setSchemaInfo(emptySchemaInfo);
      setViews([]);
      setViewHtml({});
      setActiveTab('fields');
      return;
    }

    invoke<SchemaInfo>('get_schema_fields', { schema: selectedNote.schema })
      .then(info => {
        setSchemaInfo(info);
        schemaLoadedRef.current = true;
        onSchemaLoadedRef.current(info);
        // Fetch registered views for this note type
        invoke<ViewInfo[]>('get_views_for_type', { schemaName: selectedNote.schema })
          .then(v => {
            setViews(v);
            setViewHtml({});
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

  // Effect 4: Render view HTML when the active tab changes
  useEffect(() => {
    if (activeTab !== 'fields' && selectedNote && !isEditing) {
      invoke<string>('render_view', {
        noteId: selectedNote.id,
        viewLabel: activeTab,
      }).then(html => {
        setViewHtml(prev => ({ ...prev, [activeTab]: html }));
      }).catch(err => {
        console.error('Failed to render view:', err);
      });
    }
  }, [activeTab, selectedNote?.id, isEditing]);

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
