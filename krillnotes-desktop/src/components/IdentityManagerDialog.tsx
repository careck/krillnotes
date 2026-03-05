import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { confirm } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { IdentityRef } from '../types';
import CreateIdentityDialog from './CreateIdentityDialog';
import UnlockIdentityDialog from './UnlockIdentityDialog';

interface IdentityManagerDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function IdentityManagerDialog({ isOpen, onClose }: IdentityManagerDialogProps) {
  const { t } = useTranslation();

  const [identities, setIdentities] = useState<IdentityRef[]>([]);
  const [unlockedIds, setUnlockedIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renamingValue, setRenamingValue] = useState('');
  const [changingPassphrase, setChangingPassphrase] = useState<string | null>(null);
  const [currentPassphrase, setCurrentPassphrase] = useState('');
  const [newPassphrase, setNewPassphrase] = useState('');
  const [confirmNewPassphrase, setConfirmNewPassphrase] = useState('');
  const [passphraseError, setPassphraseError] = useState('');
  const [passphraseSuccess, setPassphraseSuccess] = useState('');
  const [savingPassphrase, setSavingPassphrase] = useState(false);
  const [renameError, setRenameError] = useState('');
  const [savingRename, setSavingRename] = useState(false);
  const [unlocking, setUnlocking] = useState<string | null>(null);
  const [error, setError] = useState('');

  const loadData = async () => {
    setLoading(true);
    try {
      const [ids, unlocked] = await Promise.all([
        invoke<IdentityRef[]>('list_identities'),
        invoke<string[]>('get_unlocked_identities'),
      ]);
      setIdentities(ids);
      setUnlockedIds(new Set(unlocked));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (!isOpen) return;
    setError('');
    setShowCreate(false);
    setRenaming(null);
    setChangingPassphrase(null);
    setUnlocking(null);
    loadData();
  }, [isOpen]);

  if (!isOpen) return null;

  const handleLock = async (identityUuid: string) => {
    setError('');
    try {
      await invoke('lock_identity', { identityUuid });
      await loadData();
    } catch (err) {
      setError(String(err));
    }
  };

  const handleDelete = async (identity: IdentityRef) => {
    const confirmed = await confirm(t('identity.deleteConfirm', { name: identity.displayName }));
    if (!confirmed) return;
    setError('');
    try {
      await invoke('delete_identity', { identityUuid: identity.uuid });
      await loadData();
    } catch (err) {
      const msg = String(err);
      if (msg.includes('IdentityHasBoundWorkspaces')) {
        setError(t('identity.deleteHasBound'));
      } else if (msg.includes('unlocked') || msg.includes('lock') || msg.includes('Unlocked')) {
        setError(t('identity.mustLockFirst'));
      } else {
        setError(msg);
      }
    }
  };

  const handleStartRename = (identity: IdentityRef) => {
    setRenaming(identity.uuid);
    setRenamingValue(identity.displayName);
    setRenameError('');
    setChangingPassphrase(null);
    setPassphraseError('');
    setPassphraseSuccess('');
  };

  const handleCancelRename = () => {
    setRenaming(null);
    setRenamingValue('');
    setRenameError('');
  };

  const handleSaveRename = async (identityUuid: string) => {
    if (!renamingValue.trim()) {
      setRenameError(t('identity.nameRequired'));
      return;
    }
    setSavingRename(true);
    setRenameError('');
    try {
      await invoke('rename_identity', { identityUuid, newName: renamingValue.trim() });
      setRenaming(null);
      setRenamingValue('');
      await loadData();
    } catch (err) {
      setRenameError(String(err));
    } finally {
      setSavingRename(false);
    }
  };

  const handleStartChangePassphrase = (identityUuid: string) => {
    setChangingPassphrase(identityUuid);
    setCurrentPassphrase('');
    setNewPassphrase('');
    setConfirmNewPassphrase('');
    setPassphraseError('');
    setPassphraseSuccess('');
    setRenaming(null);
    setRenameError('');
  };

  const handleCancelChangePassphrase = () => {
    setChangingPassphrase(null);
    setCurrentPassphrase('');
    setNewPassphrase('');
    setConfirmNewPassphrase('');
    setPassphraseError('');
    setPassphraseSuccess('');
  };

  const handleSavePassphrase = async (identityUuid: string) => {
    setPassphraseError('');
    setPassphraseSuccess('');
    if (!currentPassphrase) {
      setPassphraseError(t('identity.passphraseRequired'));
      return;
    }
    if (!newPassphrase) {
      setPassphraseError(t('identity.passphraseRequired'));
      return;
    }
    if (newPassphrase !== confirmNewPassphrase) {
      setPassphraseError(t('identity.passphraseMismatch'));
      return;
    }
    setSavingPassphrase(true);
    try {
      await invoke('change_identity_passphrase', {
        identityUuid,
        oldPassphrase: currentPassphrase,
        newPassphrase,
      });
      setPassphraseSuccess(t('identity.passphraseChanged'));
      setCurrentPassphrase('');
      setNewPassphrase('');
      setConfirmNewPassphrase('');
      await loadData();
      // Close the form after a brief moment to show the success message
      setTimeout(() => {
        setChangingPassphrase(null);
        setPassphraseSuccess('');
      }, 1500);
    } catch (err) {
      const msg = String(err);
      if (msg === 'WRONG_PASSPHRASE' || msg.includes('WrongPassphrase') || msg.includes('wrong passphrase')) {
        setPassphraseError(t('identity.wrongPassphrase'));
      } else {
        setPassphraseError(msg);
      }
    } finally {
      setSavingPassphrase(false);
    }
  };

  const isUnlocked = (uuid: string) => unlockedIds.has(uuid);

  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-background border border-border rounded-lg w-[520px] max-h-[80vh] flex flex-col">
          {/* Header */}
          <div className="flex items-center justify-between p-4 border-b border-border">
            <h2 className="text-xl font-bold">{t('identity.manageTitle')}</h2>
            <button
              onClick={() => {
                setShowCreate(true);
              }}
              className="px-3 py-1.5 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 text-sm"
            >
              + {t('identity.create')}
            </button>
          </div>

          {/* Body */}
          <div className="flex-1 overflow-y-auto p-4">
            {loading ? (
              <p className="text-muted-foreground text-center py-8">{t('workspace.loading')}</p>
            ) : identities.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-12 gap-4">
                <p className="text-muted-foreground text-center">{t('identity.noIdentities')}</p>
                <button
                  onClick={() => setShowCreate(true)}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
                >
                  {t('identity.create')}
                </button>
              </div>
            ) : (
              <div className="space-y-3">
                {identities.map((identity) => {
                  const unlocked = isUnlocked(identity.uuid);
                  const isRenaming = renaming === identity.uuid;
                  const isChangingPass = changingPassphrase === identity.uuid;

                  return (
                    <div
                      key={identity.uuid}
                      className="border border-border rounded-md overflow-hidden"
                    >
                      {/* Identity row */}
                      <div className="flex items-center gap-3 p-3">
                        <span
                          className="text-lg shrink-0"
                          title={unlocked ? t('identity.unlocked') : t('identity.locked')}
                        >
                          {unlocked ? '🔓' : '🔒'}
                        </span>
                        <div className="flex-1 min-w-0">
                          <div className="font-medium truncate">{identity.displayName}</div>
                        </div>
                        <div className="flex items-center gap-1 shrink-0">
                          {unlocked ? (
                            <button
                              onClick={() => handleLock(identity.uuid)}
                              className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
                            >
                              {t('identity.lock')}
                            </button>
                          ) : (
                            <button
                              onClick={() => setUnlocking(identity.uuid)}
                              className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
                            >
                              {t('identity.unlock')}
                            </button>
                          )}
                          <button
                            onClick={() =>
                              isRenaming ? handleCancelRename() : handleStartRename(identity)
                            }
                            className={[
                              'px-2 py-1 text-xs border border-border rounded hover:bg-secondary',
                              isRenaming ? 'bg-secondary' : '',
                            ].join(' ')}
                          >
                            {t('identity.rename')}
                          </button>
                          <button
                            onClick={() =>
                              isChangingPass
                                ? handleCancelChangePassphrase()
                                : handleStartChangePassphrase(identity.uuid)
                            }
                            className={[
                              'px-2 py-1 text-xs border border-border rounded hover:bg-secondary',
                              isChangingPass ? 'bg-secondary' : '',
                            ].join(' ')}
                          >
                            {t('identity.changePassphrase')}
                          </button>
                          <button
                            onClick={() => handleDelete(identity)}
                            className="px-2 py-1 text-xs border border-red-500/40 text-red-500 rounded hover:bg-red-500/10"
                          >
                            {t('identity.delete')}
                          </button>
                        </div>
                      </div>

                      {/* Inline rename form */}
                      {isRenaming && (
                        <div className="px-3 pb-3 pt-1 border-t border-border bg-secondary/30">
                          <label className="block text-xs font-medium mb-1 text-muted-foreground">
                            {t('identity.displayName')}
                          </label>
                          <div className="flex gap-2">
                            <input
                              type="text"
                              value={renamingValue}
                              onChange={(e) => setRenamingValue(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') handleSaveRename(identity.uuid);
                                if (e.key === 'Escape') handleCancelRename();
                              }}
                              className="flex-1 bg-background border border-border rounded px-2 py-1 text-sm"
                              autoFocus
                              disabled={savingRename}
                            />
                            <button
                              onClick={() => handleSaveRename(identity.uuid)}
                              className="px-3 py-1 bg-primary text-primary-foreground rounded text-sm hover:bg-primary/90 disabled:opacity-50"
                              disabled={savingRename}
                            >
                              {savingRename ? t('common.saving') : t('common.save')}
                            </button>
                            <button
                              onClick={handleCancelRename}
                              className="px-3 py-1 border border-border rounded text-sm hover:bg-secondary"
                              disabled={savingRename}
                            >
                              {t('common.cancel')}
                            </button>
                          </div>
                          {renameError && (
                            <p className="mt-1 text-xs text-red-500">{renameError}</p>
                          )}
                        </div>
                      )}

                      {/* Inline change passphrase form */}
                      {isChangingPass && (
                        <div className="px-3 pb-3 pt-1 border-t border-border bg-secondary/30 space-y-2">
                          <div>
                            <label className="block text-xs font-medium mb-1 text-muted-foreground">
                              {t('identity.currentPassphrase')}
                            </label>
                            <input
                              type="password"
                              value={currentPassphrase}
                              onChange={(e) => setCurrentPassphrase(e.target.value)}
                              className="w-full bg-background border border-border rounded px-2 py-1 text-sm"
                              autoFocus
                              disabled={savingPassphrase}
                            />
                          </div>
                          <div>
                            <label className="block text-xs font-medium mb-1 text-muted-foreground">
                              {t('identity.newPassphrase')}
                            </label>
                            <input
                              type="password"
                              value={newPassphrase}
                              onChange={(e) => setNewPassphrase(e.target.value)}
                              className="w-full bg-background border border-border rounded px-2 py-1 text-sm"
                              disabled={savingPassphrase}
                            />
                          </div>
                          <div>
                            <label className="block text-xs font-medium mb-1 text-muted-foreground">
                              {t('identity.confirmPassphrase')}
                            </label>
                            <input
                              type="password"
                              value={confirmNewPassphrase}
                              onChange={(e) => setConfirmNewPassphrase(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') handleSavePassphrase(identity.uuid);
                                if (e.key === 'Escape') handleCancelChangePassphrase();
                              }}
                              className="w-full bg-background border border-border rounded px-2 py-1 text-sm"
                              disabled={savingPassphrase}
                            />
                          </div>
                          {passphraseError && (
                            <p className="text-xs text-red-500">{passphraseError}</p>
                          )}
                          {passphraseSuccess && (
                            <p className="text-xs text-green-500">{passphraseSuccess}</p>
                          )}
                          <div className="flex justify-end gap-2 pt-1">
                            <button
                              onClick={handleCancelChangePassphrase}
                              className="px-3 py-1 border border-border rounded text-sm hover:bg-secondary"
                              disabled={savingPassphrase}
                            >
                              {t('common.cancel')}
                            </button>
                            <button
                              onClick={() => handleSavePassphrase(identity.uuid)}
                              className="px-3 py-1 bg-primary text-primary-foreground rounded text-sm hover:bg-primary/90 disabled:opacity-50"
                              disabled={savingPassphrase}
                            >
                              {savingPassphrase ? t('common.saving') : t('common.save')}
                            </button>
                          </div>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}

            {/* Global error */}
            {error && (
              <div className="mt-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
                {error}
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="flex justify-end p-4 border-t border-border">
            <button
              onClick={onClose}
              className="px-4 py-2 border border-border rounded-md hover:bg-secondary"
            >
              {t('common.close')}
            </button>
          </div>
        </div>
      </div>

      {/* Sub-dialogs rendered outside the main dialog to avoid z-index conflicts */}
      <CreateIdentityDialog
        isOpen={showCreate}
        onCreated={() => {
          setShowCreate(false);
          loadData();
        }}
        onCancel={() => setShowCreate(false)}
      />

      <UnlockIdentityDialog
        isOpen={unlocking !== null}
        identityUuid={unlocking ?? ''}
        identityName={identities.find((i) => i.uuid === unlocking)?.displayName ?? ''}
        onUnlocked={() => {
          setUnlocking(null);
          loadData();
        }}
        onCancel={() => setUnlocking(null)}
      />
    </>
  );
}

export default IdentityManagerDialog;
