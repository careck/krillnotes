import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

const POLL_INTERVAL_MS = 60_000;

export function useRelayPolling(hasRelayPeers: boolean) {
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (!hasRelayPeers) return;

    const poll = async () => {
      try {
        await invoke("poll_receive_workspace");
      } catch (e) {
        console.warn("poll_receive_workspace failed:", e);
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
  }, [hasRelayPeers]);
}
