import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import TreeView from './TreeView';
import SearchBar from './SearchBar';
import InfoPanel from './InfoPanel';
import AddNoteDialog from './AddNoteDialog';
import ContextMenu from './ContextMenu';
import DeleteConfirmDialog from './DeleteConfirmDialog';
import ScriptManagerDialog from './ScriptManagerDialog';
import OperationsLogDialog from './OperationsLogDialog';
import type { Note, TreeNode, WorkspaceInfo, DeleteResult, SchemaInfo } from '../types';
import { DeleteStrategy } from '../types';
import { buildTree, flattenVisibleTree, findNoteInTree, getAncestorIds } from '../utils/tree';

interface WorkspaceViewProps {
  workspaceInfo: WorkspaceInfo;
}

function WorkspaceView({ workspaceInfo }: WorkspaceViewProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const selectedNoteIdRef = useRef(selectedNoteId);
  const treePanelRef = useRef<HTMLDivElement>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [error, setError] = useState<string>('');
  const selectionInitialized = useRef(false);
  const isRefreshing = useRef(false);

  // Context menu state
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; noteId: string } | null>(null);

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
  const [dropIndicator, setDropIndicator] = useState<{ noteId: string; position: 'before' | 'after' | 'child' } | null>(null);

  // Resizable tree panel
  const [treeWidth, setTreeWidth] = useState(300);
  const isDragging = useRef(false);
  const dragStartX = useRef(0);
  const dragStartWidth = useRef(0);

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

  selectedNoteIdRef.current = selectedNoteId;

  // Load notes on mount
  useEffect(() => {
    loadNotes();
  }, []);

  // Set up menu listener
  useEffect(() => {
    const unlisten = listen<string>('menu-action', async (event) => {
      const isFocused = await getCurrentWebviewWindow().isFocused();
      if (!isFocused) return;

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
      const [fetchedNotes, allSchemas] = await Promise.all([
        invoke<Note[]>('list_notes'),
        invoke<Record<string, SchemaInfo>>('get_all_schemas'),
      ]);
      setNotes(fetchedNotes);

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

  const handleSelectNote = async (noteId: string) => {
    setSelectedNoteId(noteId);
    try {
      await invoke('set_selected_note', { noteId });
    } catch (err) {
      console.error('Failed to save selection:', err);
    }
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
    setContextMenu({ x: e.clientX, y: e.clientY, noteId });
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
        <SearchBar notes={notes} onSelect={handleSearchSelect} />
        <div className="flex-1 overflow-y-auto">
          <TreeView
            tree={tree}
            selectedNoteId={selectedNoteId}
            onSelect={handleSelectNote}
            onToggleExpand={handleToggleExpand}
            onContextMenu={handleContextMenu}
            onKeyDown={handleTreeKeyDown}
            notes={notes}
            draggedNoteId={draggedNoteId}
            setDraggedNoteId={setDraggedNoteId}
            dropIndicator={dropIndicator}
            setDropIndicator={setDropIndicator}
            onMoveNote={handleMoveNote}
          />
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
        />
      </div>

      {/* Add Note Dialog */}
      <AddNoteDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onNoteCreated={handleNoteCreated}
        selectedNoteId={selectedNoteId}
        hasNotes={notes.length > 0}
      />

      {/* Context Menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          onAddNote={() => handleContextAddNote(contextMenu.noteId)}
          onEdit={() => handleContextEdit(contextMenu.noteId)}
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
