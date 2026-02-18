import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceView from './components/WorkspaceView';
import WelcomeDialog from './components/WelcomeDialog';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import type { WorkspaceInfo as WorkspaceInfoType } from './types';
import './styles/globals.css';

const createMenuHandlers = (
  _setWorkspace: (info: WorkspaceInfoType | null) => void,
  setStatus: (msg: string, isError?: boolean) => void
) => ({
  'File > New Workspace clicked': async () => {
    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        defaultPath: 'workspace.db',
        title: 'Create New Workspace'
      });

      if (!path) return;

      await invoke<WorkspaceInfoType>('create_workspace', { path })
        .catch(err => {
          if (err !== 'focused_existing') {
            setStatus(`Error: ${err}`, true);
          }
        });
    } catch (error) {
      setStatus(`Error: ${error}`, true);
    }
  },

  'File > Open Workspace clicked': async () => {
    try {
      const path = await open({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        multiple: false,
        title: 'Open Workspace'
      });

      if (!path || Array.isArray(path)) return;

      await invoke<WorkspaceInfoType>('open_workspace', { path })
        .catch(err => {
          if (err !== 'focused_existing') {
            setStatus(`Error: ${err}`, true);
          }
        });
    } catch (error) {
      setStatus(`Error: ${error}`, true);
    }
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

    // If this is a workspace window (not "main"), fetch workspace info immediately
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      const window = getCurrentWebviewWindow();
      if (window.label !== 'main') {
        invoke<WorkspaceInfoType>('get_workspace_info')
          .then(info => {
            setWorkspace(info);
            setShowWelcome(false);
          })
          .catch(err => console.error('Failed to fetch workspace info:', err));
      }
    });
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

    const unlisten = listen<string>('menu-action', async (event) => {
      // Only handle menu events if this window is focused
      const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow');
      const window = getCurrentWebviewWindow();
      const isFocused = await window.isFocused();

      if (!isFocused) return;

      const handler = handlers[event.payload as keyof typeof handlers];
      if (handler) handler();
    });

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
      {workspace ? <WorkspaceView workspaceInfo={workspace} /> : <EmptyState />}
      {status && <StatusMessage message={status} isError={isError} />}
    </div>
  );
}

export default App;
