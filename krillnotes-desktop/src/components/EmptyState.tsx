function EmptyState() {
  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="text-center">
        <img
          src="/KrillNotesLogo.png"
          alt="KrillNotes"
          className="w-64 h-64 mx-auto mb-6 object-contain"
        />
        <p className="text-muted-foreground">
          Use File &gt; New Workspace or File &gt; Open Workspace to get started
        </p>
      </div>
    </div>
  );
}

export default EmptyState;
