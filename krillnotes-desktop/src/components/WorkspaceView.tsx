import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import TreeView from './TreeView';
import InfoPanel from './InfoPanel';
import AddNoteDialog from './AddNoteDialog';
import type { Note, TreeNode, WorkspaceInfo } from '../types';
import { buildTree } from '../utils/tree';

interface WorkspaceViewProps {
  workspaceInfo: WorkspaceInfo;
}

function WorkspaceView({ workspaceInfo }: WorkspaceViewProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [tree, setTree] = useState<TreeNode[]>([]);
  const [selectedNoteId, setSelectedNoteId] = useState<string | null>(null);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [error, setError] = useState<string>('');
  const selectionInitialized = useRef(false);
  const isRefreshing = useRef(false);

  // Load notes on mount
  useEffect(() => {
    loadNotes();
  }, []);

  // Set up menu listener
  useEffect(() => {
    const unlisten = listen<string>('menu-action', async (event) => {
      // Only handle menu events if this window is focused
      const isFocused = await getCurrentWebviewWindow().isFocused();
      if (!isFocused) return;

      if (event.payload === 'Edit > Add Note clicked') {
        setShowAddDialog(true);
      }
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  const loadNotes = async (): Promise<Note[]> => {
    try {
      const fetchedNotes = await invoke<Note[]>('list_notes');
      setNotes(fetchedNotes);

      const builtTree = buildTree(fetchedNotes);
      setTree(builtTree);

      // Set initial selection only on first load — subsequent reloads preserve
      // the current in-session selection managed by handleSelectNote
      if (!selectionInitialized.current) {
        selectionInitialized.current = true;
        if (workspaceInfo.selectedNoteId) {
          setSelectedNoteId(workspaceInfo.selectedNoteId);
        } else if (builtTree.length > 0) {
          // Auto-select first root node
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
      // Reload notes to get updated is_expanded values
      await loadNotes();
    } catch (err) {
      console.error('Failed to toggle expansion:', err);
    }
  };

  const handleNoteCreated = async () => {
    await loadNotes();
  };

  const handleNoteUpdated = async () => {
    if (isRefreshing.current) return;
    isRefreshing.current = true;
    try {
      const currentId = selectedNoteId;
      const freshNotes = await loadNotes();

      // Auto-select if the previously selected note was deleted
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

  const selectedNote = selectedNoteId
    ? notes.find(n => n.id === selectedNoteId) || null
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
      <div className="w-[300px] border-r border-secondary bg-background overflow-hidden">
        <TreeView
          tree={tree}
          selectedNoteId={selectedNoteId}
          onSelect={handleSelectNote}
          onToggleExpand={handleToggleExpand}
        />
      </div>

      {/* Right panel - Info */}
      <div className="flex-1 overflow-y-auto">
        <InfoPanel selectedNote={selectedNote} onNoteUpdated={handleNoteUpdated} />
      </div>

      {/* Add Note Dialog */}
      <AddNoteDialog
        isOpen={showAddDialog}
        onClose={() => setShowAddDialog(false)}
        onNoteCreated={handleNoteCreated}
        selectedNoteId={selectedNoteId}
        hasNotes={notes.length > 0}
      />
    </div>
  );
}

export default WorkspaceView;
