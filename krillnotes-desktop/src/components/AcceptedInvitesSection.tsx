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

  if (loading || invites.length === 0) return null;

  return (
    <div className="mt-4">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-purple-400 mb-2">
        {t("polling.acceptedInvites")}
      </h4>
      <div className="flex flex-col gap-2">
        {invites.map((invite) => (
          <div key={invite.inviteId}
            className="bg-white/5 rounded-lg px-4 py-3 flex items-center justify-between"
          >
            <div>
              <div className="font-semibold text-sm">{invite.workspaceName}</div>
              <div className="text-xs text-gray-400 mt-0.5">
                {t("common.from", "From")}: {invite.inviterDeclaredName} · {new Date(invite.acceptedAt).toLocaleDateString()}
              </div>
            </div>
            <div className="flex items-center gap-2">
              {invite.status === "waitingSnapshot" ? (
                <span className="bg-amber-500/20 text-amber-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                  {t("polling.waitingForSnapshot")}
                </span>
              ) : (
                <>
                  <span className="bg-green-500/20 text-green-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    ✓ {t("polling.workspaceCreated")}
                  </span>
                  {invite.workspacePath && (
                    <button className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1 rounded-md">
                      {t("common.open")}
                    </button>
                  )}
                </>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
