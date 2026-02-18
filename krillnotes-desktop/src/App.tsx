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
  const [showWelcome, setShowWelcome] = useState(true);
  const [workspace, setWorkspace] = useState<WorkspaceInfoType | null>(null);
  const [status, setStatus] = useState('');
  const [isError, setIsError] = useState(false);

  useEffect(() => {
    const welcomed = localStorage.getItem('krillnotes_welcomed');
    if (welcomed === 'true') {
      setShowWelcome(false);
    }
  }, []);

  useEffect(() => {
    const handlers = createMenuHandlers(
      setWorkspace,
      (msg, error = false) => {
        setStatus(msg);
        setIsError(error);
        setTimeout(() => setStatus(''), 5000);
      }
    );

    const unlisten = listen<string>('menu-action', (event) =>
      handlers[event.payload as keyof typeof handlers]?.()
    );

    return () => { unlisten.then(f => f()); };
  }, []);

  const handleDismissWelcome = () => {
    localStorage.setItem('krillnotes_welcomed', 'true');
    setShowWelcome(false);
  };

  if (showWelcome) {
    return <WelcomeDialog onDismiss={handleDismissWelcome} />;
  }

  return (
    <div className="min-h-screen bg-background text-foreground p-8">
      {workspace ? <WorkspaceInfo info={workspace} /> : <EmptyState />}
      {status && <StatusMessage message={status} isError={isError} />}
    </div>
  );
}

export default App;
