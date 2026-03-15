// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { confirm, save, open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { IdentityRef, ContactInfo, RelayAccountInfo } from '../types';
import CreateIdentityDialog from './CreateIdentityDialog';
import UnlockIdentityDialog from './UnlockIdentityDialog';
import ContactBookDialog from './ContactBookDialog';
import RelayBookDialog from './RelayBookDialog';

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
  const [renamingValue, setRenamingValue] = useState('');
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

  const [showContactBook, setShowContactBook] = useState<string | null>(null); // identity UUID when open
  const [contactCounts, setContactCounts] = useState<Record<string, number>>({}); // identityUuid → count
  const [showRelayBook, setShowRelayBook] = useState<string | null>(null); // identity UUID when open
  const [relayCounts, setRelayCounts] = useState<Record<string, number>>({}); // identityUuid → count

  // New state for selection-based UX
  const [selectedUuid, setSelectedUuid] = useState<string | null>(null);
  const [activeForm, setActiveForm] = useState<'rename' | 'passphrase' | 'publickey' | null>(null);
  const [publicKey, setPublicKey] = useState('');
  const [fingerprint, setFingerprint] = useState('');
  const [publicKeyCopied, setPublicKeyCopied] = useState(false);
  const [exporting, setExporting] = useState(false);

  const loadData = async () => {
    setLoading(true);
    try {
      const [ids, unlocked] = await Promise.all([
        invoke<IdentityRef[]>('list_identities'),
        invoke<string[]>('get_unlocked_identities'),
      ]);
      setIdentities(ids);
      const unlockedSet = new Set(unlocked);
      setUnlockedIds(unlockedSet);
      // Load contact and relay counts for unlocked identities (silently ignore errors)
      ids.filter(id => unlockedSet.has(id.uuid)).forEach(identity => {
        invoke<ContactInfo[]>('list_contacts', { identityUuid: identity.uuid })
          .then(contacts => setContactCounts(prev => ({ ...prev, [identity.uuid]: contacts.length })))
          .catch(() => {});
        invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid: identity.uuid })
          .then(relays => setRelayCounts(prev => ({ ...prev, [identity.uuid]: relays.length })))
          .catch(() => {});
      });
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
    setUnlocking(null);
    setSelectedUuid(null);
    setActiveForm(null);
    loadData();
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSelectRow = (uuid: string) => {
    if (uuid !== selectedUuid) {
      setActiveForm(null);
      setRenameError('');
      setPassphraseError('');
      setPassphraseSuccess('');
    }
    setSelectedUuid(uuid);
  };

  const toggleForm = (form: 'rename' | 'passphrase' | 'publickey', targetUuid?: string) => {
    const uuid = targetUuid ?? selectedUuid;
    if (activeForm === form && selectedUuid === uuid) {
      setActiveForm(null);
      return;
    }
    setActiveForm(form);
    if (form === 'rename') {
      const identity = identities.find(i => i.uuid === uuid);
      setRenamingValue(identity?.displayName ?? '');
      setRenameError('');
    }
    if (form === 'passphrase') {
      setCurrentPassphrase('');
      setNewPassphrase('');
      setConfirmNewPassphrase('');
      setPassphraseError('');
      setPassphraseSuccess('');
    }
    if (form === 'publickey') {
      setPublicKey('');
      setFingerprint('');
      setPublicKeyCopied(false);
      invoke<{ publicKey: string; fingerprint: string }>('get_identity_public_key', { identityUuid: uuid })
        .then(info => { setPublicKey(info.publicKey); setFingerprint(info.fingerprint); })
        .catch(e => setError(String(e)));
    }
  };

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
      setSelectedUuid(null);
      setActiveForm(null);
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

  const handleSaveRename = async (identityUuid: string) => {
    if (!renamingValue.trim()) {
      setRenameError(t('identity.nameRequired'));
      return;
    }
    setSavingRename(true);
    setRenameError('');
    try {
      await invoke('rename_identity', { identityUuid, newName: renamingValue.trim() });
      setRenamingValue('');
      setActiveForm(null);
      await loadData();
    } catch (err) {
      setRenameError(String(err));
    } finally {
      setSavingRename(false);
    }
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
        setPassphraseSuccess('');
        setActiveForm(null);
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

  const handleExport = async () => {
    if (!selectedUuid) return;
    const identity = identities.find(i => i.uuid === selectedUuid);
    if (!identity) return;

    setExporting(true);
    setError('');
    try {
      const path = await save({
        filters: [{ name: 'Swarm Identity', extensions: ['swarmid'] }],
        defaultPath: `${identity.displayName}.swarmid`,
        title: t('identity.export'),
      });
      if (!path) return;

      await invoke('export_swarmid_cmd', {
        identityUuid: selectedUuid,
        path,
      });
    } catch (err) {
      setError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const handleImport = async () => {
    setError('');
    try {
      const path = await open({
        filters: [{ name: 'Swarm Identity', extensions: ['swarmid'] }],
        title: t('identity.importSwarmid'),
      });
      if (!path || typeof path !== 'string') return;

      try {
        const identityRef = await invoke<IdentityRef>('import_swarmid_cmd', { path });
        await loadData();
        setSelectedUuid(identityRef.uuid);
      } catch (err) {
        const msg = String(err);
        if (msg.startsWith('IDENTITY_EXISTS:')) {
          const confirmed = await confirm(t('identity.importOverwrite'));
          if (!confirmed) return;
          const identityRef = await invoke<IdentityRef>('import_swarmid_overwrite_cmd', { path });
          await loadData();
          setSelectedUuid(identityRef.uuid);
        } else {
          setError(msg);
        }
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const isUnlocked = (uuid: string) => unlockedIds.has(uuid);

  return (
    <>
      <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
        <div className="bg-background border border-border rounded-lg w-[520px] max-h-[80vh] flex flex-col">
          {/* Header */}
          <div className="flex items-center p-4 border-b border-border">
            <h2 className="text-xl font-bold">{t('identity.manageTitle')}</h2>
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
              <div className="space-y-1">
                {identities.map((identity) => {
                  const unlocked = isUnlocked(identity.uuid);
                  const selected = selectedUuid === identity.uuid;

                  return (
                    <div
                      key={identity.uuid}
                      onClick={() => handleSelectRow(identity.uuid)}
                      className={[
                        'flex items-center gap-3 px-3 py-2.5 rounded-md cursor-pointer select-none group',
                        selected
                          ? 'bg-primary/10 border border-primary/30'
                          : 'hover:bg-secondary border border-transparent',
                      ].join(' ')}
                    >
                      <button
                        className={`shrink-0 hover:scale-110 transition-transform ${unlocked ? 'text-green-600' : 'text-red-500'}`}
                        title={unlocked ? t('identity.lock') : t('identity.unlock')}
                        onClick={(e) => {
                          e.stopPropagation();
                          if (unlocked) {
                            handleLock(identity.uuid);
                          } else {
                            setUnlocking(identity.uuid);
                          }
                        }}
                      >
                        {unlocked ? (
                          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" className="w-5 h-5">
                            <path fillRule="evenodd" d="M14.5 1A4.5 4.5 0 0 0 10 5.5V9H3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-1.5V5.5a3 3 0 1 1 6 0v2.75a.75.75 0 0 0 1.5 0V5.5A4.5 4.5 0 0 0 14.5 1Z" clipRule="evenodd" />
                          </svg>
                        ) : (
                          <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" className="w-5 h-5">
                            <path fillRule="evenodd" d="M10 1a4.5 4.5 0 0 0-4.5 4.5V9H5a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2h-.5V5.5A4.5 4.5 0 0 0 10 1Zm3 8V5.5a3 3 0 1 0-6 0V9h6Z" clipRule="evenodd" />
                          </svg>
                        )}
                      </button>
                      <span className="flex-1 min-w-0 font-medium truncate">{identity.displayName}</span>
                      {unlocked && (
                        <div className="flex items-center gap-1 shrink-0">
                          <button
                            className="p-1 rounded hover:bg-secondary/80 text-muted-foreground hover:text-foreground"
                            title={t('identity.publicKey', 'Public Key')}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleSelectRow(identity.uuid);
                              toggleForm('publickey', identity.uuid);
                            }}
                          >
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" fill="currentColor" className="w-4 h-4">
                              <path fillRule="evenodd" d="M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Zm-7-4a1 1 0 1 1-2 0 1 1 0 0 1 2 0ZM9 9a.75.75 0 0 0 0 1.5h.253a.25.25 0 0 1 .244.304l-.459 2.066A1.75 1.75 0 0 0 10.747 15H11a.75.75 0 0 0 0-1.5h-.253a.25.25 0 0 1-.244-.304l.459-2.066A1.75 1.75 0 0 0 9.253 9H9Z" clipRule="evenodd" />
                            </svg>
                          </button>
                          <button
                            className="p-1.5 rounded hover:bg-red-500/10 text-muted-foreground hover:text-red-500 text-sm"
                            title={t('identity.delete')}
                            onClick={(e) => {
                              e.stopPropagation();
                              handleDelete(identity);
                            }}
                          >
                            🗑
                          </button>
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

          {/* Inline form area */}
          {activeForm && selectedUuid && (
            <div className="border-t border-border">
              {/* Rename form */}
              {activeForm === 'rename' && (
                <div className="px-4 pb-3 pt-2 bg-secondary/30 space-y-2">
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t('identity.displayName')}
                  </label>
                  <div className="flex gap-2">
                    <input
                      type="text"
                      value={renamingValue}
                      onChange={(e) => setRenamingValue(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') handleSaveRename(selectedUuid);
                        if (e.key === 'Escape') setActiveForm(null);
                      }}
                      className="flex-1 bg-background border border-border rounded px-2 py-1 text-sm"
                      autoFocus
                      disabled={savingRename}
                    />
                    <button
                      onClick={() => handleSaveRename(selectedUuid)}
                      className="px-3 py-1 bg-primary text-primary-foreground rounded text-sm hover:bg-primary/90 disabled:opacity-50"
                      disabled={savingRename}
                    >
                      {savingRename ? t('common.saving') : t('common.save')}
                    </button>
                    <button
                      onClick={() => setActiveForm(null)}
                      className="px-3 py-1 border border-border rounded text-sm hover:bg-secondary"
                      disabled={savingRename}
                    >
                      {t('common.cancel')}
                    </button>
                  </div>
                  {renameError && <p className="text-xs text-red-500">{renameError}</p>}
                </div>
              )}

              {/* Change passphrase form */}
              {activeForm === 'passphrase' && (
                <div className="px-4 pb-3 pt-2 bg-secondary/30 space-y-2">
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
                        if (e.key === 'Enter') handleSavePassphrase(selectedUuid);
                        if (e.key === 'Escape') setActiveForm(null);
                      }}
                      className="w-full bg-background border border-border rounded px-2 py-1 text-sm"
                      disabled={savingPassphrase}
                    />
                  </div>
                  {passphraseError && <p className="text-xs text-red-500">{passphraseError}</p>}
                  {passphraseSuccess && <p className="text-xs text-green-500">{passphraseSuccess}</p>}
                  <div className="flex justify-end gap-2 pt-1">
                    <button
                      onClick={() => setActiveForm(null)}
                      className="px-3 py-1 border border-border rounded text-sm hover:bg-secondary"
                      disabled={savingPassphrase}
                    >
                      {t('common.cancel')}
                    </button>
                    <button
                      onClick={() => handleSavePassphrase(selectedUuid)}
                      className="px-3 py-1 bg-primary text-primary-foreground rounded text-sm hover:bg-primary/90 disabled:opacity-50"
                      disabled={savingPassphrase}
                    >
                      {savingPassphrase ? t('common.saving') : t('common.save')}
                    </button>
                  </div>
                </div>
              )}

              {/* Export passphrase form */}

            </div>
          )}

          {/* Toolbar */}
          <div className="flex items-center justify-between px-4 py-3 border-t border-border gap-2 flex-wrap">
            {/* Identity-specific actions */}
            <div className="flex items-center gap-1 flex-wrap">
              {selectedUuid && isUnlocked(selectedUuid) && (
                <>
                  <button
                    onClick={() => setShowContactBook(selectedUuid)}
                    className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
                  >
                    {t('contacts.contactsButton', { count: contactCounts[selectedUuid] ?? 0 })}
                  </button>
                  <button
                    onClick={() => setShowRelayBook(selectedUuid)}
                    className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
                  >
                    {t('identityManager.relays')} ({relayCounts[selectedUuid] ?? 0})
                  </button>
                </>
              )}
              <button
                onClick={() => selectedUuid && toggleForm('rename')}
                disabled={!selectedUuid || !isUnlocked(selectedUuid)}
                className={['px-2 py-1 text-xs border border-border rounded hover:bg-secondary disabled:opacity-40 disabled:cursor-not-allowed', activeForm === 'rename' ? 'bg-secondary' : ''].join(' ')}
              >
                {t('identity.rename')}
              </button>
              <button
                onClick={() => selectedUuid && toggleForm('passphrase')}
                disabled={!selectedUuid || !isUnlocked(selectedUuid)}
                className={['px-2 py-1 text-xs border border-border rounded hover:bg-secondary disabled:opacity-40 disabled:cursor-not-allowed', activeForm === 'passphrase' ? 'bg-secondary' : ''].join(' ')}
              >
                {t('identity.changePassphrase')}
              </button>
              <button
                onClick={handleExport}
                disabled={!selectedUuid || !isUnlocked(selectedUuid) || exporting}
                className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary disabled:opacity-40 disabled:cursor-not-allowed"
              >
                {t('identity.export')}
              </button>
            </div>

            {/* Global actions */}
            <div className="flex items-center gap-1">
              <button
                onClick={() => setShowCreate(true)}
                className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
              >
                + {t('identity.create')}
              </button>
              <button
                onClick={handleImport}
                className="px-2 py-1 text-xs border border-border rounded hover:bg-secondary"
              >
                {t('identity.importSwarmid')}
              </button>
              <button
                onClick={onClose}
                className="px-3 py-1 text-xs border border-border rounded hover:bg-secondary"
              >
                {t('common.close')}
              </button>
            </div>
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

      {activeForm === 'publickey' && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[60]">
          <div className="bg-background border border-border rounded-lg w-[420px] p-5 space-y-4">
            <h3 className="text-lg font-bold">{t('identity.publicKey', 'Public Key')}</h3>
            <p className="text-xs text-muted-foreground">{t('identity.publicKeyPrompt', 'Share this with anyone who wants to invite you to their workspace.')}</p>
            <div>
              <p className="text-xs font-medium mb-1">{t('identity.fingerprintLabel', 'Fingerprint (read aloud to verify)')}</p>
              <code className="block text-sm font-mono bg-secondary border border-border rounded px-2 py-2 tracking-wide select-all">
                {fingerprint || '…'}
              </code>
            </div>
            <div>
              <p className="text-xs font-medium mb-1">{t('identity.publicKeyLabel', 'Public Key')}</p>
              <code className="block text-xs font-mono bg-secondary border border-border rounded px-2 py-2 break-all select-all">
                {publicKey || '…'}
              </code>
            </div>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setActiveForm(null)}
                className="px-3 py-1.5 border border-border rounded text-sm hover:bg-secondary"
              >
                {t('common.close')}
              </button>
              <button
                onClick={() => {
                  navigator.clipboard.writeText(publicKey).then(() => {
                    setPublicKeyCopied(true);
                    setTimeout(() => setPublicKeyCopied(false), 2000);
                  });
                }}
                disabled={!publicKey}
                className="px-3 py-1.5 bg-primary text-primary-foreground rounded text-sm hover:bg-primary/90 disabled:opacity-50"
              >
                {publicKeyCopied ? t('identity.publicKeyCopied', 'Copied!') : t('identity.copyPublicKey', 'Copy')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showContactBook && (
        <ContactBookDialog
          identityUuid={showContactBook}
          identityName={identities.find(i => i.uuid === showContactBook)?.displayName ?? ''}
          onClose={() => { setShowContactBook(null); loadData(); }}
        />
      )}

      {showRelayBook && (
        <RelayBookDialog
          identityUuid={showRelayBook}
          identityName={identities.find(i => i.uuid === showRelayBook)?.displayName ?? ''}
          onClose={() => { setShowRelayBook(null); loadData(); }}
        />
      )}
    </>
  );
}

export default IdentityManagerDialog;
