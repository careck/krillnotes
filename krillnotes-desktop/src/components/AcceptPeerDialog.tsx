import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { PendingPeer, ContactInfo } from '../types';

interface Props {
  identityUuid: string;
  pendingPeer: PendingPeer | null;
  onAccepted: (contact: ContactInfo) => void;
  onClose: () => void;
}

const TRUST_LEVELS = ['Tofu', 'CodeVerified', 'Vouched', 'VerifiedInPerson'];

export function AcceptPeerDialog({ identityUuid, pendingPeer, onAccepted, onClose }: Props) {
  const { t } = useTranslation();
  const [peer, setPeer] = useState<PendingPeer | null>(pendingPeer);
  const [trustLevel, setTrustLevel] = useState('Tofu');
  const [localName, setLocalName] = useState('');
  const [fingerprintConfirmed, setFingerprintConfirmed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isDuplicate, setIsDuplicate] = useState(false);

  const handleImport = async () => {
    const path = await open({ filters: [{ name: 'Swarm Response', extensions: ['swarm'] }] });
    if (!path) return;
    try {
      const result = await invoke<PendingPeer>('import_invite_response', {
        identityUuid,
        path: typeof path === 'string' ? path : path[0],
      });
      // Check if this public key is already in contacts (spec C5)
      try {
        const existing = await invoke<ContactInfo | null>('get_contact_by_public_key', {
          identityUuid,
          publicKey: result.inviteePublicKey,
        });
        setIsDuplicate(!!existing);
      } catch { setIsDuplicate(false); }
      setPeer(result);
      setFingerprintConfirmed(false);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleAccept = async () => {
    if (!peer) return;
    setLoading(true);
    setError(null);
    try {
      const contact = await invoke<ContactInfo>('accept_peer', {
        identityUuid,
        inviteePublicKey: peer.inviteePublicKey,
        declaredName: peer.inviteeDeclaredName,
        trustLevel,
        localName: localName || undefined,
      });
      onAccepted(contact);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-4">{t('invite.acceptTitle')}</h2>

        {!peer ? (
          <div className="text-center py-6">
            <p className="text-sm text-zinc-500 mb-4">{t('invite.importResponsePrompt')}</p>
            <button onClick={handleImport} className="px-4 py-2 text-sm rounded bg-blue-600 text-white">
              {t('invite.importResponse')}
            </button>
          </div>
        ) : (
          <>
            <div className="mb-4 p-3 bg-zinc-100 dark:bg-zinc-800 rounded">
              <p className="text-sm font-medium">{peer.inviteeDeclaredName}</p>
              <p className="text-xs text-zinc-500 font-mono mt-1">{peer.fingerprint}</p>
            </div>

            {isDuplicate && (
              <p className="text-sm text-amber-600 dark:text-amber-400 mb-3">
                {t('invite.duplicateContact')}
              </p>
            )}

            <p className="text-sm text-amber-600 dark:text-amber-400 mb-3">
              {t('invite.fingerprintVerifyPrompt')}
            </p>

            <label className="flex items-center gap-2 mb-4 text-sm cursor-pointer">
              <input
                type="checkbox"
                checked={fingerprintConfirmed}
                onChange={e => setFingerprintConfirmed(e.target.checked)}
              />
              {t('invite.fingerprintConfirm')}
            </label>

            <label className="block text-sm font-medium mb-1">{t('contacts.trustLevel')}</label>
            <select
              className="w-full border rounded px-3 py-2 mb-4 dark:bg-zinc-800 dark:border-zinc-700"
              value={trustLevel}
              onChange={e => setTrustLevel(e.target.value)}
            >
              {TRUST_LEVELS.map(tl => <option key={tl} value={tl}>{tl}</option>)}
            </select>

            <label className="block text-sm font-medium mb-1">{t('contacts.localName')}</label>
            <input
              type="text"
              placeholder={t('contacts.localNamePlaceholder')}
              className="w-full border rounded px-3 py-2 mb-4 dark:bg-zinc-800 dark:border-zinc-700"
              value={localName}
              onChange={e => setLocalName(e.target.value)}
            />

            {error && <p className="text-red-500 text-sm mb-3">{error}</p>}

            <div className="flex justify-end gap-2">
              <button onClick={onClose} className="px-4 py-2 text-sm rounded border dark:border-zinc-700">
                {t('common.reject')}
              </button>
              <button
                onClick={handleAccept}
                disabled={loading || !fingerprintConfirmed}
                className="px-4 py-2 text-sm rounded bg-green-600 text-white disabled:opacity-50"
              >
                {loading ? t('common.saving') : t('common.accept')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
