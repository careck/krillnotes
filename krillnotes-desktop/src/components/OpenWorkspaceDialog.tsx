import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { WorkspaceEntry, WorkspaceInfo } from '../types';

interface OpenWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function OpenWorkspaceDialog({ isOpen, onClose }: OpenWorkspaceDialogProps) {
  const [entries, setEntries] = useState<WorkspaceEntry[]>([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [opening, setOpening] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      setError('');
      setOpening(null);
      setLoading(true);
      invoke<WorkspaceEntry[]>('list_workspace_files')
        .then(setEntries)
        .catch(err => setError(`Failed to list workspaces: ${err}`))
        .finally(() => setLoading(false));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !opening) onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, opening]);

  if (!isOpen) return null;

  const handleOpen = async (entry: WorkspaceEntry) => {
    if (entry.isOpen) return;

    setOpening(entry.path);
    setError('');
    try {
      await invoke<WorkspaceInfo>('open_workspace', { path: entry.path });
      onClose();
    } catch (err) {
      if (err === 'focused_existing') {
        onClose();
      } else {
        setError(`${err}`);
      }
      setOpening(null);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary rounded-lg w-[450px] max-h-[60vh] flex flex-col">
        <div className="p-6 pb-0">
          <h2 className="text-xl font-bold mb-4">Open Workspace</h2>
        </div>

        <div className="flex-1 overflow-y-auto px-6">
          {loading ? (
            <p className="text-muted-foreground text-center py-8">
              Loading...
            </p>
          ) : entries.length === 0 ? (
            <p className="text-muted-foreground text-center py-8">
              No workspaces found in the default directory.
              <br />
              Use "New Workspace" to create one.
            </p>
          ) : (
            <div className="space-y-1">
              {entries.map(entry => (
                <button
                  key={entry.path}
                  onClick={() => handleOpen(entry)}
                  disabled={opening !== null || entry.isOpen}
                  className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between ${
                    entry.isOpen
                      ? 'opacity-40 cursor-not-allowed'
                      : 'hover:bg-secondary/50 disabled:opacity-50'
                  }`}
                >
                  <span className="font-medium truncate">{entry.name}</span>
                  {entry.isOpen && (
                    <span className="text-xs text-muted-foreground ml-2">Already open</span>
                  )}
                  {opening === entry.path && (
                    <span className="text-xs text-muted-foreground ml-2">Opening...</span>
                  )}
                </button>
              ))}
            </div>
          )}
        </div>

        {error && (
          <div className="px-6 pt-2">
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {error}
            </div>
          </div>
        )}

        <div className="flex justify-end p-6 pt-4">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={opening !== null}
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}

export default OpenWorkspaceDialog;
