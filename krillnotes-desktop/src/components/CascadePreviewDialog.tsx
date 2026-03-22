import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { CascadeImpactRow } from '../types';

interface CascadePreviewDialogProps {
  open: boolean;
  noteId: string;
  userId: string;
  userName: string;
  action: 'demote' | 'revoke';
  newRole?: string;
  oldRole: string;
  noteTitle: string;
  onConfirm: (revokeGrants: Array<{ noteId: string; userId: string }>) => void;
  onClose: () => void;
}

export function CascadePreviewDialog({
  open, noteId, userId, userName, action, newRole, oldRole, noteTitle,
  onConfirm, onClose,
}: CascadePreviewDialogProps) {
  const { t } = useTranslation();
  const [impacts, setImpacts] = useState<CascadeImpactRow[]>([]);
  const [nameMap, setNameMap] = useState<Record<string, string>>({});
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    const effectiveNewRole = action === 'revoke' ? 'none' : (newRole ?? 'none');
    invoke<CascadeImpactRow[]>('preview_cascade', {
      noteId, userId, newRole: effectiveNewRole,
    })
      .then(async (rows) => {
        setImpacts(rows);
        setChecked(new Set(rows.map(r => `${r.grant.noteId}:${r.grant.userId}`)));

        const keys = new Set(rows.map(r => r.grant.userId));
        const names: Record<string, string> = {};
        await Promise.all(
          Array.from(keys).map(async (key) => {
            try {
              names[key] = await invoke<string>('resolve_identity_name', { publicKey: key });
            } catch {
              names[key] = key.slice(0, 8) + '…';
            }
          })
        );
        setNameMap(names);
      })
      .catch(() => setImpacts([]))
      .finally(() => setLoading(false));
  }, [open, noteId, userId, action, newRole]);

  if (!open) return null;

  const grantKey = (g: CascadeImpactRow) => `${g.grant.noteId}:${g.grant.userId}`;

  const toggleCheck = (key: string) => {
    setChecked(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const actionLabel = action === 'revoke'
    ? t('cascade.revoking', 'Revoking access for')
    : t('cascade.demoting', 'Demoting');

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-xl shadow-xl p-6 w-full max-w-md">
        <h2 className="text-lg font-semibold mb-2">
          {actionLabel} {userName}
        </h2>
        {action === 'demote' && (
          <p className="text-sm text-zinc-500 mb-1">
            {oldRole} &rarr; {newRole} {t('cascade.on', 'on')} {noteTitle}
          </p>
        )}
        {action === 'revoke' && (
          <p className="text-sm text-zinc-500 mb-1">
            {t('cascade.revokeFrom', 'Revoking from')} {noteTitle}
          </p>
        )}

        {loading ? (
          <p className="text-sm text-zinc-400 py-4">{t('common.loading', 'Loading…')}</p>
        ) : impacts.length === 0 ? (
          <p className="text-sm text-zinc-500 py-4 mb-4">
            {t('cascade.noImpact', 'No downstream grants will be affected.')}
          </p>
        ) : (
          <>
            <p className="text-sm text-zinc-500 mb-3">
              {t('cascade.explanation', 'This user previously granted access to others. These grants would no longer be valid:')}
            </p>
            <div className="max-h-48 overflow-y-auto border rounded dark:border-zinc-700 mb-4">
              {impacts.map(impact => (
                <label
                  key={grantKey(impact)}
                  className="flex items-center gap-2 px-3 py-2 text-sm hover:bg-zinc-50 dark:hover:bg-zinc-800 cursor-pointer"
                >
                  <input
                    type="checkbox"
                    checked={checked.has(grantKey(impact))}
                    onChange={() => toggleCheck(grantKey(impact))}
                    className="rounded"
                  />
                  <span className={
                    impact.grant.role === 'owner' ? 'text-green-500' :
                    impact.grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
                  }>&#x25CF;</span>
                  <span className="flex-1">
                    {nameMap[impact.grant.userId] ?? impact.grant.userId.slice(0, 8)}
                    <span className="text-zinc-400 ml-1">— {impact.grant.role}</span>
                  </span>
                  <span className="text-xs text-zinc-400">{impact.reason}</span>
                </label>
              ))}
            </div>
          </>
        )}

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded border dark:border-zinc-700"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={() => onConfirm([])}
            className="px-4 py-2 text-sm rounded border dark:border-zinc-700"
          >
            {action === 'demote'
              ? t('cascade.demoteOnly', 'Demote only')
              : t('cascade.revokeOnly', 'Revoke only')}
          </button>
          {impacts.length > 0 && (
            <button
              onClick={() => onConfirm(
                impacts
                  .filter(i => checked.has(grantKey(i)))
                  .map(i => ({ noteId: i.grant.noteId, userId: i.grant.userId }))
              )}
              className="px-4 py-2 text-sm rounded bg-red-600 text-white"
            >
              {action === 'demote'
                ? t('cascade.demoteAndRevoke', 'Demote & revoke selected')
                : t('cascade.revokeAndRevoke', 'Revoke & revoke selected')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
