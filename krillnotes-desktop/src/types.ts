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
  tags: string[];
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
  | { Email: string }
  | { NoteLink: string | null };  // null = not set, string = linked note UUID

export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email' | 'select' | 'rating' | 'note_link';

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];   // non-empty for 'select' fields
  max: number;         // non-zero for 'rating' fields
  targetType?: string;  // only meaningful for note_link fields
  showOnHover: boolean;
}

export interface NoteSearchResult {
  id: string;
  title: string;
}

export interface SchemaInfo {
  fields: FieldDefinition[];
  titleCanView: boolean;
  titleCanEdit: boolean;
  childrenSort: 'asc' | 'desc' | 'none';
  allowedParentTypes: string[];
  allowedChildrenTypes: string[];
  hasViewHook: boolean;
  hasHoverHook: boolean;
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

export interface ScriptError {
  scriptName: string;
  message: string;
}

export interface ScriptMutationResult<T> {
  data: T;
  loadErrors: ScriptError[];
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
  cacheWorkspacePasswords: boolean;
  activeThemeMode?: string;
  lightTheme?: string;
  darkTheme?: string;
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  isOpen: boolean;
}

export interface WorkspaceMetadata {
  version: number;
  authorName?: string;
  authorOrg?: string;
  homepageUrl?: string;
  description?: string;
  license?: string;
  licenseUrl?: string;
  language?: string;
  /** Workspace-level taxonomy tags for gallery discovery (not per-note tags). */
  tags: string[];
}
