import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { TrustLevel, ContactInfo } from '../types';
import { TRUST_LEVELS } from '../constants';

interface AddContactDialogProps {
  identityUuid: string;
  onSaved: (contact: ContactInfo) => void;
  onClose: () => void;
}

export default function AddContactDialog({ identityUuid, onSaved, onClose }: AddContactDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [trustLevel, setTrustLevel] = useState<TrustLevel>('Tofu');
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [fingerprintConfirmed, setFingerprintConfirmed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Live fingerprint preview as public key is entered
  useEffect(() => {
    if (publicKey.trim().length < 10) {
      setFingerprint(null);
      return;
    }
    invoke<string>('get_fingerprint', { publicKey: publicKey.trim() })
      .then(fp => setFingerprint(fp))
      .catch(() => setFingerprint(null));
  }, [publicKey]);

  // Reset fingerprint confirmation when trust level changes away from VerifiedInPerson
  useEffect(() => {
    if (trustLevel !== 'VerifiedInPerson') setFingerprintConfirmed(false);
  }, [trustLevel]);

  const canSave =
    name.trim().length > 0 &&
    publicKey.trim().length > 0 &&
    fingerprint !== null &&
    (trustLevel !== 'VerifiedInPerson' || fingerprintConfirmed);

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      const contact = await invoke<ContactInfo>('create_contact', {
        identityUuid,
        declaredName: name.trim(),
        publicKey: publicKey.trim(),
        trustLevel,
      });
      onSaved(contact);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-[var(--color-surface)] border border-[var(--color-border)] rounded-lg p-6 w-full max-w-md shadow-xl">
        <h2 className="text-lg font-semibold mb-4">{t('contacts.addContact')}</h2>

        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium mb-1">{t('contacts.contactName')}</label>
            <input
              type="text"
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder={t('contacts.namePlaceholder')}
              className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-1">{t('contacts.publicKey')}</label>
            <textarea
              value={publicKey}
              onChange={e => setPublicKey(e.target.value)}
              placeholder={t('contacts.publicKeyPlaceholder')}
              rows={3}
              className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm font-mono"
            />
            {fingerprint && (
              <p className="mt-1 text-xs text-[var(--color-text-muted)] font-mono">
                {t('contacts.fingerprintLabel')} <span className="font-semibold">{fingerprint}</span>
              </p>
            )}
          </div>

          <div>
            <label className="block text-sm font-medium mb-1">{t('contacts.trustLevel')}</label>
            <select
              value={trustLevel}
              onChange={e => setTrustLevel(e.target.value as TrustLevel)}
              className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm"
            >
              {TRUST_LEVELS.map(tl => (
                <option key={tl.value} value={tl.value}>{t(tl.labelKey)}</option>
              ))}
            </select>
          </div>

          {trustLevel === 'VerifiedInPerson' && fingerprint && (
            <div className="rounded-lg border border-amber-400/50 bg-amber-50/10 p-4 space-y-3">
              <p className="text-sm font-medium">{t('contacts.fingerprintVerificationRequired')}</p>
              <p className="text-xs text-[var(--color-text-muted)]">
                {t('contacts.fingerprintVerificationHint')}
              </p>
              <p className="text-lg font-mono font-bold tracking-wider text-center py-2">
                {fingerprint}
              </p>
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="checkbox"
                  checked={fingerprintConfirmed}
                  onChange={e => setFingerprintConfirmed(e.target.checked)}
                  className="rounded"
                />
                {t('contacts.fingerprintMatches')}
              </label>
            </div>
          )}

          {error && (
            <p className="text-sm text-red-500">{error}</p>
          )}
        </div>

        <div className="flex justify-end gap-2 mt-6">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded border border-[var(--color-border)] hover:bg-[var(--color-hover)]"
          >
            {t('common.cancel')}
          </button>
          <button
            onClick={handleSave}
            disabled={!canSave || saving}
            className="px-4 py-2 text-sm rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {saving ? t('common.saving') : t('contacts.saveContact')}
          </button>
        </div>
      </div>
    </div>
  );
}
