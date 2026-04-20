import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';

interface SyncOnCloseDialogProps {
  mode: 'ask' | 'syncing';
  syncError: string | null;
  onSyncAndClose: () => void;
  onCloseWithoutSync: () => void;
  onCancel: () => void;
}

export default function SyncOnCloseDialog({
  mode,
  syncError,
  onSyncAndClose,
  onCloseWithoutSync,
  onCancel,
}: SyncOnCloseDialogProps) {
  const { t } = useTranslation();

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onCancel();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onCancel]);

  const isSyncing = mode === 'syncing' && !syncError;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border p-6 rounded-lg w-[420px]">
        {syncError ? (
          <>
            <h3 className="text-lg font-semibold mb-3">{t('syncOnClose.errorTitle')}</h3>
            <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 text-red-500 rounded text-sm">
              {syncError}
            </div>
            <div className="flex justify-end gap-2">
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCancel}
              >
                {t('syncOnClose.cancel')}
              </button>
              <button
                className="px-4 py-2 bg-orange-500 text-white rounded hover:bg-orange-600"
                onClick={onCloseWithoutSync}
              >
                {t('syncOnClose.closeAnyway')}
              </button>
            </div>
          </>
        ) : isSyncing ? (
          <div className="flex flex-col items-center py-4 gap-3">
            <div className="w-6 h-6 border-2 border-primary border-t-transparent rounded-full animate-spin" />
            <span className="text-sm text-muted-foreground">{t('syncOnClose.syncing')}</span>
          </div>
        ) : (
          <>
            <h3 className="text-lg font-semibold mb-3">{t('settings.syncOnClose')}</h3>
            <p className="text-sm text-muted-foreground mb-5">{t('syncOnClose.message')}</p>
            <div className="flex justify-end gap-2">
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCancel}
              >
                {t('syncOnClose.cancel')}
              </button>
              <button
                className="px-4 py-2 border border-secondary rounded hover:bg-secondary"
                onClick={onCloseWithoutSync}
              >
                {t('syncOnClose.closeWithoutSync')}
              </button>
              <button
                className="px-4 py-2 bg-primary text-primary-foreground rounded hover:bg-primary/90"
                onClick={onSyncAndClose}
              >
                {t('syncOnClose.syncAndClose')}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
