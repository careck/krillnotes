import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import TreeView from './TreeView';
import SearchBar from './SearchBar';
import InfoPanel from './InfoPanel';
import AddNoteDialog from './AddNoteDialog';
import ContextMenu from './ContextMenu';
import DeleteConfirmDialog from './DeleteConfirmDialog';
import ScriptManagerDialog from './ScriptManagerDialog';
import OperationsLogDialog from './OperationsLogDialog';
import type { Note, TreeNode, WorkspaceInfo, DeleteResult, SchemaInfo, DropIndicator } from '../types';
import { DeleteStrategy } from '../types';
import { buildTree, flattenVisibleTree, findNoteInTree, getAncestorIds, getDescendantIds } from '../utils/tree';
import TagPill from './TagPill';

interface WorkspaceViewProps {
  workspaceInfo: WorkspaceInfo;
}

function WorkspaceView({ workspaceInfo }: WorkspaceViewProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [schemas, setSchemas] = useState<Record<string, SchemaInfo>>({});
  const [treeActionMap, setTreeActionMap] = useState<Record<string, string[]>>({});
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const [copiedNoteId, setCopiedNoteId] = useState<string | null>(null);
  const [viewHistory, setViewHistory] = useState<string[]>([]);
  const selectedNoteIdRef = useRef(selectedNoteId);
  const treePanelRef = useRef<HTMLDivElement>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [error, setError] = useState<string>('');
  const selectionInitialized = useRef(false);
  const isRefreshing = useRef(false);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string; noteType: string } | null>(null);

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

  // Drag and drop state
  const [draggedNoteId, setDraggedNoteId] = useState<string | null>(null);
  const [dropIndicator, setDropIndicator] = useState<DropIndicator | null>(null);
  const dragDescendants = useMemo(
    () => draggedNoteId ? getDescendantIds(notes, draggedNoteId) : new Set<string>(),
    [notes, draggedNoteId]
  );

  // Resizable tree panel
  const [treeWidth, setTreeWidth] = useState(300);
  const isDragging = useRef(false);
  const dragStartX = useRef(0);
  const dragStartWidth = useRef(0);

  // Tag cloud
  const [workspaceTags, setWorkspaceTags] = useState<string[]>([]);
  const [tagCloudHeight, setTagCloudHeight] = useState(120);
  const [tagFilterQuery, setTagFilterQuery] = useState<string | undefined>(undefined);
  const isTagDragging = useRef(false);
  const tagDragStartY = useRef(0);
  const tagDragStartHeight = useRef(0);

  const handleDividerMouseDown = useCallback((e: React.MouseEvent) => {
    isDragging.current = true;
    dragStartX.current = e.clientX;
    dragStartWidth.current = treeWidth;
    e.preventDefault();
  }, [treeWidth]);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!isDragging.current) return;
      const delta = e.clientX - dragStartX.current;
      setTreeWidth(Math.max(180, Math.min(600, dragStartWidth.current + delta)));
    };
    const onMouseUp = () => { isDragging.current = false; };
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
    return () => {
      document.removeEventListener('mousemove', onMouseMove);
      document.removeEventListener('mouseup', onMouseUp);
    };
  }, []);

  const handleTagDividerMouseDown = useCallback((e: React.MouseEvent) => {
    isTagDragging.current = true;
    tagDragStartY.current = e.clientY;
    tagDragStartHeight.current = tagCloudHeight;
    e.preventDefault();
  }, [tagCloudHeight]);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!isTagDragging.current) return;
      const delta = tagDragStartY.current - e.clientY;
      setTagCloudHeight(Math.max(0, Math.min(400, tagDragStartHeight.current + delta)));
    };
    const onMouseUp = () => { isTagDragging.current = false; };
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
    return () => {
      document.removeEventListener('mousemove', onMouseMove);
      document.removeEventListener('mouseup', onMouseUp);
    };
  }, []);

  selectedNoteIdRef.current = selectedNoteId;

  // Load notes on mount
  useEffect(() => {
    loadNotes();
  }, []);

  // Set up menu listener
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
      if (event.payload === 'Edit > Add Note clicked') {
        setShowAddDialog(true);
      }
      if (event.payload === 'Edit > Manage Scripts clicked') {
        setShowScriptManager(true);
      }
      if (event.payload === 'View > Operations Log clicked') {
        setShowOperationsLog(true);
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
      setError(`Failed to load notes: ${err}`);
      return [];
    }
  };

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
      if (position === 'child') {
        await invoke('toggle_note_expansion', { noteId: selectedNoteId, expanded: true });
      }
      setSelectedNoteId(newId);
      setCopiedNoteId(null);
      invoke('set_paste_menu_enabled', { enabled: false }).catch(console.error);
    } catch (err) {
      console.error('Failed to paste note:', err);
    }
  }, [copiedNoteId, selectedNoteId]);

  const handleTreeAction = useCallback(async (noteId: string, label: string) => {
    try {
      await invoke('invoke_tree_action', { noteId, label });
      await loadNotes();
    } catch (err) {
      setError(`Tree action failed: ${err}`);
    }
  }, []);

  // Keyboard shortcuts: Cmd/Ctrl+C copies selected note, Cmd/Ctrl+V pastes as child,
  // Cmd/Ctrl+Shift+V pastes as sibling. Guards against input fields so normal
  // text copy/paste is unaffected.
  useEffect(() => {
    const isInputFocused = () => {
      const el = document.activeElement;
      if (!el) return false;
      const tag = el.tagName.toLowerCase();
      return tag === 'input' || tag === 'textarea' || (el as HTMLElement).isContentEditable;
    };

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

  const handleSelectNote = async (noteId: string) => {
    setViewHistory([]);
    setSelectedNoteId(noteId);
    try {
      await invoke('set_selected_note', { noteId });
    } catch (err) {
      console.error('Failed to save selection:', err);
    }
  };

  const handleLinkNavigate = (noteId: string) => {
    if (selectedNoteId) {
      setViewHistory(h => [...h, selectedNoteId]);
    }
    setSelectedNoteId(noteId);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
  };

  const handleBack = () => {
    if (viewHistory.length === 0) return;
    const prev = viewHistory[viewHistory.length - 1];
    setViewHistory(h => h.slice(0, -1));
    setSelectedNoteId(prev);
    invoke('set_selected_note', { noteId: prev }).catch(err =>
      console.error('Failed to save selection:', err)
    );
  };

  const handleToggleExpand = async (noteId: string) => {
    try {
      await invoke('toggle_note_expansion', { noteId });
      await loadNotes();
    } catch (err) {
      console.error('Failed to toggle expansion:', err);
    }
  };

  const handleMoveNote = async (noteId: string, newParentId: string | null, newPosition: number) => {
    try {
      await invoke('move_note', { noteId, newParentId, newPosition });
      await loadNotes();
    } catch (err) {
      console.error('Failed to move note:', err);
    }
  };

  const handleSearchSelect = async (noteId: string) => {
    // Expand any collapsed ancestors so the note becomes visible in the tree
    const ancestors = getAncestorIds(notes, noteId);
    const collapsedAncestors = ancestors.filter(
      id => notes.find(n => n.id === id)?.isExpanded === false
    );

    for (const ancestorId of collapsedAncestors) {
      await invoke('toggle_note_expansion', { noteId: ancestorId });
    }

    if (collapsedAncestors.length > 0) {
      await loadNotes();
    }

    await handleSelectNote(noteId);

    // Scroll the note into view in the tree
    requestAnimationFrame(() => {
      document.querySelector(`[data-note-id="${noteId}"]`)?.scrollIntoView({ block: 'nearest' });
    });
  };

  const handleTreeKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).closest('button') !== null) return;
    if (!selectedNoteId) return;
    const flat = flattenVisibleTree(tree);
    const idx = flat.findIndex(n => n.note.id === selectedNoteId);
    if (idx === -1) return;
    const current = flat[idx];

    const selectAndScroll = (noteId: string) => {
      handleSelectNote(noteId);
      requestAnimationFrame(() => {
        document.querySelector(`[data-note-id="${noteId}"]`)?.scrollIntoView({ block: 'nearest' });
      });
    };

    switch (e.key) {
      case 'ArrowDown': {
        e.preventDefault();
        if (idx < flat.length - 1) selectAndScroll(flat[idx + 1].note.id);
        break;
      }
      case 'ArrowUp': {
        e.preventDefault();
        if (idx > 0) selectAndScroll(flat[idx - 1].note.id);
        break;
      }
      case 'ArrowRight': {
        e.preventDefault();
        if (current.children.length > 0) {
          if (!current.note.isExpanded) {
            handleToggleExpand(current.note.id);
          } else {
            selectAndScroll(current.children[0].note.id);
          }
        }
        break;
      }
      case 'ArrowLeft': {
        e.preventDefault();
        if (current.note.isExpanded) {
          handleToggleExpand(current.note.id);
        } else if (current.note.parentId) {
          const parent = findNoteInTree(tree, current.note.parentId);
          if (parent) selectAndScroll(parent.note.id);
        }
        break;
      }
      case 'Enter': {
        e.preventDefault();
        setRequestEditMode(prev => prev + 1);
        break;
      }
    }
  };

  const handleNoteCreated = async (noteId: string) => {
    const fetchedNotes = await loadNotes();
    if (!fetchedNotes.some(n => n.id === noteId)) return;
    await handleSelectNote(noteId);
    setRequestEditMode(prev => prev + 1);
  };

  const handleNoteUpdated = async () => {
    if (isRefreshing.current) return;
    isRefreshing.current = true;
    try {
      const currentId = selectedNoteIdRef.current;
      const freshNotes = await loadNotes();

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
    } finally {
      isRefreshing.current = false;
    }
  };

  // --- Context menu handlers ---

  const handleContextMenu = (e: React.MouseEvent, noteId: string) => {
    const note = notes.find(n => n.id === noteId);
    const noteType = note?.nodeType ?? '';
    setContextMenu({ x: e.clientX, y: e.clientY, noteId, noteType });
  };

  const handleContextAddNote = (noteId: string) => {
    setContextMenu(null);
    setSelectedNoteId(noteId);
    setShowAddDialog(true);
    invoke('set_selected_note', { noteId }).catch(err =>
      console.error('Failed to save selection:', err)
    );
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
      alert(`Failed to check children: ${err}`);
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
      handleNoteUpdated();
    } catch (err) {
      alert(`Failed to delete: ${err}`);
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
    requestAnimationFrame(() => {
      // targets the TreeView container div which carries tabIndex={0}
      treePanelRef.current?.querySelector<HTMLElement>('[tabindex="0"]')?.focus();
    });
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
            onKeyDown={handleTreeKeyDown}
            notes={notes}
            schemas={schemas}
            draggedNoteId={draggedNoteId}
            setDraggedNoteId={setDraggedNoteId}
            dropIndicator={dropIndicator}
            setDropIndicator={setDropIndicator}
            dragDescendants={dragDescendants}
            onMoveNote={handleMoveNote}
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
              onClick={() => setTagFilterQuery(tag)}
            />
          ))}
          {workspaceTags.length === 0 && (
            <span className="kn-tag-cloud__empty">No tags yet</span>
          )}
        </div>
      </div>

      {/* Resize divider */}
      <div
        className="w-1 shrink-0 cursor-col-resize bg-secondary hover:bg-primary/30 transition-colors"
        onMouseDown={handleDividerMouseDown}
      />

      {/* Right panel - Info */}
      <div className="flex-1 min-w-0 overflow-y-auto">
        <InfoPanel
          selectedNote={selectedNote}
          onNoteUpdated={handleNoteUpdated}
          onDeleteRequest={handleDeleteRequest}
          requestEditMode={requestEditMode}
          onEditDone={handleEditDone}
          onLinkNavigate={handleLinkNavigate}
          onBack={handleBack}
          backNoteTitle={backNoteTitle}
        />
      </div>

      {/* Add Note Dialog */}
      <AddNoteDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onNoteCreated={handleNoteCreated}
        selectedNoteId={selectedNoteId}
        hasNotes={notes.length > 0}
        notes={notes}
        schemas={schemas}
      />

      {/* Context Menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          copiedNoteId={copiedNoteId}
          treeActions={treeActionMap[contextMenu.noteType] ?? []}
          onAddNote={() => handleContextAddNote(contextMenu.noteId)}
          onEdit={() => handleContextEdit(contextMenu.noteId)}
          onCopy={() => copyNote(contextMenu.noteId)}
          onPasteAsChild={() => pasteNote('child')}
          onPasteAsSibling={() => pasteNote('sibling')}
          onTreeAction={(label) => handleTreeAction(contextMenu.noteId, label)}
          onDelete={() => handleContextDelete(contextMenu.noteId)}
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
        onScriptsChanged={loadNotes}
      />

      {/* Operations Log Dialog */}
      <OperationsLogDialog
        isOpen={showOperationsLog}
        onClose={() => setShowOperationsLog(false)}
      />
    </div>
  );
}

export default WorkspaceView;
