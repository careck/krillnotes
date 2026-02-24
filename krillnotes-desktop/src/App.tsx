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

function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

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
  setShowExportPasswordDialog: (show: boolean) => void,
  doImport: (zipPath: string) => void,
) => ({
  'File > New Workspace clicked': () => {
    setShowNewWorkspace(true);
  },

  'File > Open Workspace clicked': () => {
    setShowOpenWorkspace(true);
  },

  'File > Export Workspace clicked': () => {
    setShowExportPasswordDialog(true);
  },

  'File > Import Workspace clicked': async () => {
    try {
      const zipPath = await open({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        multiple: false,
        title: 'Import Workspace',
      });
      if (!zipPath || Array.isArray(zipPath)) return;
      doImport(zipPath as string);
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
  const [showImportPasswordDialog, setShowImportPasswordDialog] = useState(false);
  const [importPassword, setImportPassword] = useState('');
  const [importPasswordError, setImportPasswordError] = useState('');
  const [pendingImportZipPath, setPendingImportZipPath] = useState<string | null>(null);
  const [pendingImportPassword, setPendingImportPassword] = useState<string | null>(null);
  const [showExportPasswordDialog, setShowExportPasswordDialog] = useState(false);
  const [exportPassword, setExportPassword] = useState('');
  const [exportPasswordConfirm, setExportPasswordConfirm] = useState('');

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
      setShowExportPasswordDialog,
      (zipPath) => proceedWithImport(zipPath, null),
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

    const slug = slugify(trimmed);
    if (!slug) {
      setImportError('Name must contain at least one letter or number.');
      return;
    }

    setImporting(true);
    setImportError('');

    try {
      const settings = await invoke<AppSettings>('get_settings');
      const dbPath = `${settings.workspaceDirectory}/${slug}.db`;
      await invoke('execute_import', { zipPath: importState.zipPath, dbPath, password: pendingImportPassword });
      statusSetter(`Imported ${importState.noteCount} notes and ${importState.scriptCount} scripts`);
      setImportState(null);
      setPendingImportPassword(null);
    } catch (error) {
      setImportError(`${error}`);
      setImporting(false);
    }
  };

  const handleExportConfirm = async (password: string | null) => {
    setShowExportPasswordDialog(false);
    setExportPassword('');
    setExportPasswordConfirm('');

    try {
      const path = await save({
        filters: [{ name: 'Krillnotes Export', extensions: ['zip'] }],
        defaultPath: `${(workspace?.filename ?? 'workspace').replace(/\.db$/, '')}.krillnotes.zip`,
        title: 'Export Workspace',
      });

      if (!path) return;

      await invoke('export_workspace_cmd', { path, password });
      statusSetter('Workspace exported successfully');
    } catch (error) {
      statusSetter(`Export failed: ${error}`, true);
    }
  };

  const proceedWithImport = async (zipPath: string, password: string | null) => {
    try {
      const result = await invoke<{ appVersion: string; noteCount: number; scriptCount: number }>(
        'peek_import_cmd', { zipPath, password }
      );

      const currentVersion = await invoke<string>('get_app_version');
      if (result.appVersion > currentVersion) {
        const { confirm } = await import('@tauri-apps/plugin-dialog');
        const proceed = await confirm(
          `This export was created with Krillnotes v${result.appVersion}, but you are running v${currentVersion}. Some data may not import correctly.\n\nImport anyway?`,
          { title: 'Version Mismatch', kind: 'warning' }
        );
        if (!proceed) return;
      }

      setShowImportPasswordDialog(false);
      setImportPassword('');
      setPendingImportPassword(password);
      setImportState({
        zipPath,
        noteCount: result.noteCount,
        scriptCount: result.scriptCount,
      });
    } catch (error) {
      const errStr = `${error}`;
      if (errStr === 'ENCRYPTED_ARCHIVE') {
        setPendingImportZipPath(zipPath);
        setImportPassword('');
        setImportPasswordError('');
        setShowImportPasswordDialog(true);
      } else if (errStr === 'INVALID_PASSWORD') {
        setImportPasswordError('Incorrect password — try again.');
      } else {
        statusSetter(`Import failed: ${errStr}`, true);
      }
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

      {/* Export password dialog */}
      {showExportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">Protect with a password?</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Leave blank to export without encryption.
            </p>
            <div className="mb-3">
              <label className="block text-sm font-medium mb-2">Password</label>
              <input
                type="password"
                value={exportPassword}
                onChange={(e) => setExportPassword(e.target.value)}
                placeholder="Optional password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
              />
            </div>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">Confirm password</label>
              <input
                type="password"
                value={exportPasswordConfirm}
                onChange={(e) => setExportPasswordConfirm(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    if (!exportPassword || exportPassword === exportPasswordConfirm) {
                      handleExportConfirm(exportPassword || null);
                    }
                  }
                }}
                placeholder="Confirm password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              />
            </div>
            {exportPassword && exportPasswordConfirm && exportPassword !== exportPasswordConfirm && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                Passwords do not match.
              </div>
            )}
            <div className="flex justify-between items-center">
              <button
                onClick={() => {
                  setShowExportPasswordDialog(false);
                  setExportPassword('');
                  setExportPasswordConfirm('');
                }}
                className="text-sm text-muted-foreground hover:text-foreground underline"
              >
                Cancel
              </button>
              <div className="flex gap-2">
                <button
                  onClick={() => handleExportConfirm(null)}
                  className="px-4 py-2 border border-secondary rounded hover:bg-secondary text-sm"
                >
                  Skip — no encryption
                </button>
                <button
                  onClick={() => handleExportConfirm(exportPassword)}
                  disabled={!exportPassword || exportPassword !== exportPasswordConfirm}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  Encrypt
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Import password dialog */}
      {showImportPasswordDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background border border-secondary p-6 rounded-lg w-96">
            <h2 className="text-xl font-bold mb-4">This archive is password-protected</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Enter the password used when the workspace was exported.
            </p>
            <div className="mb-4">
              <label className="block text-sm font-medium mb-2">Password</label>
              <input
                type="password"
                value={importPassword}
                onChange={(e) => setImportPassword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && importPassword && pendingImportZipPath) {
                    setImportPasswordError('');
                    proceedWithImport(pendingImportZipPath, importPassword);
                  }
                }}
                placeholder="Enter password"
                className="w-full bg-secondary border border-secondary rounded px-3 py-2"
                autoFocus
              />
            </div>
            {importPasswordError && (
              <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {importPasswordError}
              </div>
            )}
            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setShowImportPasswordDialog(false);
                  setPendingImportZipPath(null);
                  setImportPassword('');
                  setImportPasswordError('');
                }}
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  if (!pendingImportZipPath) return;
                  setImportPasswordError('');
                  proceedWithImport(pendingImportZipPath, importPassword);
                }}
                disabled={!importPassword}
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Decrypt
              </button>
            </div>
          </div>
        </div>
      )}

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
                onClick={() => { setImportState(null); setPendingImportPassword(null); }}
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
