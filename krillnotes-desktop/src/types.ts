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

export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email' | 'select' | 'rating';

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];   // non-empty for 'select' fields
  max: number;         // non-zero for 'rating' fields
}

export interface SchemaInfo {
  fields: FieldDefinition[];
  titleCanView: boolean;
  titleCanEdit: boolean;
  childrenSort: 'asc' | 'desc' | 'none';
  allowedParentTypes: string[];
  allowedChildrenTypes: string[];
  hasViewHook: boolean;
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

export interface DropIndicator {
  noteId: string;
  position: 'before' | 'after' | 'child';
}

export interface OperationSummary {
  operationId: string;
  timestamp: number;
  deviceId: string;
  operationType: string;
  targetName: string;
}

export interface AppSettings {
  workspaceDirectory: string;
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  isOpen: boolean;
}
