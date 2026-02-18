import type { Note } from '../types';

interface InfoPanelProps {
  selectedNote: Note | null;
}

function InfoPanel({ selectedNote }: InfoPanelProps) {
  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        Select a note to view details
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  return (
    <div className="p-6">
      <h1 className="text-4xl font-bold mb-6">{selectedNote.title}</h1>

      <div className="bg-secondary p-6 rounded-lg space-y-4">
        <div>
          <p className="text-sm text-muted-foreground">Type</p>
          <p className="text-lg">{selectedNote.nodeType}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Created</p>
          <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">Modified</p>
          <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
        </div>

        <div>
          <p className="text-sm text-muted-foreground">ID</p>
          <p className="text-xs font-mono">{selectedNote.id}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoPanel;
