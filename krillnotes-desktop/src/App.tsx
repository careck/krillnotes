import { useEffect, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open, save } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import WorkspaceView from './components/WorkspaceView';
import WelcomeDialog from './components/WelcomeDialog';
import EmptyState from './components/EmptyState';
import StatusMessage from './components/StatusMessage';
import NewWorkspaceDialog from './components/NewWorkspaceDialog';
import OpenWorkspaceDialog from './components/OpenWorkspaceDialog';
import SettingsDialog from './components/SettingsDialog';
import type { WorkspaceInfo as WorkspaceInfoType, AppSettings } from './types';
import './styles/globals.css';

interface ImportState {
  zipPath: string;
  noteCount: number;
  scriptCount: number;
}

const createMenuHandlers = (
  setStatus: (msg: string, isError?: boolean) => void,
  setShowNewWorkspace: (show: boolean) => void,
  setShowOpenWorkspace: (show: boolean) => void,
  setShowSettings: (show: boolean) => void,
  setImportState: (state: ImportState | null) => void,
  workspace: WorkspaceInfoType | null,
) => ({
  'File > New Workspace clicked': () => {
    setShowNewWorkspace(true);
  },

  'File > Open Workspace clicked': () => {
    setShowOpenWorkspace(true);
  },

  'File > Export Workspace clicked': async () => {
    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        defaultPath: `${(workspace?.filename ?? 'workspace').replace(/\.db$/, '')}.krillnotes.zip`,
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

      // Show name-entry dialog instead of save dialog
      setImportState({
        zipPath: zipPath as string,
        noteCount: result.noteCount,
        scriptCount: result.scriptCount,
      });
    } catch (error) {
      setStatus(`Import failed: ${error}`, true);
    }
  },

  'Edit > Settings clicked': () => {
    setShowSettings(true);
  },
});

function App() {
  const [showWelcome, setShowWelcome] = useState(true);
  const [workspace, setWorkspace] = useState<WorkspaceInfoType | null>(null);
  const [status, setStatus] = useState('');
  const [isError, setIsError] = useState(false);
  const [showNewWorkspace, setShowNewWorkspace] = useState(false);
  const [showOpenWorkspace, setShowOpenWorkspace] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [importState, setImportState] = useState<ImportState | null>(null);
  const [importName, setImportName] = useState('');
  const [importError, setImportError] = useState('');
  const [importing, setImporting] = useState(false);

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

  const statusSetter = (msg: string, error = false) => {
    setStatus(msg);
    setIsError(error);
    setTimeout(() => setStatus(''), 5000);
  };

  useEffect(() => {
    const handlers = createMenuHandlers(
      statusSetter,
      setShowNewWorkspace,
      setShowOpenWorkspace,
      setShowSettings,
      setImportState,
      workspace,
    );

    const unlisten = getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
      const handler = handlers[event.payload as keyof typeof handlers];
      if (handler) handler();
    });

    return () => { unlisten.then(f => f()); };
  }, [workspace]);

  // Reset import dialog state when it opens
  useEffect(() => {
    if (importState) {
      setImportName('imported-workspace');
      setImportError('');
      setImporting(false);
    }
  }, [importState]);

  const handleImportConfirm = async () => {
    if (!importState) return;

    const trimmed = importName.trim();
    if (!trimmed) {
      setImportError('Please enter a workspace name.');
      return;
    }
    if (/[/\\:*?"<>|]/.test(trimmed)) {
      setImportError('Name contains invalid characters.');
      return;
    }

    setImporting(true);
    setImportError('');

    try {
      const settings = await invoke<AppSettings>('get_settings');
      const dbPath = `${settings.workspaceDirectory}/${trimmed}.db`;
      await invoke('execute_import', { zipPath: importState.zipPath, dbPath });
      statusSetter(`Imported ${importState.noteCount} notes and ${importState.scriptCount} scripts`);
      setImportState(null);
    } catch (error) {
      setImportError(`${error}`);
      setImporting(false);
    }
  };

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

      <NewWorkspaceDialog
        isOpen={showNewWorkspace}
        onClose={() => setShowNewWorkspace(false)}
      />
      <OpenWorkspaceDialog
        isOpen={showOpenWorkspace}
        onClose={() => setShowOpenWorkspace(false)}
      />
      <SettingsDialog
        isOpen={showSettings}
        onClose={() => setShowSettings(false)}
      />

      {/* Import name dialog — inline since it's a lightweight prompt */}
      {importState && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">Import Workspace</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Importing {importState.noteCount} notes and {importState.scriptCount} scripts.
            </p>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">
                Workspace Name
              </label>
              <input
                type="text"
                value={importName}
                onChange={(e) => setImportName(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && !importing) handleImportConfirm(); }}
                placeholder="imported-workspace"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
                disabled={importing}
              />
            </div>

            {importError && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {importError}
              </div>
            )}

            <div className="flex justify-end gap-2">
              <button
                onClick={() => setImportState(null)}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                disabled={importing}
              >
                Cancel
              </button>
              <button
                onClick={handleImportConfirm}
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
                disabled={importing || !importName.trim()}
              >
                {importing ? 'Importing...' : 'Import'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
