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

  'File > Export Workspace clicked': async () => {
    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        defaultPath: 'workspace.krillnotes.zip',
        title: 'Export Workspace'
      });

      if (!path) return;

      await invoke('export_workspace_cmd', { path });
      setStatus('Workspace exported successfully');
    } catch (error) {
      setStatus(`Export failed: ${error}`, true);
    }
  },

  'File > Import Workspace clicked': async () => {
    try {
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        multiple: false,
        title: 'Import Workspace'
      });

      if (!zipPath || Array.isArray(zipPath)) return;

      // Peek at metadata to check version
      const result = await invoke<{ appVersion: string; noteCount: number; scriptCount: number }>(
        'peek_import_cmd', { zipPath }
      );

      // Check app version — warn if export is from a newer version
      const currentVersion = await invoke<string>('get_app_version');
      if (result.appVersion > currentVersion) {
        const { confirm } = await import('@tauri-apps/plugin-dialog');
        const proceed = await confirm(
          `This export was created with Krillnotes v${result.appVersion}, but you are running v${currentVersion}. Some data may not import correctly.\n\nImport anyway?`,
          { title: 'Version Mismatch', kind: 'warning' }
        );
        if (!proceed) return;
      }

      // Pick where to save the new .db file
      const dbPath = await save({
        filters: [{ name: 'Krillnotes Database', extensions: ['db'] }],
        defaultPath: 'imported-workspace.db',
        title: 'Save Imported Workspace As'
      });

      if (!dbPath) return;

      // Execute the import
      await invoke('execute_import', { zipPath, dbPath });
      setStatus(`Imported ${result.noteCount} notes and ${result.scriptCount} scripts`);
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
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
    <div className="min-h-screen bg-background text-foreground">
      {workspace ? <WorkspaceView workspaceInfo={workspace} /> : <div className="p-8"><EmptyState /></div>}
      {status && <StatusMessage message={status} isError={isError} />}
    </div>
  );
}

export default App;
