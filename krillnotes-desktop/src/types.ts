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
  | { Boolean: boolean }
  | { Date: string | null }   // ISO "YYYY-MM-DD" or null when not set
  | { Email: string };

export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email';

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
}

export interface SchemaInfo {
  fields: FieldDefinition[];
  titleCanView: boolean;
  titleCanEdit: boolean;
}

export enum DeleteStrategy {
  DeleteAll = "DeleteAll",
  PromoteChildren = "PromoteChildren",
}

export interface DeleteResult {
  deletedCount: number;
  affectedIds: string[];
}

export interface UserScript {
  id: string;
  name: string;
  description: string;
  sourceCode: string;
  loadOrder: number;
  enabled: boolean;
  createdAt: number;
  modifiedAt: number;
}
