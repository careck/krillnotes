// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ListFilter, Trash2, X } from 'lucide-react';
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

function formatTimestamp(wallMs: number): string {
  return new Date(wallMs).toLocaleString();
}

function formatKey(key: string): string {
  return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
}

// Fields shown in the "metadata" section at the top of the detail panel.
const METADATA_KEYS = new Set(['type', 'operation_id', 'device_id', 'timestamp']);

// Operation fields that hold a public key identifying the author.
const AUTHOR_KEY_FIELDS = new Set(['created_by', 'modified_by', 'deleted_by', 'moved_by', 'updated_by']);

function DetailValue({ fieldKey, value }: { fieldKey: string; value: unknown }) {
  if (value === null || value === undefined) {
    return <span className="text-muted-foreground italic">—</span>;
  }
  if (typeof value === 'boolean') {
    return <span className="font-mono">{value ? 'true' : 'false'}</span>;
  }
  if (typeof value === 'number') {
    return <span className="font-mono">{value}</span>;
  }
  if (typeof value === 'string') {
    if (value.length > 100) {
      return (
        <pre className="text-xs font-mono bg-muted/50 rounded p-2 max-h-48 overflow-auto whitespace-pre-wrap break-all">
          {value}
        </pre>
      );
    }
    return <span className="font-mono text-xs break-all">{value}</span>;
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <span className="text-muted-foreground italic">[]</span>;
    return (
      <ul className="text-xs font-mono list-disc list-inside space-y-0.5">
        {(value as unknown[]).map((item, i) => (
          <li key={i}>{typeof item === 'object' ? JSON.stringify(item) : String(item)}</li>
        ))}
      </ul>
    );
  }
  if (typeof value === 'object') {
    // Render HLC timestamp inline.
    if (fieldKey === 'timestamp') {
      const ts = value as { wall_ms: number; counter: number; node_id: number };
      return (
        <span className="font-mono text-xs">
          {new Date(ts.wall_ms).toISOString()} (counter={ts.counter})
        </span>
      );
    }
    return (
      <pre className="text-xs font-mono bg-muted/50 rounded p-2 max-h-48 overflow-auto">
        {JSON.stringify(value, null, 2)}
      </pre>
    );
  }
  return <span className="text-xs">{String(value)}</span>;
}

function OperationDetailPanel({
  detail,
  resolvedAuthor,
  onClose,
}: {
  detail: Record<string, unknown>;
  resolvedAuthor: string;
  onClose: () => void;
}) {
  const opType = detail['type'] as string | undefined;

  const metaEntries = Object.entries(detail).filter(([k]) => METADATA_KEYS.has(k));
  const dataEntries = Object.entries(detail).filter(([k]) => !METADATA_KEYS.has(k));

  return (
    <div className="w-[380px] border-l border-border flex flex-col overflow-hidden shrink-0">
      {/* Panel header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border bg-muted/20 shrink-0">
        <span className="text-sm font-semibold font-mono">{opType ?? 'Operation'}</span>
        <button
          onClick={onClose}
          className="text-muted-foreground hover:text-foreground rounded p-0.5"
          aria-label="Close detail"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-4 text-sm">
        {/* Metadata section */}
        <section>
          <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-1">Metadata</p>
          <dl className="space-y-1.5">
            {metaEntries.map(([k, v]) => (
              k === 'type' ? null : (
                <div key={k}>
                  <dt className="text-xs text-muted-foreground">{formatKey(k)}</dt>
                  <dd className="mt-0.5"><DetailValue fieldKey={k} value={v} /></dd>
                </div>
              )
            ))}
          </dl>
        </section>

        {/* Operation-specific data */}
        {dataEntries.length > 0 && (
          <section>
            <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-1">Data</p>
            <dl className="space-y-2">
              {dataEntries.map(([k, v]) => (
                <div key={k}>
                  <dt className="text-xs text-muted-foreground">{formatKey(k)}</dt>
                  <dd className="mt-0.5">
                    <DetailValue fieldKey={k} value={v} />
                    {AUTHOR_KEY_FIELDS.has(k) && resolvedAuthor && (
                      <span className="block text-xs text-muted-foreground mt-0.5">
                        {resolvedAuthor}
                      </span>
                    )}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        )}
      </div>
    </div>
  );
}

function OperationsLogDialog({ isOpen, onClose }: OperationsLogDialogProps) {
  const { t } = useTranslation();
  const [operations, setOperations] = useState<OperationSummary[]>([]);
  const [typeFilter, setTypeFilter] = useState<string>('');
  const [sinceDate, setSinceDate] = useState('');
  const [untilDate, setUntilDate] = useState('');
  const [error, setError] = useState('');
  const [confirmPurge, setConfirmPurge] = useState(false);
  const [selectedOpId, setSelectedOpId] = useState<string | null>(null);
  const [opDetail, setOpDetail] = useState<Record<string, unknown> | null>(null);

  const loadOperations = useCallback(async () => {
    try {
      const since = sinceDate
        ? new Date(sinceDate + 'T00:00:00').getTime()
        : undefined;
      const until = untilDate
        ? new Date(untilDate + 'T23:59:59').getTime()
        : undefined;

      const result = await invoke<OperationSummary[]>('list_operations', {
        typeFilter: typeFilter || null,
        since: since ?? null,
        until: until ?? null,
      });
      setOperations(result);
      setError('');
    } catch (err) {
      setError(t('log.failedLoad', { error: String(err) }));
    }
  }, [typeFilter, sinceDate, untilDate]);

  useEffect(() => {
    if (isOpen) {
      setTypeFilter('');
      setSinceDate('');
      setUntilDate('');
      setConfirmPurge(false);
      setError('');
      setSelectedOpId(null);
      setOpDetail(null);
    }
  }, [isOpen]);

  useEffect(() => {
    if (isOpen) {
      loadOperations();
    }
  }, [isOpen, loadOperations]);

  const handleSelectOp = async (opId: string) => {
    if (selectedOpId === opId) {
      setSelectedOpId(null);
      setOpDetail(null);
      return;
    }
    setSelectedOpId(opId);
    setOpDetail(null);
    try {
      const detail = await invoke<Record<string, unknown>>('get_operation_detail', {
        operationId: opId,
      });
      setOpDetail(detail);
    } catch (err) {
      setError(String(err));
    }
  };

  const handlePurge = async () => {
    try {
      await invoke('purge_operations');
      setConfirmPurge(false);
      setSelectedOpId(null);
      setOpDetail(null);
      loadOperations();
    } catch (err) {
      setError(t('log.failedPurge', { error: String(err) }));
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div
        className="bg-background border border-border rounded-lg shadow-lg max-h-[80vh] flex flex-col"
        style={{ width: opDetail ? '1080px' : '700px', transition: 'width 0.15s ease' }}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border shrink-0">
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
        <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-muted/30 shrink-0">
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
          <div className="px-4 py-2 text-sm text-red-600 bg-red-50 border-b border-border shrink-0">
            {error}
          </div>
        )}

        {/* Content area: list + optional detail panel side by side */}
        <div className="flex flex-1 overflow-hidden">
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
                    <th className="text-left px-4 py-2 font-medium text-muted-foreground">{t('log.author')}</th>
                    <th className="text-right px-4 py-2 font-medium text-muted-foreground">{t('log.type')}</th>
                  </tr>
                </thead>
                <tbody>
                  {operations.map((op) => (
                    <tr
                      key={op.operationId}
                      onClick={() => handleSelectOp(op.operationId)}
                      className={`border-b border-border/50 cursor-pointer ${
                        selectedOpId === op.operationId
                          ? 'bg-primary/10 hover:bg-primary/15'
                          : 'hover:bg-muted/20'
                      }`}
                    >
                      <td className="px-4 py-2 text-muted-foreground whitespace-nowrap">
                        {formatTimestamp(op.timestampWallMs)}
                      </td>
                      <td className="px-4 py-2 truncate max-w-[200px]" title={op.targetName}>
                        {op.targetName || <span className="text-muted-foreground italic">&mdash;</span>}
                      </td>
                      <td className="px-4 py-2 whitespace-nowrap">
                        <span className="text-xs font-mono text-muted-foreground">
                          {op.authorKey || '—'}
                        </span>
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

          {/* Detail panel */}
          {opDetail && (
            <OperationDetailPanel
              detail={opDetail}
              resolvedAuthor={operations.find((op) => op.operationId === selectedOpId)?.authorKey ?? ''}
              onClose={() => { setSelectedOpId(null); setOpDetail(null); }}
            />
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-border shrink-0">
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
