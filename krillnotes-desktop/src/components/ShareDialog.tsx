import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { PeerInfo, PermissionGrantRow } from '../types';

interface ShareDialogProps {
  open: boolean;
  noteId: string;
  noteTitle: string;
  currentUserRole: string;  // "owner" | "root_owner" — caps available roles
  onComplete: () => void;
  onClose: () => void;
}

export function ShareDialog({
  open, noteId, noteTitle, currentUserRole, onComplete, onClose,
}: ShareDialogProps) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [existingGrants, setExistingGrants] = useState<PermissionGrantRow[]>([]);
  const [search, setSearch] = useState('');
  const [selectedPeerId, setSelectedPeerId] = useState<string | null>(null);
  const [role, setRole] = useState<string>('writer');
  const [processing, setProcessing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    Promise.all([
      invoke<PeerInfo[]>('list_workspace_peers'),
      invoke<PermissionGrantRow[]>('get_note_permissions', { noteId }),
    ]).then(([p, g]) => {
      setPeers(p);
      setExistingGrants(g);
    }).catch(() => {});
  }, [open, noteId]);

  // Filter out peers who already have an explicit grant at this node
  const availablePeers = useMemo(() => {
    const grantedKeys = new Set(existingGrants.map(g => g.userId));
    return peers.filter(p =>
      !p.isSelfPeer &&
      !grantedKeys.has(p.peerIdentityId) &&
      (search === '' ||
        p.displayName.toLowerCase().includes(search.toLowerCase()) ||
        p.fingerprint?.toLowerCase().includes(search.toLowerCase()))
    );
  }, [peers, existingGrants, search]);

  if (!open) return null;

  const handleShare = async () => {
    if (!selectedPeerId) return;
    const peer = peers.find(p => p.peerIdentityId === selectedPeerId);
    if (!peer) return;

    setProcessing(true);
    setError(null);
    try {
      await invoke('set_permission', {
        noteId,
        userId: peer.peerIdentityId,
        role,
      });
      onComplete();
    } catch (e) {
      setError(String(e));
    } finally {
      setProcessing(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-[var(--color-background)] border border-[var(--color-border)] rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-1">
          {t('share.title', 'Share subtree')}
        </h2>
        <p className="text-xs text-[var(--color-muted-foreground)] mb-4">{noteTitle}</p>

        {/* Search */}
        <input
          type="text"
          placeholder={t('share.searchPeers', 'Search peers...')}
          value={search}
          onChange={e => setSearch(e.target.value)}
          className="w-full border border-[var(--color-border)] bg-[var(--color-background)] rounded px-3 py-2 text-sm mb-2"
        />

        {/* Peer list */}
        <div className="max-h-40 overflow-y-auto border border-[var(--color-border)] rounded mb-3">
          {availablePeers.length === 0 ? (
            <p className="text-xs text-[var(--color-muted-foreground)] p-3 text-center">
              {t('share.noPeers', 'No available peers')}
            </p>
          ) : (
            availablePeers.map(peer => (
              <button
                key={peer.peerIdentityId}
                onClick={() => setSelectedPeerId(peer.peerIdentityId)}
                className={`w-full text-left px-3 py-2 text-sm flex items-center gap-2 ${
                  selectedPeerId === peer.peerIdentityId
                    ? 'bg-blue-500/20'
                    : 'hover:bg-[var(--color-secondary)]'
                }`}
              >
                <span className="flex-1 truncate">{peer.displayName}</span>
                <span className="text-xs text-[var(--color-muted-foreground)] font-mono">
                  {peer.fingerprint?.slice(0, 8) ?? ''}
                </span>
              </button>
            ))
          )}
        </div>
        <p className="text-xs text-[var(--color-muted-foreground)] mb-3">
          {t('share.peerCount', '{{available}} of {{total}} peers', { available: availablePeers.length, total: peers.length })}
        </p>

        {/* Role selector */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">
            {t('share.role', 'Role')}
          </label>
          <select
            className="w-full border border-[var(--color-border)] bg-[var(--color-background)] rounded px-3 py-2"
            value={role}
            onChange={e => setRole(e.target.value)}
            disabled={processing}
          >
            {(currentUserRole === 'root_owner' || currentUserRole === 'owner') && (
              <option value="owner">{t('roles.owner', 'Owner — full control of subtree')}</option>
            )}
            <option value="writer">{t('roles.writer', 'Writer — create and edit notes')}</option>
            <option value="reader">{t('roles.reader', 'Reader — view only')}</option>
          </select>
        </div>

        {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            disabled={processing}
            className="px-4 py-2 text-sm rounded border border-[var(--color-border)] disabled:opacity-50"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={handleShare}
            disabled={processing || !selectedPeerId}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white disabled:opacity-50"
          >
            {processing ? t('common.saving', 'Saving…') : t('share.confirm', 'Share')}
          </button>
        </div>
      </div>
    </div>
  );
}
