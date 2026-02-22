import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppSettings, WorkspaceInfo } from '../types';

interface NewWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function NewWorkspaceDialog({ isOpen, onClose }: NewWorkspaceDialogProps) {
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  const [workspaceDir, setWorkspaceDir] = useState('');

  useEffect(() => {
    if (isOpen) {
      setName('');
      setError('');
      setCreating(false);
      invoke<AppSettings>('get_settings')
        .then(s => setWorkspaceDir(s.workspaceDirectory))
        .catch(err => setError(`Failed to load settings: ${err}`));
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !creating) onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, creating]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setError('Please enter a workspace name.');
      return;
    }

    if (/[/\\:*?"<>|]/.test(trimmed)) {
      setError('Name contains invalid characters.');
      return;
    }

    setCreating(true);
    setError('');

    const path = `${workspaceDir}/${trimmed}.db`;

    try {
      await invoke<WorkspaceInfo>('create_workspace', { path });
      onClose();
    } catch (err) {
      if (err !== 'focused_existing') {
        setError(`${err}`);
      }
      setCreating(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !creating) handleCreate();
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">New Workspace</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">
            Workspace Name
          </label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={handleKeyPress}
            placeholder="My Workspace"
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            disabled={creating}
          />
          {workspaceDir && (
            <p className="text-xs text-muted-foreground mt-1">
              Will be saved to: {workspaceDir}/{name.trim() || '...'}.db
            </p>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={creating}
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
            disabled={creating || !name.trim()}
          >
            {creating ? 'Creating...' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}

export default NewWorkspaceDialog;
