import { useTranslation } from 'react-i18next';

function EmptyState() {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="text-center">
        <img
          src="/KrillNotesLogo.png"
          alt="KrillNotes"
          className="w-64 h-64 mx-auto mb-6 object-contain"
        />
        <p className="text-muted-foreground">
          {t('empty.getStarted')}
        </p>
      </div>
    </div>
  );
}

export default EmptyState;
