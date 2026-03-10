import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { ContactInfo } from '../types';
import AddContactDialog from './AddContactDialog';
import EditContactDialog from './EditContactDialog';

interface ContactBookDialogProps {
  identityUuid: string;
  identityName: string;
  onClose: () => void;
}

const TRUST_BADGE: Record<string, { label: string; class: string }> = {
  Tofu:             { label: 'TOFU',     class: 'bg-gray-500/20 text-gray-400' },
  CodeVerified:     { label: 'Code',     class: 'bg-blue-500/20 text-blue-400' },
  Vouched:          { label: 'Vouched',  class: 'bg-purple-500/20 text-purple-400' },
  VerifiedInPerson: { label: 'Verified', class: 'bg-green-500/20 text-green-400' },
};

export default function ContactBookDialog({ identityUuid, identityName, onClose }: ContactBookDialogProps) {
  const { t } = useTranslation();
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<ContactInfo | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<ContactInfo[]>('list_contacts', { identityUuid });
      setContacts(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [identityUuid]);

  useEffect(() => { load(); }, [load]);

  const filtered = contacts.filter(c => {
    const q = search.toLowerCase();
    return (
      (c.localName ?? c.declaredName).toLowerCase().includes(q) ||
      c.publicKey.toLowerCase().startsWith(q)
    );
  });

  function handleSaved(contact: ContactInfo) {
    setContacts(prev => {
      const idx = prev.findIndex(c => c.contactId === contact.contactId);
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = contact;
        return next.sort((a, b) =>
          (a.localName ?? a.declaredName).localeCompare(b.localName ?? b.declaredName)
        );
      }
      return [...prev, contact].sort((a, b) =>
        (a.localName ?? a.declaredName).localeCompare(b.localName ?? b.declaredName)
      );
    });
    setShowAdd(false);
    setEditing(null);
  }

  function handleDeleted(contactId: string) {
    setContacts(prev => prev.filter(c => c.contactId !== contactId));
    setEditing(null);
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-60">
      <div className="bg-[var(--color-surface)] border border-[var(--color-border)] rounded-lg w-full max-w-lg shadow-xl flex flex-col" style={{ maxHeight: '80vh' }}>
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-[var(--color-border)]">
          <div>
            <h2 className="text-lg font-semibold">{t('contacts.title')}</h2>
            <p className="text-xs text-[var(--color-text-muted)]">{identityName}</p>
          </div>
          <button
            onClick={onClose}
            className="text-[var(--color-text-muted)] hover:text-[var(--color-text)] px-2"
          >
            ✕
          </button>
        </div>

        {/* Search + Add */}
        <div className="flex gap-2 p-3 border-b border-[var(--color-border)]">
          <input
            type="text"
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder={t('contacts.searchContacts')}
            className="flex-1 px-3 py-1.5 rounded border border-[var(--color-border)] bg-[var(--color-input)] text-sm"
          />
          <button
            onClick={() => setShowAdd(true)}
            className="px-3 py-1.5 text-sm rounded bg-blue-600 text-white hover:bg-blue-700 whitespace-nowrap"
          >
            {t('common.add')}
          </button>
        </div>

        {/* Contact list */}
        <div className="overflow-y-auto flex-1">
          {loading && (
            <p className="text-sm text-center text-[var(--color-text-muted)] py-8">{t('common.loading')}</p>
          )}
          {!loading && error && (
            <p className="text-sm text-center text-red-500 py-8">{error}</p>
          )}
          {!loading && !error && filtered.length === 0 && (
            <p className="text-sm text-center text-[var(--color-text-muted)] py-8">
              {search ? t('contacts.noContactsMatch') : t('contacts.noContacts')}
            </p>
          )}
          {filtered.map(contact => {
            const badge = TRUST_BADGE[contact.trustLevel] ?? TRUST_BADGE.Tofu;
            const displayName = contact.localName ?? contact.declaredName;
            return (
              <button
                key={contact.contactId}
                onClick={() => setEditing(contact)}
                className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-[var(--color-hover)] border-b border-[var(--color-border)] last:border-0"
              >
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium truncate">{displayName}</p>
                  <p className="text-xs font-mono text-[var(--color-text-muted)] truncate">{contact.fingerprint}</p>
                </div>
                <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${badge.class}`}>
                  {badge.label}
                </span>
              </button>
            );
          })}
        </div>

        {/* Footer count */}
        <div className="px-4 py-2 border-t border-[var(--color-border)] text-xs text-[var(--color-text-muted)]">
          {t('contacts.contactCount', { count: contacts.length })}
        </div>
      </div>

      {/* Sub-dialogs */}
      {showAdd && (
        <AddContactDialog
          identityUuid={identityUuid}
          onSaved={handleSaved}
          onClose={() => setShowAdd(false)}
        />
      )}
      {editing && (
        <EditContactDialog
          key={editing.contactId}
          identityUuid={identityUuid}
          contact={editing}
          onSaved={handleSaved}
          onDeleted={handleDeleted}
          onClose={() => setEditing(null)}
        />
      )}
    </div>
  );
}
