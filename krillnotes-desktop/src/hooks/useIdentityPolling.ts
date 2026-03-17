import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const POLL_INTERVAL_MS = 60_000;

/**
 * Global snapshot polling for ALL unlocked identities.
 * The Rust command iterates over every unlocked identity that has
 * relay accounts + accepted invites in WaitingSnapshot status.
 * No conditions needed on the frontend — just call on a timer.
 */
export function useGlobalSnapshotPolling() {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const poll = async () => {
      try {
        await invoke("poll_all_identity_snapshots");
      } catch (e) {
        console.warn("poll_all_identity_snapshots failed:", e);
      }
    };

    poll(); // immediate first poll
    intervalRef.current = setInterval(poll, POLL_INTERVAL_MS);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, []);
}

/** @deprecated Use useGlobalSnapshotPolling instead */
export function useIdentityPolling(
  _identityUuid: string | null,
  _hasRelayAccount: boolean,
  _hasWaitingInvites: boolean,
) {
  useGlobalSnapshotPolling();
}
