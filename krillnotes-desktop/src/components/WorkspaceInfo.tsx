import type { WorkspaceInfo as WorkspaceInfoType } from '../types';

interface WorkspaceInfoProps {
  info: WorkspaceInfoType;
}

function WorkspaceInfo({ info }: WorkspaceInfoProps) {
  return (
    <div className="max-w-2xl mx-auto">
      <h1 className="text-4xl font-bold mb-2">{info.filename}</h1>
      <p className="text-muted-foreground mb-6">{info.path}</p>

      <div className="bg-secondary p-6 rounded-lg">
        <div className="grid grid-cols-2 gap-4">
          <div>
            <p className="text-sm text-muted-foreground">Notes</p>
            <p className="text-2xl font-semibold">{info.noteCount}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">Status</p>
            <p className="text-lg">Ready</p>
          </div>
        </div>
      </div>

      <p className="mt-6 text-sm text-muted-foreground">
        Phase 3 will add tree view for browsing notes
      </p>
    </div>
  );
}

export default WorkspaceInfo;
