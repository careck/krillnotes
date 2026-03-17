// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useTranslation } from "react-i18next";
import type { ReceivedResponseInfo } from "../types";

interface Props {
  identityUuid: string;
  workspaceId?: string;
  onAcceptResponse: (response: ReceivedResponseInfo) => void;
  onSendSnapshot: (response: ReceivedResponseInfo) => void;
}

export default function PendingResponsesSection({
  identityUuid, workspaceId, onAcceptResponse, onSendSnapshot,
}: Props) {
  const { t } = useTranslation();
  const [responses, setResponses] = useState<ReceivedResponseInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const loadResponses = useCallback(async () => {
    try {
      const result = await invoke<ReceivedResponseInfo[]>("list_received_responses", {
        identityUuid, workspaceId,
      });
      setResponses(result);
    } catch (e) {
      console.error("Failed to load received responses:", e);
    } finally {
      setLoading(false);
    }
  }, [identityUuid, workspaceId]);

  useEffect(() => { loadResponses(); }, [loadResponses]);

  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().listen<ReceivedResponseInfo>(
      "invite-response-received", () => { loadResponses(); }
    );
    return () => { unlisten.then(f => f()); };
  }, [loadResponses]);

  if (loading || responses.length === 0) return null;

  const pendingCount = responses.filter(r => r.status === "pending").length;

  return (
    <div className="mb-4">
      <h4 className="text-xs font-semibold uppercase tracking-wide text-amber-400 mb-2 flex items-center gap-2">
        {t("polling.pendingInviteResponses")}
        {pendingCount > 0 && (
          <span className="bg-amber-400 text-gray-900 px-1.5 py-0.5 rounded-full text-[10px] font-bold">
            {pendingCount}
          </span>
        )}
      </h4>
      <div className="flex flex-col gap-2">
        {responses.map((resp) => (
          <div key={resp.responseId}
            className={`bg-white/5 rounded-lg px-4 py-3 flex items-center justify-between ${
              resp.status === "pending" ? "border-l-3 border-amber-400" : ""
            } ${resp.status === "snapshotSent" ? "opacity-60" : ""}`}
          >
            <div>
              <div className="font-semibold text-sm">{resp.inviteeDeclaredName}</div>
              <div className="text-xs text-gray-400 mt-0.5">
                {t("polling.responded", "Responded")} {new Date(resp.receivedAt).toLocaleDateString()}
              </div>
            </div>
            <div className="flex items-center gap-2">
              {resp.status === "pending" && (
                <>
                  <span className="bg-amber-500/20 text-amber-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    {t("polling.actionNeeded")}
                  </span>
                  <button
                    className="bg-purple-600 hover:bg-purple-500 text-white text-xs px-3 py-1.5 rounded-md"
                    onClick={() => onAcceptResponse(resp)}
                  >
                    {t("polling.acceptAndSendSnapshot")}
                  </button>
                </>
              )}
              {resp.status === "peerAdded" && (
                <>
                  <span className="bg-blue-500/20 text-blue-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                    {t("polling.peerAdded")}
                  </span>
                  <button
                    className="bg-gray-600 hover:bg-gray-500 text-white text-xs px-3 py-1 rounded-md"
                    onClick={() => onSendSnapshot(resp)}
                  >
                    {t("polling.sendSnapshot")}
                  </button>
                </>
              )}
              {resp.status === "snapshotSent" && (
                <span className="bg-green-500/20 text-green-400 px-2.5 py-0.5 rounded-full text-xs font-semibold">
                  {t("polling.snapshotSent")}
                </span>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
