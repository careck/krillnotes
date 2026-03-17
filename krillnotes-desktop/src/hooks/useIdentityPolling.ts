import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const POLL_INTERVAL_MS = 60_000;

export function useIdentityPolling(
  identityUuid: string | null,
  hasRelayAccount: boolean,
  hasWaitingInvites: boolean,
) {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!identityUuid || !hasRelayAccount || !hasWaitingInvites) return;

    const poll = async () => {
      try {
        await invoke("poll_receive_identity", { identityUuid });
      } catch (e) {
        console.warn("poll_receive_identity failed:", e);
      }
    };

    poll();
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [identityUuid, hasRelayAccount, hasWaitingInvites]);
}
