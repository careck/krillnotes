import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceInfo from './components/WorkspaceInfo';
import WelcomeDialog from './components/WelcomeDialog';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import type { WorkspaceInfo as WorkspaceInfoType } from './types';
import './styles/globals.css';

const createMenuHandlers = (
  setWorkspace: (info: WorkspaceInfoType | null) => void,
  setStatus: (msg: string, isError?: boolean) => void
) => ({
  'File > New Workspace clicked': async () => {
    const path = await save({
      filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
      defaultPath: 'workspace.db',
      title: 'Create New Workspace'
    });

    if (!path) return;

    await invoke<WorkspaceInfoType>('create_workspace', { path })
      .then(info => {
        setWorkspace(info);
        setStatus(`Created: ${info.filename}`);
      })
      .catch(err => err !== 'focused_existing' && setStatus(`Error: ${err}`, true));
  },

  'File > Open Workspace clicked': async () => {
    const path = await open({
      filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
      multiple: false,
      title: 'Open Workspace'
    });

    if (!path || Array.isArray(path)) return;

    await invoke<WorkspaceInfoType>('open_workspace', { path })
      .then(info => {
        setWorkspace(info);
        setStatus(`Opened: ${info.filename}`);
      })
      .catch(err => err !== 'focused_existing' && setStatus(`Error: ${err}`, true));
  },
});

function App() {
  const [statusMessage, setStatusMessage] = useState('Welcome to Krillnotes');

  useEffect(() => {
    // Listen for menu events from Rust backend
    const unlisten = listen<string>('menu-action', (event) => {
      setStatusMessage(event.payload);
    });

    return () => {
      unlisten.then(f => f());
    };
  }, []);

  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center">
      <div className="text-center">
        <h1 className="text-4xl font-bold mb-4">Krillnotes</h1>
        <StatusMessage message={statusMessage} />
      </div>
    </div>
  );
}

export default App;
