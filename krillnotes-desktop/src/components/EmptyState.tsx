function EmptyState() {
  return (
    <div className="flex items-center justify-center min-h-screen">
      <div className="text-center">
        <h1 className="text-4xl font-bold mb-4">Krillnotes</h1>
        <p className="text-muted-foreground">
          Use File &gt; New Workspace or File &gt; Open Workspace to get started
        </p>
      </div>
    </div>
  );
}

export default EmptyState;
