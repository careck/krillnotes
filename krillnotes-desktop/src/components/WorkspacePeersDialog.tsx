// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { PeerInfo, WorkspaceInfo, PendingPeer, ContactInfo, RelayAccountInfo, ReceivedResponseInfo } from '../types';
import AddPeerFromContactsDialog from './AddPeerFromContactsDialog';
import AddContactDialog from './AddContactDialog';
import { InviteManagerDialog } from './InviteManagerDialog';
import { AcceptPeerDialog } from './AcceptPeerDialog';
import { PostAcceptDialog } from './PostAcceptDialog';
import { SendSnapshotDialog } from './SendSnapshotDialog';
import AddRelayAccountDialog from './AddRelayAccountDialog';
import PendingResponsesSection from './PendingResponsesSection';
import { ChannelPicker } from './ChannelPicker';
import { OnboardPeerDialog } from './OnboardPeerDialog';
import type { ChannelType } from './ChannelPicker';

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

// Maps channel type strings to badge CSS classes
const CHANNEL_BADGE: Record<string, { label: string; class: string }> = {
  relay:  { label: 'Relay',  class: 'bg-sky-500/20 text-sky-400' },
  folder: { label: 'Folder', class: 'bg-teal-500/20 text-teal-400' },
  manual: { label: 'Manual', class: 'bg-orange-500/20 text-orange-400' },
};

// Returns Tailwind classes for the sync status dot
function syncStatusDotClass(status: string): string {
  switch (status) {
    case 'syncing':      return 'bg-blue-400';
    case 'error':
    case 'auth_expired': return 'bg-red-500';
    default:             return 'bg-gray-400';
  }
}

export default function WorkspacePeersDialog({
  identityUuid,
  workspaceInfo,
  unlockedIdentityUuid: _unlockedIdentityUuid,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);
  const [showAddFromContacts, setShowAddFromContacts] = useState(false);
  const [addContactForPeer, setAddContactForPeer] = useState<PeerInfo | null>(null);
  const [showInviteManager, setShowInviteManager] = useState(false);
  const [pendingResponsePeer, setPendingResponsePeer] = useState<PendingPeer | null>(null);
  const [postAcceptPeer, setPostAcceptPeer] = useState<{ name: string; publicKey: string } | null>(null);
  const [showSendSnapshot, setShowSendSnapshot] = useState(false);
  const [sendSnapshotFor, setSendSnapshotFor] = useState<string[]>([]);
  // Per-peer pending channel type selection (before "Configure" is clicked)
  const [pendingChannelType, setPendingChannelType] = useState<Record<string, string>>({});
  const [relayAccounts, setRelayAccounts] = useState<RelayAccountInfo[]>([]);
  // Per-peer selected relay account ID (for the dropdown)
  const [pendingRelayAccount, setPendingRelayAccount] = useState<Record<string, string>>({});
  const [resyncingPeer, setResyncingPeer] = useState<string | null>(null);
  // Share Invite Link state
  const [sharingLink, setSharingLink] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const [shareSuccess, setShareSuccess] = useState<string | null>(null);
  const [showRelaySetup, setShowRelaySetup] = useState(false);
  const [pendingShareAction, setPendingShareAction] = useState(false);

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
    // Load relay accounts for the bound identity
    if (identityUuid) {
      invoke<RelayAccountInfo[]>('list_relay_accounts', { identityUuid })
        .then(accounts => setRelayAccounts(accounts))
        .catch(() => {});
    }
  }, [loadPeers, identityUuid]);

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

  const handleUpdateChannel = async (peer: PeerInfo, channelType: string) => {
    let channelParams = '{}';
    if (channelType === 'folder') {
      const selected = await openDialog({ directory: true, multiple: false });
      if (typeof selected !== 'string') return; // user cancelled
      channelParams = JSON.stringify({ path: selected });
    }
    try {
      await invoke('update_peer_channel', {
        peerDeviceId: peer.peerDeviceId,
        channelType,
        channelParams,
      });
      // Clear the pending selection for this peer and refresh
      setPendingChannelType(prev => {
        const next = { ...prev };
        delete next[peer.peerDeviceId];
        return next;
      });
      await loadPeers();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleForceResync = async (peer: PeerInfo) => {
    setResyncingPeer(peer.peerDeviceId);
    setError(null);
    try {
      await invoke('reset_peer_watermark', { peerDeviceId: peer.peerDeviceId });
    } catch (e) {
      setError(String(e));
    } finally {
      setResyncingPeer(null);
    }
  };

  const handleShareInviteLink = async () => {
    if (!workspaceInfo) return;
    setSharingLink(true);
    setShareError(null);
    setShareSuccess(null);
    try {
      const hasRelay = await invoke<boolean>('has_relay_credentials');
      if (!hasRelay) {
        setPendingShareAction(true);
        setShowRelaySetup(true);
        setSharingLink(false);
        return;
      }
      await doShareInviteLink();
    } catch (e) {
      setShareError(String(e));
      setSharingLink(false);
    }
  };

  const doShareInviteLink = async () => {
    if (!workspaceInfo) return;
    setSharingLink(true);
    setShareError(null);
    try {
      const info = await invoke<{ relayUrl: string | null }>('share_invite_link', {
        identityUuid,
        workspaceName: workspaceInfo.filename,
        expiresInDays: 7,
      });
      if (info.relayUrl) {
        try {
          await navigator.clipboard.writeText(info.relayUrl);
          setShareSuccess(t('invite.linkCopied'));
        } catch {
          // WKWebView blocks clipboard after async — show URL as fallback
          setShareSuccess(info.relayUrl);
        }
      }
    } catch (e) {
      setShareError(String(e));
    } finally {
      setSharingLink(false);
    }
  };

  // Track which ReceivedResponse we're currently accepting (for status update after dialog)
  const [acceptingResponseId, setAcceptingResponseId] = useState<string | null>(null);

  const handleAcceptResponse = async (response: ReceivedResponseInfo) => {
    try {
      const fingerprint = await invoke<string>("get_fingerprint", {
        publicKey: response.inviteePublicKey,
      });
      setAcceptingResponseId(response.responseId);
      setPendingResponsePeer({
        inviteId: response.inviteId,
        inviteePublicKey: response.inviteePublicKey,
        inviteeDeclaredName: response.inviteeDeclaredName,
        fingerprint,
      });
    } catch (e) {
      console.error("Failed to prepare accept response:", e);
    }
  };
  const handleSendSnapshot = async (response: ReceivedResponseInfo) => {
    setSendSnapshotFor([response.inviteePublicKey]);
    setShowSendSnapshot(true);
    try {
      await invoke("update_response_status", {
        identityUuid,
        responseId: response.responseId,
        status: "snapshotSent",
      });
    } catch (e) {
      console.error("Failed to update response status:", e);
    }
  };

  const [onboardResponse, setOnboardResponse] = useState<ReceivedResponseInfo | null>(null);
  const handleOnboardPeer = (response: ReceivedResponseInfo) => {
    setOnboardResponse(response);
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
          <PendingResponsesSection
            identityUuid={identityUuid}
            workspaceId={workspaceInfo?.workspaceId}
            onAcceptResponse={handleAcceptResponse}
            onSendSnapshot={handleSendSnapshot}
            onOnboardPeer={handleOnboardPeer}
          />
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
            const channelBadge = CHANNEL_BADGE[peer.channelType] ?? { label: peer.channelType, class: 'bg-gray-500/20 text-gray-400' };
            const dotClass = syncStatusDotClass(peer.syncStatus);
            const selectedChannelType = pendingChannelType[peer.peerDeviceId] ?? peer.channelType;
            const currentFolderPath = peer.channelType === 'folder' ? (() => { try { return JSON.parse(peer.channelParams).path as string ?? null; } catch { return null; } })() : null;
            const currentRelayAccountId = peer.channelType === 'relay' ? (() => { try { return JSON.parse(peer.channelParams).relay_account_id as string ?? null; } catch { return null; } })() : null;
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
                    {peer.isOwner && (
                      <span className="text-xs px-1.5 py-0.5 rounded-full font-medium bg-amber-500/20 text-amber-400">
                        Owner
                      </span>
                    )}
                    {/* Channel type badge */}
                    <span className={`text-xs px-1.5 py-0.5 rounded-full font-medium ${channelBadge.class}`}>
                      {channelBadge.label}
                    </span>
                    {/* Sync status dot with optional tooltip */}
                    <span
                      title={peer.syncStatusDetail ?? undefined}
                      className={`inline-block w-2 h-2 rounded-full ${dotClass} shrink-0`}
                    />
                  </div>
                  <div className="text-xs text-[var(--color-muted-foreground)] font-mono mt-0.5">
                    {peer.fingerprint}
                  </div>
                  <div className="text-xs text-[var(--color-muted-foreground)] mt-0.5">
                    {formatLastSync(peer.lastSync)}
                  </div>
                  {/* Channel config controls */}
                  <div className="mt-1.5">
                    <ChannelPicker
                      selectedType={selectedChannelType as ChannelType}
                      onTypeChange={async (type) => {
                        setPendingChannelType(prev => ({ ...prev, [peer.peerDeviceId]: type }));
                        // "manual" applies immediately — no configuration needed
                        if (type === 'manual') {
                          await handleUpdateChannel(peer, type);
                        }
                      }}
                      relayAccounts={relayAccounts}
                      selectedRelayAccountId={pendingRelayAccount[peer.peerDeviceId] ?? currentRelayAccountId ?? undefined}
                      onRelayAccountSelect={async (accountId) => {
                        if (!accountId) return;
                        try {
                          await invoke('set_peer_relay', {
                            peerDeviceId: peer.peerDeviceId,
                            relayAccountId: accountId,
                          });
                          setPendingRelayAccount(prev => {
                            const next = { ...prev };
                            delete next[peer.peerDeviceId];
                            return next;
                          });
                          await loadPeers();
                        } catch (err) {
                          setError(String(err));
                        }
                      }}
                      currentFolderPath={currentFolderPath}
                      onConfigureFolder={() => handleUpdateChannel(peer, selectedChannelType)}
                    />
                  </div>
                </div>

                <div className="flex items-center gap-1 ml-2 shrink-0">
                  {peer.channelType !== 'manual' && (
                    <button
                      title={t('peers.forceResync', 'Force full resync from this peer')}
                      onClick={() => handleForceResync(peer)}
                      disabled={resyncingPeer === peer.peerDeviceId}
                      className="p-1.5 rounded hover:bg-[var(--color-secondary)] text-[var(--color-muted-foreground)] hover:text-amber-400 text-sm disabled:opacity-40"
                    >
                      ↺
                    </button>
                  )}
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

        {/* Footer buttons */}
        {shareSuccess && (
          shareSuccess.startsWith('http') ? (
            <div className="px-4 pb-1">
              <p className="text-xs text-green-500 mb-1">{t('invite.linkCopied')}</p>
              <input readOnly value={shareSuccess} className="w-full text-xs font-mono p-1 rounded border border-[var(--color-border)] bg-[var(--color-background)] select-all" onClick={e => (e.target as HTMLInputElement).select()} />
            </div>
          ) : (
            <p className="px-4 pb-1 text-xs text-green-500">{shareSuccess}</p>
          )
        )}
        {shareError && (
          <p className="px-4 pb-1 text-xs text-red-500">{shareError}</p>
        )}
        <div className="flex flex-wrap items-center gap-2 p-4 border-t border-[var(--color-border)]">
          <button
            onClick={() => setShowAddFromContacts(true)}
            className="whitespace-nowrap px-3 py-1.5 text-sm font-medium bg-blue-600 text-white rounded-md hover:bg-blue-700"
          >
            {t('peers.addFromContacts', '＋ Add from Contacts')}
          </button>
          <button
            onClick={() => setShowInviteManager(true)}
            className="whitespace-nowrap px-3 py-1.5 text-sm rounded-md border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
          >
            {t('invite.manageInvites')}
          </button>
          <button
            onClick={handleShareInviteLink}
            disabled={sharingLink || !workspaceInfo}
            className="whitespace-nowrap px-3 py-1.5 text-sm rounded-md border border-[var(--color-border)] hover:bg-[var(--color-secondary)] disabled:opacity-40"
          >
            {sharingLink ? t('invite.sharing') : t('invite.shareInviteLink')}
          </button>
          <button
            onClick={() => {
              setSendSnapshotFor(peers.map(p => p.peerIdentityId));
              setShowSendSnapshot(true);
            }}
            className="whitespace-nowrap px-3 py-1.5 text-sm rounded-md border border-[var(--color-border)] hover:bg-[var(--color-secondary)]"
          >
            Create Snapshot…
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

      {showInviteManager && workspaceInfo && (
        <InviteManagerDialog
          identityUuid={identityUuid}
          workspaceName={workspaceInfo.filename}
          onClose={() => setShowInviteManager(false)}
        />
      )}
      {pendingResponsePeer !== null && (
        <AcceptPeerDialog
          identityUuid={identityUuid}
          pendingPeer={pendingResponsePeer}
          onAccepted={async (contact: ContactInfo) => {
            setPendingResponsePeer(null);
            loadPeers();
            const peerName = contact.localName || contact.declaredName;
            setPostAcceptPeer({ name: peerName, publicKey: contact.publicKey });
            // Update ReceivedResponse status if this was triggered by polling
            if (acceptingResponseId) {
              try {
                await invoke("update_response_status", {
                  identityUuid,
                  responseId: acceptingResponseId,
                  status: "peerAdded",
                });
              } catch (e) {
                console.error("Failed to update response status:", e);
              }
              setAcceptingResponseId(null);
            }
          }}
          onClose={() => { setPendingResponsePeer(null); setAcceptingResponseId(null); }}
        />
      )}

      <PostAcceptDialog
        open={postAcceptPeer !== null}
        peerName={postAcceptPeer?.name ?? ''}
        onSendNow={() => {
          setSendSnapshotFor([postAcceptPeer!.publicKey]);
          setPostAcceptPeer(null);
          setShowSendSnapshot(true);
        }}
        onLater={() => setPostAcceptPeer(null)}
      />

      <SendSnapshotDialog
        open={showSendSnapshot}
        identityUuid={identityUuid}
        preSelectedPublicKeys={sendSnapshotFor}
        onClose={() => setShowSendSnapshot(false)}
        onSuccess={() => {}}
      />

      {showRelaySetup && (
        <AddRelayAccountDialog
          identityUuid={identityUuid}
          onClose={() => {
            setShowRelaySetup(false);
            setPendingShareAction(false);
          }}
          onCreated={async () => {
            setShowRelaySetup(false);
            if (pendingShareAction) {
              setPendingShareAction(false);
              await doShareInviteLink();
            }
          }}
        />
      )}

      {onboardResponse && (
        <OnboardPeerDialog
          open={true}
          response={onboardResponse}
          identityUuid={identityUuid}
          onComplete={() => { setOnboardResponse(null); loadPeers(); }}
          onClose={() => setOnboardResponse(null)}
        />
      )}

    </div>
  );
}
