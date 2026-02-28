import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ListFilter, Trash2 } from 'lucide-react';
import type { OperationSummary } from '../types';
import { useTranslation } from 'react-i18next';

interface OperationsLogDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

const OPERATION_TYPES = [
  'CreateNote',
  'UpdateField',
  'DeleteNote',
  'MoveNote',
  'CreateUserScript',
  'UpdateUserScript',
  'DeleteUserScript',
] as const;

function formatTimestamp(unix: number): string {
  const date = new Date(unix * 1000);
  return date.toLocaleString();
}

function OperationsLogDialog({ isOpen, onClose }: OperationsLogDialogProps) {
  const { t } = useTranslation();
  const [operations, setOperations] = useState<OperationSummary[]>([]);
  const [typeFilter, setTypeFilter] = useState<string>('');
  const [sinceDate, setSinceDate] = useState('');
  const [untilDate, setUntilDate] = useState('');
  const [error, setError] = useState('');
  const [confirmPurge, setConfirmPurge] = useState(false);

  const loadOperations = useCallback(async () => {
    try {
      const since = sinceDate
        ? Math.floor(new Date(sinceDate + 'T00:00:00').getTime() / 1000)
        : undefined;
      const until = untilDate
        ? Math.floor(new Date(untilDate + 'T23:59:59').getTime() / 1000)
        : undefined;

      const result = await invoke<OperationSummary[]>('list_operations', {
        typeFilter: typeFilter || null,
        since: since ?? null,
        until: until ?? null,
      });
      setOperations(result);
      setError('');
    } catch (err) {
      setError(`Failed to load operations: ${err}`);
    }
  }, [typeFilter, sinceDate, untilDate]);

  useEffect(() => {
    if (isOpen) {
      setTypeFilter('');
      setSinceDate('');
      setUntilDate('');
      setConfirmPurge(false);
      setError('');
    }
  }, [isOpen]);

  useEffect(() => {
    if (isOpen) {
      loadOperations();
    }
  }, [isOpen, loadOperations]);

  const handlePurge = async () => {
    try {
      await invoke('purge_operations');
      setConfirmPurge(false);
      loadOperations();
    } catch (err) {
      setError(`Failed to purge operations: ${err}`);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg shadow-lg w-[700px] max-h-[80vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <ListFilter className="w-5 h-5" />
            {t('log.title')}
          </h2>
          <button
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground text-xl leading-none px-1"
          >
            &times;
          </button>
        </div>

        {/* Filters */}
        <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-muted/30">
          <select
            value={typeFilter}
            onChange={(e) => setTypeFilter(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          >
            <option value="">{t('log.allTypes')}</option>
            {OPERATION_TYPES.map((opType) => (
              <option key={opType} value={opType}>{opType}</option>
            ))}
          </select>

          <label className="text-sm text-muted-foreground">{t('log.from')}</label>
          <input
            type="date"
            value={sinceDate}
            onChange={(e) => setSinceDate(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          />

          <label className="text-sm text-muted-foreground">{t('log.to')}</label>
          <input
            type="date"
            value={untilDate}
            onChange={(e) => setUntilDate(e.target.value)}
            className="bg-background border border-input rounded px-2 py-1 text-sm"
          />
        </div>

        {/* Error */}
        {error && (
          <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border">
            {error}
          </div>
        )}

        {/* Operations list */}
        <div className="flex-1 overflow-y-auto">
          {operations.length === 0 ? (
            <div className="px-4 py-8 text-center text-muted-foreground text-sm">
              {t('log.noOperations')}
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead className="bg-muted/30 sticky top-0">
                <tr>
                  <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.dateTime')}</th>
                  <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.target')}</th>
                  <th className="text-right px-4 py-2 font-medium text-muted-foreground">{t('log.type')}</th>
                </tr>
              </thead>
              <tbody>
                {operations.map((op) => (
                  <tr key={op.operationId} className="border-b border-border/50 hover:bg-muted/20">
                    <td className="px-4 py-2 text-muted-foreground whitespace-nowrap">
                      {formatTimestamp(op.timestamp)}
                    </td>
                    <td className="px-4 py-2 truncate max-w-[250px]" title={op.targetName}>
                      {op.targetName || <span className="text-muted-foreground italic">&mdash;</span>}
                    </td>
                    <td className="px-4 py-2 text-right">
                      <span className="inline-block bg-muted text-muted-foreground rounded px-2 py-0.5 text-xs font-mono">
                        {op.operationType}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-border">
          <span className="text-sm text-muted-foreground">
            {t('log.count', { count: operations.length })}
          </span>
          <div className="flex items-center gap-2">
            {confirmPurge ? (
              <>
                <span className="text-sm text-red-600">{t('log.deleteAll')}</span>
                <button
                  onClick={handlePurge}
                  className="bg-red-600 text-white px-3 py-1 rounded text-sm hover:bg-red-700"
                >
                  {t('common.confirm')}
                </button>
                <button
                  onClick={() => setConfirmPurge(false)}
                  className="bg-muted text-foreground px-3 py-1 rounded text-sm hover:bg-muted/80"
                >
                  {t('common.cancel')}
                </button>
              </>
            ) : (
              <button
                onClick={() => setConfirmPurge(true)}
                className="flex items-center gap-1 text-sm text-muted-foreground hover:text-red-600 px-3 py-1 rounded border border-border hover:border-red-300"
              >
                <Trash2 className="w-3.5 h-3.5" />
                {t('log.purgeAll')}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default OperationsLogDialog;
