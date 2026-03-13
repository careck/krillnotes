// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { Note, SchemaInfo } from '../types';

export function useHoverTooltip(
  draggedNoteId: string | null,
  notes: Note[],
  schemas: Record<string, SchemaInfo>,
) {
  const [hoveredNoteId, setHoveredNoteId] = useState<string | null>(null);
  const [tooltipAnchorY, setTooltipAnchorY] = useState(0);
  const [hoverHtml, setHoverHtml] = useState<string | null>(null);
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleHoverEnd = useCallback(() => {
    if (hoverTimer.current) clearTimeout(hoverTimer.current);
    hoverTimer.current = null;
    setHoveredNoteId(null);
    setHoverHtml(null);
  }, []);

  const handleHoverStart = useCallback((noteId: string, anchorY: number) => {
    if (draggedNoteId !== null) return;
    if (hoverTimer.current) clearTimeout(hoverTimer.current);
    hoverTimer.current = setTimeout(async () => {
      const noteSchema = notes.find(n => n.id === noteId)?.schema ?? '';
      const schema = schemas[noteSchema] ?? null;
      if (schema?.hasHover) {
        try {
          const html = await invoke<string | null>('get_note_hover', { noteId });
          setHoverHtml(html);
        } catch {
          setHoverHtml(null);
        }
      } else {
        setHoverHtml(null);
      }
      setHoveredNoteId(noteId);
      setTooltipAnchorY(anchorY);
    }, 600);
  }, [draggedNoteId, notes, schemas]);

  // Dismiss tooltip immediately when a drag starts
  useEffect(() => {
    if (draggedNoteId !== null) handleHoverEnd();
  }, [draggedNoteId, handleHoverEnd]);

  // Cancel pending timer on unmount to prevent state updates on a dead component
  useEffect(() => {
    return () => {
      if (hoverTimer.current) clearTimeout(hoverTimer.current);
    };
  }, []);

  return {
    hoveredNoteId,
    tooltipAnchorY,
    hoverHtml,
    handleHoverStart,
    handleHoverEnd,
  };
}
