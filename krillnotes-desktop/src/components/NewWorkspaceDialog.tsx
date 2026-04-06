// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { WorkspaceInfo, IdentityRef } from '../types';
import { slugify } from '../utils/slugify';

interface NewWorkspaceDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

function NewWorkspaceDialog({ isOpen, onClose }: NewWorkspaceDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  const [identities, setIdentities] = useState<IdentityRef[]>([]);
  const [selectedIdentity, setSelectedIdentity] = useState<string>('');

  useEffect(() => {
    if (!isOpen) return;
    setName('');
    setError('');
    setCreating(false);

    Promise.all([
      invoke<IdentityRef[]>('list_identities'),
      invoke<string[]>('get_unlocked_identities'),
    ]).then(([ids, unlocked]) => {
      const unlockedIdentities = ids.filter(i => unlocked.includes(i.uuid));
      setIdentities(unlockedIdentities);
      if (unlockedIdentities.length > 0 && !selectedIdentity) {
        setSelectedIdentity(unlockedIdentities[0].uuid);
      } else if (unlockedIdentities.length === 0) {
        setSelectedIdentity('');
      }
    }).catch(err => setError(t('settings.failedLoad', { error: String(err) })));
  }, [isOpen]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !creating) onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose, creating]);

  if (!isOpen) return null;

  const handleCreate = async () => {
    const trimmed = name.trim();
    if (!trimmed) { setError(t('workspace.nameRequired')); return; }
    const slug = slugify(trimmed);
    if (!slug) { setError(t('workspace.nameInvalid')); return; }
    if (!selectedIdentity) { setError(t('identity.noUnlockedIdentities')); return; }

    setCreating(true);
    setError('');
    try {
      await invoke<WorkspaceInfo>('create_workspace', {
        name: slug,
        identityUuid: selectedIdentity,
      });
      onClose();
    } catch (err) {
      if (err !== 'focused_existing') {
        setError(`${err}`);
      }
      setCreating(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-secondary p-6 rounded-lg w-96">
        <h2 className="text-xl font-bold mb-4">{t('workspace.newTitle')}</h2>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">{t('workspace.nameLabel')}</label>
          <input
            type="text"
            value={name}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && !creating && handleCreate()}
            placeholder={t('workspace.namePlaceholder')}
            className="w-full bg-secondary border border-secondary rounded px-3 py-2"
            autoFocus
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            disabled={creating}
          />
          {name.trim() && (
            <p className="text-xs text-muted-foreground mt-1">
              {t('workspace.savedAs', { name: slugify(name.trim()) || '...' })}
            </p>
          )}
        </div>

        <div className="mb-4">
          <label className="block text-sm font-medium mb-2">{t('identity.selectIdentity')}</label>
          {identities.length > 0 ? (
            <select
              value={selectedIdentity}
              onChange={e => setSelectedIdentity(e.target.value)}
              className="w-full bg-secondary border border-secondary rounded px-3 py-2"
              disabled={creating}
            >
              {identities.map(i => (
                <option key={i.uuid} value={i.uuid}>{i.displayName}</option>
              ))}
            </select>
          ) : (
            <p className="text-sm text-muted-foreground p-2 bg-secondary/30 rounded border border-secondary">
              {t('identity.noUnlockedIdentities')}
            </p>
          )}
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
            disabled={creating}
          >
            {t('common.cancel')}
          </button>
          <button
            onClick={handleCreate}
            className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={creating || !name.trim() || !selectedIdentity}
          >
            {creating ? t('common.creating') : t('common.create')}
          </button>
        </div>
      </div>
    </div>
  );
}

export default NewWorkspaceDialog;
