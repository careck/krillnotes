// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { UndoResult } from '../types';

// Note: loadNotes should ideally be wrapped in useCallback in the consumer (WorkspaceView)
// so that performUndo/performRedo remain stable across renders. Currently loadNotes is a
// plain async function, so these callbacks will re-create on every render.
export function useUndoRedo(
  loadNotes: () => Promise<unknown>,
  setSelectedNoteId: (id: string | null) => void,
) {
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const [noteRefreshSignal, setNoteRefreshSignal] = useState(0);
  // Tracks whether a note-creation undo group is currently open.
  // Set to true just before create_note_with_type; cleared when edit mode ends.
  const pendingUndoGroupRef = useRef(false);

  const refreshUndoState = useCallback(async () => {
    const [u, r] = await Promise.all([
      invoke<boolean>('can_undo'),
      invoke<boolean>('can_redo'),
    ]);
    setCanUndo(u);
    setCanRedo(r);
  }, []);

  const performUndo = useCallback(async () => {
    try {
      const result = await invoke<UndoResult>('undo');
      await loadNotes();
      if (result.affectedNoteId) setSelectedNoteId(result.affectedNoteId);
      setNoteRefreshSignal(s => s + 1);
      await refreshUndoState();
    } catch (e) {
      const msg = String(e);
      if (!msg.includes('Nothing to undo') && !msg.includes('Nothing to redo')) {
        console.error('[undo/redo]', e);
      }
    }
  }, [loadNotes, setSelectedNoteId, refreshUndoState]);

  const performRedo = useCallback(async () => {
    try {
      const result = await invoke<UndoResult>('redo');
      await loadNotes();
      if (result.affectedNoteId) setSelectedNoteId(result.affectedNoteId);
      setNoteRefreshSignal(s => s + 1);
      await refreshUndoState();
    } catch (e) {
      const msg = String(e);
      if (!msg.includes('Nothing to undo') && !msg.includes('Nothing to redo')) {
        console.error('[undo/redo]', e);
      }
    }
  }, [loadNotes, setSelectedNoteId, refreshUndoState]);

  // Closes the pending note-creation undo group (if one is open) and refreshes state.
  // Safe to call at any time — if no group is open, end_undo_group is a no-op.
  const closePendingUndoGroup = useCallback(async () => {
    if (pendingUndoGroupRef.current) {
      pendingUndoGroupRef.current = false;
      await invoke('end_undo_group');
      await refreshUndoState();
    }
  }, [refreshUndoState]);

  return {
    canUndo,
    canRedo,
    noteRefreshSignal,
    refreshUndoState,
    performUndo,
    performRedo,
    closePendingUndoGroup,
    pendingUndoGroupRef,
  };
}
