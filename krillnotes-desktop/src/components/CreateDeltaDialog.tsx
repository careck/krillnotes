// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { PeerInfo, GenerateDeltasResult } from "../types";

interface Props {
  onClose: () => void;
}

export function CreateDeltaDialog({ onClose }: Props) {
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [dirPath, setDirPath] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<GenerateDeltasResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<PeerInfo[]>("get_workspace_peers")
      .then(setPeers)
      .catch((e) => setError(String(e)));
  }, []);

  const handleBrowse = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Choose a directory",
      });
      if (selected && !Array.isArray(selected)) {
        setDirPath(selected);
      }
    } catch {
      // user cancelled or API unavailable — leave dirPath unchanged
    }
  };

  const togglePeer = (id: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const handleGenerate = async () => {
    if (!dirPath || checked.size === 0) return;
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<GenerateDeltasResult>("generate_deltas_for_peers", {
        dirPath,
        peerDeviceIds: Array.from(checked),
      });
      setResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const canGenerate = dirPath.length > 0 && checked.size > 0 && !loading;
  const allDone = result !== null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-zinc-900 rounded-lg shadow-xl w-[480px] max-h-[80vh] flex flex-col p-6 gap-4">
        <h2 className="text-lg font-semibold">Create delta Swarm</h2>

        {/* Directory picker */}
        <div className="flex flex-col gap-1">
          <label className="text-sm text-zinc-500">Save to directory</label>
          <div className="flex gap-2">
            <input
              type="text"
              readOnly
              value={dirPath}
              placeholder="Choose a directory…"
              className="flex-1 border rounded px-2 py-1 text-sm bg-zinc-50 dark:bg-zinc-800"
            />
            <button
              onClick={handleBrowse}
              className="px-3 py-1 text-sm border rounded hover:bg-zinc-100 dark:hover:bg-zinc-700"
            >
              Browse…
            </button>
          </div>
        </div>

        {/* Peer list */}
        <div className="flex flex-col gap-1 overflow-y-auto max-h-48">
          <label className="text-sm text-zinc-500">Generate delta for</label>
          {peers.length === 0 && !error && (
            <p className="text-sm text-zinc-400 italic">Loading peers…</p>
          )}
          {peers.map((p) => {
            const syncable = p.lastSync !== undefined;
            return (
              <label
                key={p.peerDeviceId}
                className={`flex items-center gap-2 px-2 py-1 rounded cursor-pointer
                  ${syncable ? "hover:bg-zinc-50 dark:hover:bg-zinc-800" : "opacity-40 cursor-not-allowed"}`}
              >
                <input
                  type="checkbox"
                  disabled={!syncable || allDone}
                  checked={checked.has(p.peerDeviceId)}
                  onChange={() => togglePeer(p.peerDeviceId)}
                />
                <span className="flex-1 text-sm font-medium">{p.displayName}</span>
                <span className="text-xs text-zinc-400">{p.fingerprint}</span>
                {!syncable && (
                  <span className="text-xs text-orange-400 ml-1">— never synced</span>
                )}
                {result?.succeeded.includes(p.peerDeviceId) && (
                  <span className="text-xs text-green-500">✓</span>
                )}
                {result?.failed.find(([id]) => id === p.peerDeviceId) && (
                  <span className="text-xs text-red-500">
                    ✗ {result.failed.find(([id]) => id === p.peerDeviceId)![1]}
                  </span>
                )}
              </label>
            );
          })}
        </div>

        {error && <p className="text-xs text-red-500">{error}</p>}

        {allDone && result.failed.length === 0 && (
          <p className="text-sm text-green-600">
            ✓ {result.filesWritten.length} file(s) written to {dirPath}
          </p>
        )}

        {/* Buttons */}
        <div className="flex justify-end gap-2 pt-2">
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-sm border rounded hover:bg-zinc-100 dark:hover:bg-zinc-700"
          >
            {allDone ? "Close" : "Cancel"}
          </button>
          {!allDone && (
            <button
              onClick={handleGenerate}
              disabled={!canGenerate}
              className="px-4 py-1.5 text-sm bg-blue-600 text-white rounded
                         hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? "Generating…" : "Generate"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
