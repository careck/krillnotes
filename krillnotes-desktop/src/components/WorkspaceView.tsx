// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useResizablePanels } from '../hooks/useResizablePanels';
import { useHoverTooltip } from '../hooks/useHoverTooltip';
import { useUndoRedo } from '../hooks/useUndoRedo';
import { useTagCloud } from '../hooks/useTagCloud';
import { useTreeState } from '../hooks/useTreeState';
import { useRelayPolling } from '../hooks/useRelayPolling';
import { Undo2, Redo2 } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useTranslation } from 'react-i18next';
import TreeView from './TreeView';
import SearchBar from './SearchBar';
import InfoPanel from './InfoPanel';
import AddNoteDialog from './AddNoteDialog';
import ContextMenu from './ContextMenu';
import DeleteConfirmDialog from './DeleteConfirmDialog';
import HoverTooltip from './HoverTooltip';
import ScriptManagerDialog from './ScriptManagerDialog';
import OperationsLogDialog from './OperationsLogDialog';
import WorkspacePropertiesDialog from './WorkspacePropertiesDialog';
import { InviteManagerDialog } from './InviteManagerDialog';
import { ShareDialog } from './ShareDialog';
import { CascadePreviewDialog } from './CascadePreviewDialog';
import type { Note, TreeNode, WorkspaceInfo, DeleteResult, SchemaInfo, DropIndicator, SchemaMigratedEvent, ReceivedResponseInfo, CascadeImpactRow } from '../types';
import { DeleteStrategy } from '../types';
import { buildTree, getDescendantIds } from '../utils/tree';
import { getAvailableTypes, type NotePosition } from '../utils/noteTypes';
import TagPill from './TagPill';

interface WorkspaceViewProps {
  workspaceInfo: WorkspaceInfo;
  onOpenWorkspacePeers?: () => void;
}

function WorkspaceView({ workspaceInfo, onOpenWorkspacePeers }: WorkspaceViewProps) {
  const { t } = useTranslation();
  const [notes, setNotes] = useState<Note[]>([]);
  const [schemas, setSchemas] = useState<Record<string, SchemaInfo>>({});
  const [treeActionMap, setTreeActionMap] = useState<Record<string, string[]>>({});
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [copiedNoteId, setCopiedNoteId] = useState<string | null>(null);
  const [effectiveRoles, setEffectiveRoles] = useState<Record<string, string>>({});
  const [shareAnchorIds, setShareAnchorIds] = useState<Set<string>>(new Set());
  const [isRootOwner, setIsRootOwner] = useState(false);
  const [permissionRefreshSignal, setPermissionRefreshSignal] = useState(0);
  const treePanelRef = useRef<HTMLDivElement>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [addDialogNoteId, setAddDialogNoteId] = useState<string | null>(null);
  const [addDialogPosition, setAddDialogPosition] = useState<NotePosition>('child');
  const [error, setError] = useState<string>('');
  const isRefreshing = useRef(false);
  // Ref used to break the circular dep between useTreeState (needs closePendingUndoGroup)
  // and useUndoRedo (needs setSelectedNoteId). Populated after both hooks have run.
  const closePendingUndoGroupRef = useRef<(() => Promise<void>) | undefined>(undefined);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string | null; noteType: string; effectiveRole: string | null; isRootOwner: boolean; isRootNote: boolean } | null>(null);

  // Delete dialog state (lifted from InfoPanel)
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [pendingDeleteChildCount, setPendingDeleteChildCount] = useState(0);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);

  // Incremented to signal InfoPanel to enter edit mode
  const [requestEditMode, setRequestEditMode] = useState(0);

  // Script manager dialog state
  const [showScriptManager, setShowScriptManager] = useState(false);

  // Operations log dialog state
  const [showOperationsLog, setShowOperationsLog] = useState(false);

  // Workspace properties dialog state
  const [showWorkspaceProperties, setShowWorkspaceProperties] = useState(false);

  // Invite-to-subtree dialog state
  const [inviteScope, setInviteScope] = useState<{ noteId: string; noteTitle: string } | null>(null);

  // Share-subtree dialog state
  const [shareScope, setShareScope] = useState<{ noteId: string; noteTitle: string } | null>(null);

  // Cascade preview dialog state
  const [cascadeState, setCascadeState] = useState<{
    noteId: string;
    userId: string;
    userName: string;
    action: 'demote' | 'revoke';
    newRole?: string;
    oldRole: string;
    noteTitle: string;
  } | null>(null);

  // Schema migration toast state
  const [migrationToasts, setMigrationToasts] = useState<SchemaMigratedEvent[]>([]);

  // Invite response toast state
  const [responseToasts, setResponseToasts] = useState<ReceivedResponseInfo[]>([]);

  // Workspace-level relay polling
  const [hasRelayPeers, setHasRelayPeers] = useState(false);

  // Drag and drop state
  const [draggedNoteId, setDraggedNoteId] = useState<string | null>(null);
  const [dropIndicator, setDropIndicator] = useState<DropIndicator | null>(null);
  const dragDescendants = useMemo(
    () => draggedNoteId ? getDescendantIds(notes, draggedNoteId) : new Set<string>(),
    [notes, draggedNoteId]
  );

  // Hover tooltip
  const { hoveredNoteId, tooltipAnchorY, hoverHtml, handleHoverStart, handleHoverEnd } =
    useHoverTooltip(draggedNoteId, notes, schemas);

  // Resizable panels
  const { treeWidth, tagCloudHeight, handleDividerMouseDown, handleTagDividerMouseDown } =
    useResizablePanels();

  // Tag cloud
  const { workspaceTags, setWorkspaceTags, tagFilterQuery, handleTagClick } =
    useTagCloud();

  // Load notes on mount
  useEffect(() => {
    loadNotes();
    loadPermissionState();
  }, []);

  // Listen for schema migration events emitted on workspace open.
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<SchemaMigratedEvent>('schema-migrated', (event) => {
      const toast = event.payload;
      setMigrationToasts(prev => [...prev, toast]);
      setTimeout(() => {
        setMigrationToasts(prev => prev.filter(t => t !== toast));
      }, 6000);
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  // Refresh the tree when a delta bundle has been applied to this workspace.
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen('workspace-updated', () => {
      loadNotes();
      loadPermissionState();
    });
    return () => { unlisten.then(f => f()); };
  }, []);

  // Listen for invite response notifications.
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<ReceivedResponseInfo>(
      "invite-response-received",
      (event) => {
        const toast = event.payload;
        setResponseToasts(prev => [...prev, toast]);
        setTimeout(() => {
          setResponseToasts(prev => prev.filter(t2 => t2 !== toast));
        }, 10000);
      }
    );
    return () => { unlisten.then(f => f()); };
  }, []);

  // Enable polling when workspace has non-manual peers OR relay credentials
  // (inviter needs polling to discover accept bundles even before peers exist).
  useEffect(() => {
    Promise.all([
      invoke<any[]>("list_workspace_peers").catch(() => []),
      invoke<boolean>("has_relay_credentials").catch(() => false),
    ]).then(([peers, hasCreds]) => {
      setHasRelayPeers(
        (peers as any[]).some((p: any) => p.channelType !== "manual") || (hasCreds as boolean)
      );
    });
  }, []);

  useRelayPolling(hasRelayPeers);

  // Set up menu listener
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
      if (event.payload === 'Edit > Add Note clicked') {
        openAddDialogRef.current?.('child', selectedNoteIdRef.current);
      }
      if (event.payload === 'Edit > Manage Scripts clicked') {
        setShowScriptManager(true);
      }
      if (event.payload === 'View > Operations Log clicked') {
        setShowOperationsLog(true);
      }
      if (event.payload === 'Edit > Workspace Properties clicked') {
        setShowWorkspaceProperties(true);
      }
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  const loadNotes = async (): Promise<Note[]> => {
    try {
      const [fetchedNotes, allSchemas, actionMap, allTags] = await Promise.all([
        invoke<Note[]>('list_notes'),
        invoke<Record<string, SchemaInfo>>('get_all_schemas'),
        invoke<Record<string, string[]>>('get_tree_action_map'),
        invoke<string[]>('get_all_tags'),
      ]);
      setNotes(fetchedNotes);
      setSchemas(allSchemas);
      setTreeActionMap(actionMap);
      setWorkspaceTags(allTags);

      // Build sort config from schemas
      const sortConfig: Record<string, 'asc' | 'desc' | 'none'> = {};
      for (const [nodeType, schema] of Object.entries(allSchemas)) {
        sortConfig[nodeType] = schema.childrenSort;
      }

      const builtTree = buildTree(fetchedNotes, sortConfig);
      setTree(builtTree);

      if (!selectionInitialized.current) {
        selectionInitialized.current = true;
        if (workspaceInfo.selectedNoteId) {
          setSelectedNoteId(workspaceInfo.selectedNoteId);
        } else if (builtTree.length > 0) {
          const firstRootId = builtTree[0].note.id;
          setSelectedNoteId(firstRootId);
          await invoke('set_selected_note', { noteId: firstRootId });
        }
      }

      return fetchedNotes;
    } catch (err) {
      setError(t('workspace.failedLoad', { error: String(err) }));
      return [];
    }
  };

  const loadPermissionState = async () => {
    try {
      const [roles, anchors, rootOwner] = await Promise.all([
        invoke<Record<string, string>>('get_all_effective_roles'),
        invoke<string[]>('get_share_anchor_ids'),
        invoke<boolean>('is_root_owner'),
      ]);
      setEffectiveRoles(roles);
      setShareAnchorIds(new Set(anchors));
      setIsRootOwner(rootOwner);
    } catch {
      // RBAC not enabled for this workspace — default to permissive
      setEffectiveRoles({});
      setShareAnchorIds(new Set());
      setIsRootOwner(true);
    }
  };

  // Tree state — selection, expansion, keyboard navigation, link navigation.
  // Placed after loadNotes because the hook receives loadNotes as a parameter and
  // TypeScript enforces const TDZ. loadNotes in turn closes over selectionInitialized
  // and setSelectedNoteId from this hook — this is safe because loadNotes is only
  // ever called from effects/event handlers, never synchronously during render.
  // TODO: Convert loadNotes to useCallback (or ref pattern) to make this ordering explicit.
  const {
    selectedNoteId, setSelectedNoteId, selectedNoteIdRef, viewHistory,
    handleSelectNote, handleToggleExpand, handleLinkNavigate, handleBack,
    handleSearchSelect, handleTreeKeyDown, selectionInitialized,
  } = useTreeState(notes, tree, schemas, closePendingUndoGroupRef, loadNotes, setRequestEditMode);

  // Undo/redo state — placed after useTreeState so setSelectedNoteId is available.
  const { canUndo, canRedo, noteRefreshSignal, refreshUndoState, performUndo, performRedo, closePendingUndoGroup, pendingUndoGroupRef } =
    useUndoRedo(loadNotes, setSelectedNoteId);

  // Keep the ref in sync so useTreeState can call closePendingUndoGroup without
  // creating a circular dependency between the two hooks.
  closePendingUndoGroupRef.current = closePendingUndoGroup;

  const copyNote = useCallback((noteId: string) => {
    setCopiedNoteId(noteId);
    invoke('set_paste_menu_enabled', { enabled: true }).catch(console.error);
  }, []);

  const pasteNote = useCallback(async (position: 'child' | 'sibling') => {
    if (!copiedNoteId || !selectedNoteId) return;
    try {
      const newId = await invoke<string>('deep_copy_note_cmd', {
        sourceNoteId: copiedNoteId,
        targetNoteId: selectedNoteId,
        position,
      });
      await loadNotes();
      await loadPermissionState();
      if (position === 'child') {
        await invoke('toggle_note_expansion', { noteId: selectedNoteId, expanded: true });
      }
      setSelectedNoteId(newId);
      setCopiedNoteId(null);
      invoke('set_paste_menu_enabled', { enabled: false }).catch(console.error);
      await refreshUndoState();
    } catch (err) {
      console.error('Failed to paste note:', err);
    }
  }, [copiedNoteId, selectedNoteId, refreshUndoState]);

  const handleTreeAction = useCallback(async (noteId: string, label: string) => {
    try {
      await invoke('invoke_tree_action', { noteId, label });
      await loadNotes();
      await loadPermissionState();
      await refreshUndoState();
    } catch (err) {
      setError(t('workspace.treeActionFailed', { error: String(err) }));
    }
  }, [refreshUndoState]);

  const isInputFocused = () => {
    const el = document.activeElement;
    if (!el) return false;
    const tag = el.tagName.toLowerCase();
    return tag === 'input' || tag === 'textarea' || (el as HTMLElement).isContentEditable;
  };

  // Keyboard shortcuts: Cmd/Ctrl+C copies selected note, Cmd/Ctrl+V pastes as child,
  // Cmd/Ctrl+Shift+V pastes as sibling. Guards against input fields so normal
  // text copy/paste is unaffected.
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (isInputFocused()) return;

      if (e.key === 'c' && !e.shiftKey) {
        if (selectedNoteId) { copyNote(selectedNoteId); e.preventDefault(); }
      } else if (e.key === 'v' && !e.shiftKey) {
        pasteNote('child'); e.preventDefault();
      } else if (e.key === 'v' && e.shiftKey) {
        pasteNote('sibling'); e.preventDefault();
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [selectedNoteId, copiedNoteId, copyNote, pasteNote]);

  // Keyboard shortcuts: Cmd/Ctrl+Z for undo, Cmd/Ctrl+Shift+Z or Cmd/Ctrl+Y for redo.
  useEffect(() => {
    const handleUndoRedo = async (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (isInputFocused()) return;
      if (e.key === 'z' && !e.shiftKey) {
        e.preventDefault();
        await performUndo();
      } else if ((e.key === 'z' && e.shiftKey) || e.key === 'y') {
        e.preventDefault();
        await performRedo();
      }
    };
    document.addEventListener('keydown', handleUndoRedo);
    return () => document.removeEventListener('keydown', handleUndoRedo);
  }, [performUndo, performRedo]);

  // When this window regains focus, re-sync the native paste menu state.
  // This matters on macOS where a single menu bar is shared by all workspace
  // windows — the enabled state must reflect whichever window is now active.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    let unlisten: (() => void) | null = null;
    win.onFocusChanged(({ payload: focused }) => {
      if (focused) {
        invoke('set_paste_menu_enabled', { enabled: copiedNoteId !== null }).catch(console.error);
      }
    }).then(fn => { unlisten = fn; });
    return () => { unlisten?.(); };
  }, [copiedNoteId]);

  // Handle copy/paste actions from the Edit menu.
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlisten = win.listen<string>('menu-action', (event) => {
      switch (event.payload) {
        case 'Edit > Copy Note clicked':
          if (selectedNoteId) copyNote(selectedNoteId);
          break;
        case 'Edit > Paste as Child clicked':
          pasteNote('child');
          break;
        case 'Edit > Paste as Sibling clicked':
          pasteNote('sibling');
          break;
      }
    });
    return () => { unlisten.then(f => f()); };
  }, [selectedNoteId, copiedNoteId, copyNote, pasteNote]);

  const handleMoveNote = async (noteId: string, newParentId: string | null, newPosition: number) => {
    try {
      await invoke('move_note', { noteId, newParentId, newPosition });
      await loadNotes();
      await loadPermissionState();
      await refreshUndoState();
    } catch (err) {
      console.error('Failed to move note:', err);
    }
  };

  const handleNoteCreated = async (noteId: string) => {
    const fetchedNotes = await loadNotes();
    await loadPermissionState();
    if (!fetchedNotes.some(n => n.id === noteId)) return;
    // Mark that a note-creation undo group is open so handleEditDone can close it.
    pendingUndoGroupRef.current = true;
    await handleSelectNote(noteId);
    setRequestEditMode(prev => prev + 1);
    await refreshUndoState();
  };

  const handleNoteUpdated = async () => {
    if (isRefreshing.current) return;
    isRefreshing.current = true;
    try {
      const currentId = selectedNoteIdRef.current;
      const freshNotes = await loadNotes();
      await loadPermissionState();

      if (currentId && !freshNotes.some(n => n.id === currentId)) {
        const freshTree = buildTree(freshNotes);
        const firstId = freshTree.length > 0 ? freshTree[0].note.id : null;

        if (firstId) {
          setSelectedNoteId(firstId);
          try {
            await invoke('set_selected_note', { noteId: firstId });
          } catch (err) {
            console.error('Failed to save auto-selection:', err);
          }
        } else {
          setSelectedNoteId(null);
        }
      }
      await refreshUndoState();
    } finally {
      isRefreshing.current = false;
    }
  };

  // --- Context menu handlers ---

  const handleContextMenu = (e: React.MouseEvent, noteId: string) => {
    const note = notes.find(n => n.id === noteId);
    const noteType = note?.schema ?? '';
    const effectiveRole = effectiveRoles[noteId] ?? null;
    const isRootNote = !note?.parentId;
    setContextMenu({ x: e.clientX, y: e.clientY, noteId, noteType, effectiveRole, isRootOwner, isRootNote });
  };

  // Opens AddNoteDialog or creates directly if only one type is available
  const openAddDialogRef = useRef<typeof openAddDialog | null>(null);
  const openAddDialog = (position: NotePosition, referenceNoteId: string | null) => {
    const available = getAvailableTypes(position, referenceNoteId, notes, schemas);
    if (available.length === 0) return;
    if (available.length === 1) {
      const parentId = position === 'root' ? null : referenceNoteId;
      const tauriPosition = position === 'root' ? 'child' : position;
      invoke('begin_undo_group')
        .then(() => invoke<Note>('create_note_with_type', { parentId, position: tauriPosition, schema: available[0] }))
        .then(note => handleNoteCreated(note.id).then(() => refreshUndoState()))
        .catch(err => console.error('Failed to create note:', err));
      return;
    }
    setAddDialogNoteId(referenceNoteId);
    setAddDialogPosition(position);
    setShowAddDialog(true);
  };
  openAddDialogRef.current = openAddDialog;

  const handleContextAddChild = (noteId: string) => {
    setContextMenu(null);
    openAddDialog('child', noteId);
  };

  const handleContextAddSibling = (noteId: string) => {
    setContextMenu(null);
    openAddDialog('sibling', noteId);
  };

  const handleContextAddRoot = () => {
    setContextMenu(null);
    openAddDialog('root', null);
  };

  const handleBackgroundContextMenu = (e: React.MouseEvent) => {
    setContextMenu({ x: e.clientX, y: e.clientY, noteId: null, noteType: '', effectiveRole: null, isRootOwner, isRootNote: false });
  };

  const handleContextEdit = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    setRequestEditMode(prev => prev + 1);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
  };

  const handleContextDelete = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
    handleDeleteRequest(noteId);
  };

  // --- Delete handlers (lifted from InfoPanel) ---

  const handleDeleteRequest = async (noteId: string) => {
    try {
      const count = await invoke<number>('count_children', { noteId });
      setPendingDeleteChildCount(count);
      setPendingDeleteId(noteId);
      setShowDeleteDialog(true);
    } catch (err) {
      alert(t('workspace.failedCheckChildren', { error: String(err) }));
    }
  };

  const handleDeleteConfirm = async (strategy: DeleteStrategy) => {
    if (!pendingDeleteId || isDeleting) return;
    setIsDeleting(true);
    try {
      await invoke<DeleteResult>('delete_note', {
        noteId: pendingDeleteId,
        strategy,
      });
      setShowDeleteDialog(false);
      setPendingDeleteId(null);
      setIsDeleting(false);
      await handleNoteUpdated();
    } catch (err) {
      alert(t('workspace.failedDelete', { error: String(err) }));
      setShowDeleteDialog(false);
      setPendingDeleteId(null);
      setIsDeleting(false);
    }
  };

  const handleDeleteCancel = () => {
    setShowDeleteDialog(false);
    setPendingDeleteId(null);
    setIsDeleting(false);
  };

  const handleEditDone = () => {
    closePendingUndoGroup();
    requestAnimationFrame(() => {
      // targets the TreeView container div which carries tabIndex={0}
      treePanelRef.current?.querySelector<HTMLElement>('[tabindex="0"]')?.focus();
    });
  };

  // --- Share/cascade handlers ---

  const handleShareSubtree = (noteId: string) => {
    const note = notes.find(n => n.id === noteId);
    setShareScope({ noteId, noteTitle: note?.title ?? noteId });
  };

  const handleRoleChange = async (noteId: string, userId: string, newRole: string, oldRole: string) => {
    // Check for cascade impact first
    try {
      const impacts = await invoke<CascadeImpactRow[]>('preview_cascade', {
        noteId, userId, newRole,
      });
      if (impacts.length > 0) {
        const name = await invoke<string>('resolve_identity_name', { publicKey: userId }).catch(() => userId.slice(0, 8));
        const note = notes.find(n => n.id === noteId);
        setCascadeState({
          noteId, userId, userName: name,
          action: 'demote', newRole, oldRole,
          noteTitle: note?.title ?? noteId,
        });
        return;
      }
    } catch { /* no cascade needed */ }

    // No cascade impact — apply directly
    try {
      await invoke('set_permission', { noteId, userId, role: newRole });
      loadPermissionState();
      setPermissionRefreshSignal(prev => prev + 1);
    } catch (e) {
      console.error('Failed to change role:', e);
    }
  };

  const handleRevokeGrant = async (noteId: string, userId: string) => {
    try {
      const impacts = await invoke<CascadeImpactRow[]>('preview_cascade', {
        noteId, userId, newRole: 'none',
      });
      if (impacts.length > 0) {
        const name = await invoke<string>('resolve_identity_name', { publicKey: userId }).catch(() => userId.slice(0, 8));
        const note = notes.find(n => n.id === noteId);
        setCascadeState({
          noteId, userId, userName: name,
          action: 'revoke', oldRole: 'unknown',
          noteTitle: note?.title ?? noteId,
        });
        return;
      }
    } catch { /* no cascade needed */ }

    try {
      await invoke('revoke_permission', { noteId, userId });
      loadPermissionState();
      setPermissionRefreshSignal(prev => prev + 1);
    } catch (e) {
      console.error('Failed to revoke:', e);
    }
  };

  const handleCascadeConfirm = async (revokeGrants: Array<{ noteId: string; userId: string }>) => {
    if (!cascadeState) return;
    try {
      if (cascadeState.action === 'demote') {
        await invoke('set_permission', {
          noteId: cascadeState.noteId,
          userId: cascadeState.userId,
          role: cascadeState.newRole,
        });
      } else {
        await invoke('revoke_permission', {
          noteId: cascadeState.noteId,
          userId: cascadeState.userId,
        });
      }
      for (const grant of revokeGrants) {
        await invoke('revoke_permission', {
          noteId: grant.noteId,
          userId: grant.userId,
        });
      }
      loadPermissionState();
      setPermissionRefreshSignal(prev => prev + 1);
    } catch (e) {
      console.error('Cascade action failed:', e);
    } finally {
      setCascadeState(null);
    }
  };

  const selectedNote = selectedNoteId
    ? notes.find(n => n.id === selectedNoteId) || null
    : null;

  const pendingDeleteNote = pendingDeleteId
    ? notes.find(n => n.id === pendingDeleteId) || null
    : null;

  const backNoteTitle = viewHistory.length > 0
    ? (notes.find(n => n.id === viewHistory[viewHistory.length - 1])?.title ?? '…')
    : undefined;

  if (error) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-red-500">{error}</div>
      </div>
    );
  }

  return (
    <div className="flex h-screen">
      {/* Left sidebar - Tree */}
      <div
        ref={treePanelRef}
        className="shrink-0 bg-background overflow-hidden flex flex-col"
        style={{ width: treeWidth }}
      >
        <SearchBar notes={notes} onSelect={handleSearchSelect} externalQuery={tagFilterQuery} />
        <div className="flex-1 overflow-y-auto">
          <TreeView
            tree={tree}
            selectedNoteId={selectedNoteId}
            onSelect={handleSelectNote}
            onToggleExpand={handleToggleExpand}
            onContextMenu={handleContextMenu}
            onBackgroundContextMenu={handleBackgroundContextMenu}
            onKeyDown={handleTreeKeyDown}
            notes={notes}
            schemas={schemas}
            draggedNoteId={draggedNoteId}
            setDraggedNoteId={setDraggedNoteId}
            dropIndicator={dropIndicator}
            setDropIndicator={setDropIndicator}
            dragDescendants={dragDescendants}
            onMoveNote={handleMoveNote}
            onHoverStart={handleHoverStart}
            onHoverEnd={handleHoverEnd}
            effectiveRoles={effectiveRoles}
            shareAnchorIds={shareAnchorIds}
          />
        </div>

        {/* Tag cloud drag handle */}
        <div className="kn-tag-divider" onMouseDown={handleTagDividerMouseDown} />

        {/* Tag cloud */}
        <div
          className="kn-tag-cloud"
          style={{ height: tagCloudHeight, overflow: tagCloudHeight === 0 ? 'hidden' : 'auto' }}
        >
          {workspaceTags.map(tag => (
            <TagPill
              key={tag}
              tag={tag}
              onClick={() => handleTagClick(tag)}
            />
          ))}
          {workspaceTags.length === 0 && (
            <span className="kn-tag-cloud__empty">{t('workspace.noTagsYet')}</span>
          )}
        </div>
      </div>

      {/* Resize divider */}
      <div
        className="w-1 shrink-0 cursor-col-resize bg-secondary hover:bg-primary/30 transition-colors"
        onMouseDown={handleDividerMouseDown}
      />

      {/* Right panel - Info */}
      <div className="flex-1 min-w-0 flex flex-col overflow-hidden">
        {/* Toolbar */}
        <div className="flex items-center gap-1 px-2 py-1 border-b border-border shrink-0">
          <button
            onClick={performUndo}
            disabled={!canUndo}
            title={t('workspace.undoTooltip')}
            className="p-1 rounded hover:bg-muted disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Undo2 className="w-4 h-4" />
          </button>
          <button
            onClick={performRedo}
            disabled={!canRedo}
            title={t('workspace.redoTooltip')}
            className="p-1 rounded hover:bg-muted disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Redo2 className="w-4 h-4" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto">
          <InfoPanel
            selectedNote={selectedNote}
            onNoteUpdated={handleNoteUpdated}
            onDeleteRequest={handleDeleteRequest}
            requestEditMode={requestEditMode}
            onEditDone={handleEditDone}
            onLinkNavigate={handleLinkNavigate}
            onBack={handleBack}
            backNoteTitle={backNoteTitle}
            refreshSignal={noteRefreshSignal + permissionRefreshSignal}
            onShareSubtree={handleShareSubtree}
            onRoleChange={handleRoleChange}
            onRevokeGrant={handleRevokeGrant}
          />
        </div>
      </div>

      {/* Add Note Dialog */}
      <AddNoteDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onNoteCreated={handleNoteCreated}
        referenceNoteId={addDialogNoteId}
        position={addDialogPosition}
        notes={notes}
        schemas={schemas}
      />

      {/* Context Menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          noteId={contextMenu.noteId}
          copiedNoteId={copiedNoteId}
          isLeaf={schemas[contextMenu.noteType ?? '']?.isLeaf ?? false}
          treeActions={contextMenu.noteId ? (treeActionMap[contextMenu.noteType] ?? []) : []}
          effectiveRole={contextMenu.effectiveRole}
          isRootOwner={contextMenu.isRootOwner}
          isRootNote={contextMenu.isRootNote}
          onAddChild={() => contextMenu.noteId && handleContextAddChild(contextMenu.noteId)}
          onAddSibling={() => contextMenu.noteId && handleContextAddSibling(contextMenu.noteId)}
          onAddRoot={handleContextAddRoot}
          onEdit={() => contextMenu.noteId && handleContextEdit(contextMenu.noteId)}
          onCopy={() => contextMenu.noteId && copyNote(contextMenu.noteId)}
          onPasteAsChild={() => pasteNote('child')}
          onPasteAsSibling={() => pasteNote('sibling')}
          onTreeAction={(label) => contextMenu.noteId && handleTreeAction(contextMenu.noteId, label)}
          onInviteToSubtree={(noteId) => {
            const note = notes.find(n => n.id === noteId);
            setInviteScope({ noteId, noteTitle: note?.title ?? noteId });
          }}
          onShareSubtree={handleShareSubtree}
          onDelete={() => contextMenu.noteId && handleContextDelete(contextMenu.noteId)}
          onClose={() => setContextMenu(null)}
        />
      )}

      {/* Delete Confirm Dialog (handles both InfoPanel button and context menu) */}
      {showDeleteDialog && pendingDeleteNote && (
        <DeleteConfirmDialog
          noteTitle={pendingDeleteNote.title}
          childCount={pendingDeleteChildCount}
          onConfirm={handleDeleteConfirm}
          onCancel={handleDeleteCancel}
          disabled={isDeleting}
        />
      )}

      {/* Script Manager Dialog */}
      <ScriptManagerDialog
        isOpen={showScriptManager}
        onClose={() => setShowScriptManager(false)}
        onScriptsChanged={async () => { await loadNotes(); await loadPermissionState(); await refreshUndoState(); }}
      />

      {/* Operations Log Dialog */}
      <OperationsLogDialog
        isOpen={showOperationsLog}
        onClose={() => setShowOperationsLog(false)}
      />

      {/* Workspace Properties Dialog */}
      <WorkspacePropertiesDialog
        isOpen={showWorkspaceProperties}
        onClose={() => setShowWorkspaceProperties(false)}
      />

      {/* Invite to Subtree Dialog */}
      {inviteScope && workspaceInfo.identityUuid && (
        <InviteManagerDialog
          identityUuid={workspaceInfo.identityUuid}
          workspaceName={workspaceInfo.filename}
          initialScope={inviteScope}
          onClose={() => setInviteScope(null)}
        />
      )}

      {/* Share Subtree Dialog */}
      {shareScope && (
        <ShareDialog
          open={true}
          noteId={shareScope.noteId}
          noteTitle={shareScope.noteTitle}
          currentUserRole={effectiveRoles[shareScope.noteId] ?? 'owner'}
          onComplete={() => {
            setShareScope(null);
            loadPermissionState();
            setPermissionRefreshSignal(prev => prev + 1);
          }}
          onClose={() => setShareScope(null)}
        />
      )}

      {/* Cascade Preview Dialog */}
      {cascadeState && (
        <CascadePreviewDialog
          open={true}
          noteId={cascadeState.noteId}
          userId={cascadeState.userId}
          userName={cascadeState.userName}
          action={cascadeState.action}
          newRole={cascadeState.newRole}
          oldRole={cascadeState.oldRole}
          noteTitle={cascadeState.noteTitle}
          onConfirm={handleCascadeConfirm}
          onClose={() => setCascadeState(null)}
        />
      )}

      {/* Schema migration toasts */}
      {migrationToasts.length > 0 && (
        <div className="fixed bottom-4 right-4 flex flex-col gap-2 z-50">
          {migrationToasts.map((t, i) => (
            <div key={i} className="bg-blue-600 text-white px-4 py-2 rounded-lg shadow-lg text-sm">
              <strong>"{t.schemaName}" schema updated</strong> — {t.notesMigrated} note{t.notesMigrated !== 1 ? 's' : ''} migrated to version {t.toVersion}
            </div>
          ))}
        </div>
      )}

      {/* Invite response toasts */}
      {responseToasts.length > 0 && (
        <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
          {responseToasts.map((toast, i) => (
            <div key={i} className="bg-gray-800 border border-purple-600 rounded-xl px-4 py-3 shadow-lg max-w-xs text-white">
              <div className="font-semibold text-sm">{t("polling.newInviteResponse")}</div>
              <div className="text-xs text-gray-300 mt-1">
                {toast.inviteeDeclaredName} {t("polling.respondedToYourInvite")}
              </div>
              <div className="flex gap-2 mt-2">
                <button
                  className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1 rounded-md"
                  onClick={() => {
                    onOpenWorkspacePeers?.();
                    setResponseToasts(prev => prev.filter(t2 => t2 !== toast));
                  }}
                >
                  {t("polling.viewInPeers")}
                </button>
                <button
                  className="bg-transparent text-gray-400 border border-gray-600 text-xs px-3 py-1 rounded-md"
                  onClick={() => setResponseToasts(prev => prev.filter(t2 => t2 !== toast))}
                >
                  {t("common.dismiss", "Dismiss")}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Hover Tooltip */}
      {hoveredNoteId && (() => {
        const note = notes.find(n => n.id === hoveredNoteId);
        const schema = note ? (schemas[note.schema] ?? null) : null;
        if (!note) return null;
        const hasHoverFields = schema?.fields.some(f => f.showOnHover) ?? false;
        if (hoverHtml === null && !hasHoverFields) return null;
        return (
          <HoverTooltip
            note={note}
            schema={schema}
            hoverHtml={hoverHtml}
            anchorY={tooltipAnchorY}
            treeWidth={treeWidth}
            visible={true}
          />
        );
      })()}
    </div>
  );
}

export default WorkspaceView;
