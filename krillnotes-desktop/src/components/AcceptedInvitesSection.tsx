// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";
import type { AcceptedInviteInfo, SnapshotReceivedEvent } from "../types";

interface Props {
  identityUuid: string;
}

export default function AcceptedInvitesSection({ identityUuid }: Props) {
  const { t } = useTranslation();
  const [invites, setInvites] = useState<AcceptedInviteInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [creatingId, setCreatingId] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [createError, setCreateError] = useState<string | null>(null);

  const loadInvites = useCallback(async () => {
    try {
      const result = await invoke<AcceptedInviteInfo[]>("list_accepted_invites", { identityUuid });
      setInvites(result);
    } catch (e) {
      console.error("Failed to load accepted invites:", e);
    } finally {
      setLoading(false);
    }
  }, [identityUuid]);

  useEffect(() => { loadInvites(); }, [loadInvites]);

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<SnapshotReceivedEvent>("snapshot-received", () => {
      loadInvites();
    });
    return () => { unlisten.then(f => f()); };
  }, [loadInvites]);

  const handleCreateWorkspace = async (invite: AcceptedInviteInfo) => {
    if (!invite.snapshotPath) return;
    setCreatingId(invite.inviteId);
    setCreateError(null);
    try {
      const nameOverride = renamingId === invite.inviteId && renameValue.trim()
        ? renameValue.trim()
        : undefined;
      await invoke("apply_swarm_snapshot", {
        path: invite.snapshotPath,
        identityUuid,
        workspaceNameOverride: nameOverride || null,
      });
      await invoke("update_accepted_invite_status", {
        identityUuid,
        inviteId: invite.inviteId,
        status: "workspaceCreated",
        workspacePath: null,
      });
      setRenamingId(null);
      loadInvites();
    } catch (e) {
      console.error("Failed to create workspace from snapshot:", e);
      setCreateError(String(e));
    } finally {
      setCreatingId(null);
    }
  };

  if (loading || invites.length === 0) return null;

  return (
    <div className="mt-4">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-purple-400 mb-2">
        {t("polling.acceptedInvites")}
      </h4>
      <div className="flex flex-col gap-2">
        {invites.map((invite) => (
          <div key={invite.inviteId}
            className="bg-white/5 rounded-lg px-4 py-3"
          >
            <div className="flex items-center justify-between">
              <div>
                <div className="flex items-center">
                  <span className="font-semibold text-sm">{invite.workspaceName}</span>
                  {invite.offeredRole && (
                    <span className={`ml-2 px-2 py-0.5 rounded text-xs font-medium ${
                      invite.offeredRole === 'owner' ? 'bg-purple-500/20 text-purple-300' :
                      invite.offeredRole === 'writer' ? 'bg-green-500/20 text-green-300' :
                      'bg-blue-500/20 text-blue-300'
                    }`}>
                      {t(`roles.${invite.offeredRole}`)}
                    </span>
                  )}
                </div>
                <div className="text-xs text-gray-400 mt-0.5">
                  {t("common.from", "From")}: {invite.inviterDeclaredName} · {new Date(invite.acceptedAt).toLocaleDateString()}
                </div>
              </div>
              <div className="flex items-center gap-2">
                {invite.status === "waitingSnapshot" && !invite.snapshotPath && (
                  <span className="bg-amber-500/20 text-amber-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    {t("polling.waitingForSnapshot")}
                  </span>
                )}
                {invite.status === "waitingSnapshot" && invite.snapshotPath && (
                  <>
                    <span className="bg-blue-500/20 text-blue-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                      {t("polling.snapshotReceived", "Snapshot received")}
                    </span>
                    <button
                      className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1.5 rounded-md disabled:opacity-50"
                      disabled={creatingId === invite.inviteId}
                      onClick={() => {
                        if (renamingId === invite.inviteId) {
                          handleCreateWorkspace(invite);
                        } else {
                          setRenamingId(invite.inviteId);
                          setRenameValue(invite.workspaceName);
                        }
                      }}
                    >
                      {creatingId === invite.inviteId
                        ? t("common.loading", "Creating...")
                        : t("polling.createWorkspace", "Create Workspace")}
                    </button>
                  </>
                )}
                {invite.status === "workspaceCreated" && (
                  <span className="bg-green-500/20 text-green-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    ✓ {t("polling.workspaceCreated")}
                  </span>
                )}
              </div>
            </div>
            {/* Inline rename row */}
            {renamingId === invite.inviteId && (
              <div className="mt-2 flex items-center gap-2">
                <input
                  type="text"
                  className="flex-1 bg-white/10 border border-white/20 rounded px-2 py-1 text-sm"
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  placeholder={invite.workspaceName}
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreateWorkspace(invite);
                    if (e.key === "Escape") setRenamingId(null);
                  }}
                />
                <button
                  className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1.5 rounded-md disabled:opacity-50"
                  disabled={creatingId === invite.inviteId}
                  onClick={() => handleCreateWorkspace(invite)}
                >
                  {creatingId === invite.inviteId ? "..." : t("common.confirm", "Confirm")}
                </button>
                <button
                  className="text-gray-400 text-xs px-2 py-1"
                  onClick={() => setRenamingId(null)}
                >
                  {t("common.cancel", "Cancel")}
                </button>
              </div>
            )}
            {createError && creatingId === null && renamingId === invite.inviteId && (
              <div className="mt-1 text-xs text-red-400">{createError}</div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
