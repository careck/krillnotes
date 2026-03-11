// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { PeerInfo, WorkspaceInfo } from '../types';
import AddPeerFromContactsDialog from './AddPeerFromContactsDialog';
import AddContactDialog from './AddContactDialog';

interface Props {
  identityUuid: string;
  workspaceInfo: WorkspaceInfo | null;
  unlockedIdentityUuid: string | null;
  onClose: () => void;
}

// Maps trust level strings to badge CSS classes (matches ContactBookDialog style)
const TRUST_BADGE: Record<string, { label: string; class: string }> = {
  Tofu:             { label: 'TOFU',     class: 'bg-gray-500/20 text-gray-400' },
  CodeVerified:     { label: 'Code',     class: 'bg-blue-500/20 text-blue-400' },
  Vouched:          { label: 'Vouched',  class: 'bg-purple-500/20 text-purple-400' },
  VerifiedInPerson: { label: 'Verified', class: 'bg-green-500/20 text-green-400' },
};

export default function WorkspacePeersDialog({
  identityUuid,
  workspaceInfo,
  unlockedIdentityUuid,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [showAddFromContacts, setShowAddFromContacts] = useState(false);
  const [addContactForPeer, setAddContactForPeer] = useState<PeerInfo | null>(null);

  // Invite form state
  const [showInviteForm, setShowInviteForm] = useState(false);
  const [inviteContactName, setInviteContactName] = useState('');
  const [invitePublicKey, setInvitePublicKey] = useState('');
  const [inviteRole, setInviteRole] = useState('writer');
  const [inviteCreating, setInviteCreating] = useState(false);
  const [inviteError, setInviteError] = useState('');
  const [inviteSuccess, setInviteSuccess] = useState('');

  const loadPeers = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<PeerInfo[]>('list_workspace_peers');
      setPeers(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPeers();
  }, [loadPeers]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  const handleRemove = async (peer: PeerInfo) => {
    if (confirmRemoveId !== peer.peerDeviceId) {
      setConfirmRemoveId(peer.peerDeviceId);
      return;
    }
    try {
      await invoke('remove_workspace_peer', { peerDeviceId: peer.peerDeviceId });
      setConfirmRemoveId(null);
      await loadPeers();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleCreateInvite = async () => {
    if (!workspaceInfo) { setInviteError(t('swarm.noWorkspace', 'No workspace open')); return; }
    if (!unlockedIdentityUuid) { setInviteError(t('swarm.identityLocked')); return; }
    if (!inviteContactName.trim()) { setInviteError(t('swarm.contactNameLabel') + ' required'); return; }
    if (!invitePublicKey.trim()) { setInviteError(t('swarm.publicKeyLabel') + ' required'); return; }

    const savePath = await save({
      filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
      defaultPath: `invite-${inviteContactName.trim().replace(/\s+/g, '-')}.swarm`,
    });
    if (!savePath) { setInviteError(t('swarm.saveCancelled')); return; }

    setInviteCreating(true);
    setInviteError('');
    try {
      await invoke('create_invite_bundle_cmd', {
        workspaceId: workspaceInfo.path,
        workspaceName: workspaceInfo.filename,
        contactName: inviteContactName.trim(),
        contactPublicKey: invitePublicKey.trim(),
        offeredRole: inviteRole,
        offeredScope: null,
        sourceDeviceId: unlockedIdentityUuid,
        identityUuid: unlockedIdentityUuid,
        savePath,
      });
      setInviteSuccess(t('swarm.inviteSaved', { name: inviteContactName.trim() }));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setInviteError(msg === 'IDENTITY_LOCKED' ? t('swarm.identityLocked') : msg);
    } finally {
      setInviteCreating(false);
    }
  };

  const resetInviteForm = () => {
    setShowInviteForm(false);
    setInviteContactName('');
    setInvitePublicKey('');
    setInviteRole('writer');
    setInviteCreating(false);
    setInviteError('');
    setInviteSuccess('');
  };

  const formatLastSync = (lastSync?: string) => {
    if (!lastSync) return t('peers.neverSynced', 'Never synced');
    const d = new Date(lastSync);
    const diff = Date.now() - d.getTime();
    const minutes = Math.floor(diff / 60000);
    if (minutes < 1) return t('peers.justNow', 'just now');
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return d.toLocaleDateString();
  };

  return (
    <div className="fixed inset-0 z-60 flex items-center justify-center bg-black/50">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-lg shadow-xl w-[520px] max-h-[600px] flex flex-col">

        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
          <div>
            <h2 className="text-lg font-semibold">{t('peers.title', 'Workspace Peers')}</h2>
            <p className="text-xs text-[var(--color-muted-foreground)]">
              {peers.length} {peers.length === 1 ? t('peers.peer', 'peer') : t('peers.peers', 'peers')}
            </p>
          </div>
          <button
            onClick={onClose}
            className="text-[var(--color-muted-foreground)] hover:text-[var(--color-foreground)] px-2"
          >
            ✕
          </button>
        </div>

        {/* Peer list */}
        <div className="flex-1 overflow-y-auto p-4 space-y-2">
          {loading && (
            <p className="text-sm text-[var(--color-muted-foreground)] text-center py-8">{t('common.loading')}</p>
          )}
          {!loading && peers.length === 0 && (
            <p className="text-sm text-[var(--color-muted-foreground)] text-center py-8">
              {t('peers.noPeers', 'No peers yet. Add a peer from your contacts or create an invite file.')}
            </p>
          )}
          {error && (
            <p className="text-sm text-red-500 p-2 rounded bg-red-500/10">{error}</p>
          )}
          {peers.map((peer) => {
            const badge = peer.trustLevel ? (TRUST_BADGE[peer.trustLevel] ?? TRUST_BADGE.Tofu) : null;
            return (
              <div
                key={peer.peerDeviceId}
                className="flex items-center justify-between p-3 rounded-md border border-[var(--color-border)] bg-[var(--color-secondary)]/30"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium text-sm truncate">{peer.displayName}</span>
                    {badge ? (
                      <span className={`text-xs px-1.5 py-0.5 rounded-full font-medium ${badge.class}`}>
                        {badge.label}
                      </span>
                    ) : (
                      <span className="text-xs text-[var(--color-muted-foreground)] italic">
                        {t('peers.notInContacts', 'not in contacts')}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-[var(--color-muted-foreground)] font-mono mt-0.5">
                    {peer.fingerprint}
                  </div>
                  <div className="text-xs text-[var(--color-muted-foreground)] mt-0.5">
                    {formatLastSync(peer.lastSync)}
                  </div>
                </div>

                <div className="flex items-center gap-1 ml-2 shrink-0">
                  {!peer.contactId && (
                    <button
                      title={t('peers.addToContacts', 'Add to contacts')}
                      onClick={() => setAddContactForPeer(peer)}
                      className="p-1.5 rounded hover:bg-[var(--color-secondary)] text-blue-500 text-sm"
                    >
                      ＋
                    </button>
                  )}

                  {confirmRemoveId === peer.peerDeviceId ? (
                    <div className="flex items-center gap-1">
                      <span className="text-xs text-red-500">{t('peers.confirmRemove', 'Remove?')}</span>
                      <button
                        onClick={() => handleRemove(peer)}
                        className="text-xs px-2 py-1 bg-red-500 text-white rounded hover:bg-red-600"
                      >
                        {t('common.remove', 'Remove')}
                      </button>
                      <button
                        onClick={() => setConfirmRemoveId(null)}
                        className="text-xs px-2 py-1 rounded hover:bg-[var(--color-secondary)]"
                      >
                        {t('common.cancel')}
                      </button>
                    </div>
                  ) : (
                    <button
                      title={t('peers.removePeer', 'Remove peer')}
                      onClick={() => handleRemove(peer)}
                      className="p-1.5 rounded hover:bg-[var(--color-secondary)] text-[var(--color-muted-foreground)] hover:text-red-500 text-sm"
                    >
                      🗑
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        {/* Invite form (collapsible) */}
        {showInviteForm && (
          <div className="border-t border-[var(--color-border)] p-4 space-y-3">
            <h3 className="text-sm font-semibold">{t('swarm.inviteDialogTitle')}</h3>
            <div>
              <label className="block text-xs font-medium mb-1">{t('swarm.contactNameLabel')}</label>
              <input
                className="w-full border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                value={inviteContactName}
                onChange={e => setInviteContactName(e.target.value)}
                placeholder={t('swarm.contactNamePlaceholder')}
                disabled={inviteCreating}
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">{t('swarm.publicKeyLabel')}</label>
              <textarea
                className="w-full border border-[var(--color-border)] rounded px-3 py-1.5 text-xs font-mono bg-[var(--color-background)]"
                rows={2}
                value={invitePublicKey}
                onChange={e => setInvitePublicKey(e.target.value)}
                placeholder={t('swarm.publicKeyPlaceholder')}
                disabled={inviteCreating}
              />
            </div>
            <div>
              <label className="block text-xs font-medium mb-1">{t('swarm.roleLabel')}</label>
              <select
                className="w-full border border-[var(--color-border)] rounded px-3 py-1.5 text-sm bg-[var(--color-background)]"
                value={inviteRole}
                onChange={e => setInviteRole(e.target.value)}
                disabled={inviteCreating}
              >
                <option value="owner">{t('swarm.roleOwner')}</option>
                <option value="writer">{t('swarm.roleWriter')}</option>
                <option value="reader">{t('swarm.roleReader')}</option>
              </select>
            </div>
            {inviteError && <p className="text-xs text-red-500">{inviteError}</p>}
            {inviteSuccess && <p className="text-xs text-green-600">{inviteSuccess}</p>}
            <div className="flex justify-end gap-2">
              <button
                onClick={resetInviteForm}
                className="px-3 py-1.5 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
                disabled={inviteCreating}
              >
                {inviteSuccess ? t('common.close', 'Close') : t('common.cancel')}
              </button>
              {!inviteSuccess && (
                <button
                  onClick={handleCreateInvite}
                  disabled={inviteCreating || !inviteContactName.trim() || !invitePublicKey.trim()}
                  className="px-3 py-1.5 text-sm rounded bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50"
                >
                  {inviteCreating ? '…' : t('swarm.createInviteButton')}
                </button>
              )}
            </div>
          </div>
        )}

        {/* Footer buttons */}
        <div className="flex items-center gap-2 p-4 border-t border-[var(--color-border)]">
          <button
            onClick={() => setShowAddFromContacts(true)}
            className="px-3 py-2 text-sm font-medium bg-blue-600 text-white rounded-md hover:bg-blue-700"
          >
            {t('peers.addFromContacts', '＋ Add from Contacts')}
          </button>
          <button
            onClick={() => { resetInviteForm(); setShowInviteForm(true); }}
            className="px-3 py-2 text-sm rounded-md border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
          >
            📨 {t('peers.createInviteFile', 'Create Invite File')}
          </button>
        </div>
      </div>

      {showAddFromContacts && (
        <AddPeerFromContactsDialog
          identityUuid={identityUuid}
          currentPeers={peers}
          onAdded={async () => {
            setShowAddFromContacts(false);
            await loadPeers();
          }}
          onClose={() => setShowAddFromContacts(false)}
        />
      )}

      {addContactForPeer && (
        <AddContactDialog
          identityUuid={identityUuid}
          prefillPublicKey={addContactForPeer.peerIdentityId}
          onSaved={() => {
            setAddContactForPeer(null);
            loadPeers();
          }}
          onClose={() => setAddContactForPeer(null)}
        />
      )}
    </div>
  );
}
