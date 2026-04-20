import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { AppSettings } from '../types';

type SyncOnCloseState =
  | { phase: 'idle' }
  | { phase: 'asking' }
  | { phase: 'syncing'; error: string | null };

export function useSyncOnClose() {
  const [state, setState] = useState<SyncOnCloseState>({ phase: 'idle' });
  const stateRef = useRef(state);
  stateRef.current = state;
  const cancelledRef = useRef(false);

  useEffect(() => {
    const unlisten = listen('krillnotes://close-requested', async () => {
      if (stateRef.current.phase !== 'idle') return;

      try {
        const settings = await invoke<AppSettings>('get_settings');
        const mode = settings.syncOnClose ?? 'ask';

        if (mode === 'never') {
          await invoke('close_window');
          return;
        }

        const hasPending = await invoke<boolean>('has_pending_sync_ops');
        if (!hasPending) {
          await invoke('close_window');
          return;
        }

        if (mode === 'always') {
          cancelledRef.current = false;
          setState({ phase: 'syncing', error: null });
          try {
            await invoke('poll_sync');
            if (!cancelledRef.current) await invoke('close_window');
          } catch (err) {
            if (!cancelledRef.current) setState({ phase: 'syncing', error: String(err) });
          }
          return;
        }

        // mode === 'ask'
        setState({ phase: 'asking' });
      } catch {
        await invoke('close_window');
      }
    });

    return () => { unlisten.then(fn => fn()); };
  }, []);

  const handleSyncAndClose = useCallback(async () => {
    cancelledRef.current = false;
    setState({ phase: 'syncing', error: null });
    try {
      await invoke('poll_sync');
      if (!cancelledRef.current) await invoke('close_window');
    } catch (err) {
      if (!cancelledRef.current) setState({ phase: 'syncing', error: String(err) });
    }
  }, []);

  const handleCloseWithoutSync = useCallback(async () => {
    setState({ phase: 'idle' });
    await invoke('close_window');
  }, []);

  const handleCancel = useCallback(() => {
    cancelledRef.current = true;
    setState({ phase: 'idle' });
  }, []);

  return {
    syncOnCloseState: state,
    handleSyncAndClose,
    handleCloseWithoutSync,
    handleCancel,
  };
}
