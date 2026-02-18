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
    console.log('New Workspace handler called');
    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        defaultPath: 'workspace.db',
        title: 'Create New Workspace'
      });

      console.log('Save dialog returned path:', path);
      if (!path) return;

      console.log('Invoking create_workspace with path:', path);
      await invoke<WorkspaceInfoType>('create_workspace', { path })
        .then(info => {
          console.log('create_workspace returned:', info);
          setWorkspace(info);
          setStatus(`Created: ${info.filename}`);
        })
        .catch(err => {
          console.error('create_workspace error:', err);
          if (err !== 'focused_existing') {
            setStatus(`Error: ${err}`, true);
          }
        });
    } catch (error) {
      console.error('New Workspace handler error:', error);
      setStatus(`Error: ${error}`, true);
    }
  },

  'File > Open Workspace clicked': async () => {
    console.log('Open Workspace handler called');
    try {
      const path = await open({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        multiple: false,
        title: 'Open Workspace'
      });

      console.log('Open dialog returned path:', path);
      if (!path || Array.isArray(path)) return;

      console.log('Invoking open_workspace with path:', path);
      await invoke<WorkspaceInfoType>('open_workspace', { path })
        .then(info => {
          console.log('open_workspace returned:', info);
          setWorkspace(info);
          setStatus(`Opened: ${info.filename}`);
        })
        .catch(err => {
          console.error('open_workspace error:', err);
          if (err !== 'focused_existing') {
            setStatus(`Error: ${err}`, true);
          }
        });
    } catch (error) {
      console.error('Open Workspace handler error:', error);
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

    const unlisten = listen<string>('menu-action', (event) => {
      console.log('Menu event received:', event.payload);
      const handler = handlers[event.payload as keyof typeof handlers];
      if (handler) {
        console.log('Calling handler for:', event.payload);
        handler();
      } else {
        console.log('No handler found for:', event.payload);
      }
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
      {workspace ? <WorkspaceInfo info={workspace} /> : <EmptyState />}
      {status && <StatusMessage message={status} isError={isError} />}
    </div>
  );
}

export default App;
