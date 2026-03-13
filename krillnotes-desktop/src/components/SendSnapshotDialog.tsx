// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import { useState, useEffect } from 'react';
import type { PeerInfo, SnapshotCreatedResult } from '../types';

interface SendSnapshotDialogProps {
  open: boolean;
  identityUuid: string;
  preSelectedPublicKeys: string[];
  onClose: () => void;
  onSuccess: (result: SnapshotCreatedResult) => void;
}

export function SendSnapshotDialog({
  open, identityUuid, preSelectedPublicKeys, onClose, onSuccess,
}: SendSnapshotDialogProps) {
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [savePath, setSavePath] = useState<string>('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setSelected(new Set(preSelectedPublicKeys));
    setSavePath('');
    setError(null);
    invoke<PeerInfo[]>('list_workspace_peers')
      .then(setPeers)
      .catch(e => setError(String(e)));
  }, [open]);  // intentionally omit preSelectedPublicKeys to avoid loop

  const chooseSavePath = async () => {
    const path = await save({
      filters: [{ name: 'Swarm Bundle', extensions: ['swarm'] }],
      defaultPath: 'snapshot.swarm',
    });
    if (path) setSavePath(path);
  };

  const toggle = (pk: string) => {
    setSelected(prev => {
      const next = new Set(prev);
      next.has(pk) ? next.delete(pk) : next.add(pk);
      return next;
    });
  };

  const handleCreate = async () => {
    if (selected.size === 0) { setError('Select at least one peer.'); return; }
    if (!savePath) { setError('Choose a save location first.'); return; }
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<SnapshotCreatedResult>('create_snapshot_for_peers', {
        identityUuid,
        peerPublicKeys: Array.from(selected),
        savePath,
      });
      onSuccess(result);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  if (!open) return null;
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg p-6 max-w-md w-full shadow-xl">
        <h2 className="text-lg font-semibold mb-4">Create Workspace Snapshot</h2>
        <p className="text-sm text-muted-foreground mb-3">
          Select peers to include. Each peer receives the same encrypted snapshot.
        </p>

        <div className="space-y-1 mb-4 max-h-48 overflow-y-auto">
          {peers.map(p => (
            <label key={p.peerIdentityId} className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                checked={selected.has(p.peerIdentityId)}
                onChange={() => toggle(p.peerIdentityId)}
              />
              <span>{p.displayName}</span>
              <span className="text-muted-foreground text-xs truncate">{p.fingerprint}</span>
            </label>
          ))}
          {peers.length === 0 && <p className="text-sm text-muted-foreground">No peers registered.</p>}
        </div>

        <div className="flex items-center gap-2 mb-4">
          <button
            onClick={chooseSavePath}
            className="px-3 py-1.5 text-sm rounded border border-secondary hover:bg-secondary"
          >
            Choose location…
          </button>
          {savePath && <span className="text-xs text-muted-foreground truncate">{savePath}</span>}
        </div>

        {error && <p className="text-sm text-red-500 mb-3">{error}</p>}

        <div className="flex justify-end gap-3">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
          >
            Cancel
          </button>
          <button
            onClick={handleCreate}
            disabled={loading}
            className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:opacity-90 disabled:opacity-50"
          >
            {loading ? 'Creating…' : 'Create Snapshot'}
          </button>
        </div>
      </div>
    </div>
  );
}
