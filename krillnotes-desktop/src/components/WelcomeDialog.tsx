interface WelcomeDialogProps {
  onDismiss: () => void;
}

function WelcomeDialog({ onDismiss }: WelcomeDialogProps) {
  return (
    <div className="min-h-screen bg-background text-foreground flex items-center justify-center">
      <div className="max-w-md bg-secondary p-8 rounded-lg text-center">
        <h1 className="text-3xl font-bold mb-4">Welcome to Krillnotes</h1>
        <p className="text-muted-foreground mb-6">
          You can start a new Workspace or load an existing one from the File menu
        </p>
        <button
          onClick={onDismiss}
          className="bg-primary text-primary-foreground px-6 py-2 rounded-md hover:bg-primary/90"
        >
          OK
        </button>
      </div>
    </div>
  );
}

export default WelcomeDialog;
