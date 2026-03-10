import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ContactInfo, PeerInfo } from '../types';

interface Props {
  identityUuid: string;
  currentPeers: PeerInfo[];
  onAdded: () => void;
  onClose: () => void;
}

export default function AddPeerFromContactsDialog({
  identityUuid,
  currentPeers,
  onAdded,
  onClose,
}: Props) {
  const [contacts, setContacts] = useState<ContactInfo[]>([]);
  const [selected, setSelected] = useState<ContactInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!identityUuid) return;
    setLoading(true);
    invoke<ContactInfo[]>('list_contacts', { identityUuid })
      .then((all) => {
        const peerKeys = new Set(currentPeers.map((p) => p.peerIdentityId));
        setContacts(all.filter((c) => !peerKeys.has(c.publicKey)));
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [identityUuid, currentPeers]);

  const handleSave = async () => {
    if (!selected) return;
    setSaving(true);
    setError(null);
    try {
      await invoke('add_contact_as_peer', {
        identityUuid,
        contactId: selected.contactId,
      });
      onAdded();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  const displayName = (c: ContactInfo) =>
    c.localName ?? c.declaredName;

  return (
    <div className="fixed inset-0 z-70 flex items-center justify-center bg-black/50">
      <div className="bg-background border border-border rounded-lg shadow-xl w-[400px] max-h-[480px] flex flex-col">

        <div className="flex items-center justify-between p-4 border-b border-border">
          <h2 className="text-base font-semibold">Add contact as peer</h2>
          <button onClick={onClose} className="text-muted-foreground hover:text-foreground">✕</button>
        </div>

        <div className="flex-1 overflow-y-auto p-4 space-y-1.5">
          {loading && (
            <p className="text-sm text-muted-foreground text-center py-6">Loading contacts…</p>
          )}
          {!loading && contacts.length === 0 && (
            <p className="text-sm text-muted-foreground text-center py-6">
              All contacts are already peers, or you have no contacts.
            </p>
          )}
          {error && (
            <p className="text-sm text-red-500">{error}</p>
          )}
          {contacts.map((c) => (
            <button
              key={c.contactId}
              onClick={() => setSelected(selected?.contactId === c.contactId ? null : c)}
              className={`w-full text-left p-3 rounded-md border transition-colors ${
                selected?.contactId === c.contactId
                  ? 'border-primary bg-primary/10'
                  : 'border-border hover:bg-muted/50'
              }`}
            >
              <div className="text-sm font-medium">{displayName(c)}</div>
              <div className="text-xs text-muted-foreground font-mono mt-0.5">{c.fingerprint}</div>
            </button>
          ))}
        </div>

        <div className="flex items-center justify-end gap-2 p-4 border-t border-border">
          <button
            onClick={onClose}
            className="px-3 py-2 text-sm rounded-md border border-border hover:bg-muted"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!selected || saving}
            className="px-3 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-md hover:bg-primary/90 disabled:opacity-50"
          >
            {saving ? 'Adding…' : 'Add as peer'}
          </button>
        </div>
      </div>
    </div>
  );
}
