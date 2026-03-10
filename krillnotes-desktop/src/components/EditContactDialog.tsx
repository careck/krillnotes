import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { ContactInfo, TrustLevel } from '../types';
import { TRUST_LEVELS } from '../constants';

interface EditContactDialogProps {
  identityUuid: string;
  contact: ContactInfo;
  onSaved: (contact: ContactInfo) => void;
  onDeleted: (contactId: string) => void;
  onClose: () => void;
}

export default function EditContactDialog({
  identityUuid,
  contact,
  onSaved,
  onDeleted,
  onClose,
}: EditContactDialogProps) {
  const { t } = useTranslation();
  const [localName, setLocalName] = useState(contact.localName ?? '');
  const [notes, setNotes] = useState(contact.notes ?? '');
  const [trustLevel, setTrustLevel] = useState<TrustLevel>(contact.trustLevel);
  const [fingerprintConfirmed, setFingerprintConfirmed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (trustLevel !== 'VerifiedInPerson') setFingerprintConfirmed(false);
  }, [trustLevel]);

  const needsFingerprintConfirm =
    trustLevel === 'VerifiedInPerson' && contact.trustLevel !== 'VerifiedInPerson';

  const canSave = !needsFingerprintConfirm || fingerprintConfirmed;

  async function handleSave() {
    setSaving(true);
    setError(null);
    try {
      const updated = await invoke<ContactInfo>('update_contact', {
        identityUuid,
        contactId: contact.contactId,
        localName: localName.trim() || null,
        notes: notes.trim() || null,
        trustLevel,
      });
      onSaved(updated);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    setDeleting(true);
    try {
      await invoke('delete_contact', {
        identityUuid,
        contactId: contact.contactId,
      });
      onDeleted(contact.contactId);
    } catch (e) {
      setError(String(e));
      setDeleting(false);
    }
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-[var(--color-surface)] border border-[var(--color-border)] rounded-lg p-6 w-full max-w-md shadow-xl">
        <h2 className="text-lg font-semibold mb-1">{t('contacts.editContact')}</h2>
        <p className="text-sm text-[var(--color-text-muted)] mb-4">
          {t('contacts.declaredNameLabel')} <span className="font-medium">{contact.declaredName}</span>
        </p>

        <div className="space-y-4">
          {/* Read-only: fingerprint + public key */}
          <div className="rounded border border-[var(--color-border)] p-3 space-y-1">
            <p className="text-xs font-medium uppercase tracking-wider text-[var(--color-text-muted)]">{t('contacts.fingerprintHeading')}</p>
            <p className="font-mono font-semibold">{contact.fingerprint}</p>
            <p className="text-xs font-mono text-[var(--color-text-muted)] break-all">{contact.publicKey}</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-1">{t('contacts.localNameOverride')}</label>
            <input
              type="text"
              value={localName}
              onChange={e => setLocalName(e.target.value)}
              placeholder={contact.declaredName}
              className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm"
            />
            <p className="text-xs text-[var(--color-text-muted)] mt-1">
              {t('contacts.localNameHint')}
            </p>
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

          {needsFingerprintConfirm && (
            <div className="rounded-lg border border-amber-400/50 bg-amber-50/10 p-4 space-y-3">
              <p className="text-sm font-medium">{t('contacts.fingerprintVerificationRequired')}</p>
              <p className="text-xs text-[var(--color-text-muted)]">
                {t('contacts.fingerprintVerificationHint')}
              </p>
              <p className="text-lg font-mono font-bold tracking-wider text-center py-2">
                {contact.fingerprint}
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

          <div>
            <label className="block text-sm font-medium mb-1">{t('contacts.notes')}</label>
            <textarea
              value={notes}
              onChange={e => setNotes(e.target.value)}
              placeholder={t('contacts.notesPlaceholder')}
              rows={3}
              className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm"
            />
          </div>

          {error && <p className="text-sm text-red-500">{error}</p>}
        </div>

        <div className="flex justify-between mt-6">
          <button
            onClick={handleDelete}
            disabled={deleting}
            className={`px-4 py-2 text-sm rounded ${
              confirmDelete
                ? 'bg-red-600 text-white hover:bg-red-700'
                : 'border border-red-400 text-red-500 hover:bg-red-50/10'
            } disabled:opacity-50`}
          >
            {deleting ? t('common.deleting') : confirmDelete ? t('contacts.confirmDeleteContact') : t('common.delete')}
          </button>
          <div className="flex gap-2">
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
    </div>
  );
}
