export interface WorkspaceInfo {
  filename: string;
  path: string;
  noteCount: number;
}

export interface Note {
  id: string;
  title: string;
  nodeType: string;
  parentId: string | null;
  position: number;
  createdAt: number;
  modifiedAt: number;
  createdBy: number;
  modifiedBy: number;
  fields: Record<string, FieldValue>;
  isExpanded: boolean;
}

export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean };
