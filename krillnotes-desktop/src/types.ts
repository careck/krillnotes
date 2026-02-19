export interface WorkspaceInfo {
  filename: string;
  path: string;
  noteCount: number;
  selectedNoteId?: string;
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

export interface TreeNode {
  note: Note;
  children: TreeNode[];
}

export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean };

export interface FieldDefinition {
  name: string;
  fieldType: string;  // "text" | "number" | "boolean"
  required: boolean;
}

export enum DeleteStrategy {
  DeleteAll = "DeleteAll",
  PromoteChildren = "PromoteChildren",
}

export interface DeleteResult {
  deletedCount: number;
  affectedIds: string[];
}
