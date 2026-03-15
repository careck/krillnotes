// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

export interface WorkspaceInfo {
  filename: string;
  path: string;
  noteCount: number;
  selectedNoteId?: string;
  identityUuid?: string;
}

export interface Note {
  id: string;
  title: string;
  schema: string;
  parentId: string | null;
  position: number;
  createdAt: number;
  modifiedAt: number;
  createdBy: string;
  modifiedBy: string;
  fields: Record<string, FieldValue>;
  isExpanded: boolean;
  tags: string[];
  schemaVersion: number;
}

export interface TreeNode {
  note: Note;
  children: TreeNode[];
}

export interface SchemaMigratedEvent {
  schemaName: string;
  fromVersion: number;
  toVersion: number;
  notesMigrated: number;
}

export type FieldValue =
  | { Text: string }
  | { Number: number }
  | { Boolean: boolean }
  | { Date: string | null }   // ISO "YYYY-MM-DD" or null when not set
  | { Email: string }
  | { NoteLink: string | null }  // null = not set, string = linked note UUID
  | { File: string | null };     // null = not set, string = attachment UUID

export type FieldType = 'text' | 'textarea' | 'number' | 'boolean' | 'date' | 'email' | 'select' | 'rating' | 'note_link' | 'file';

export interface FieldDefinition {
  name: string;
  fieldType: FieldType;
  required: boolean;
  canView: boolean;
  canEdit: boolean;
  options: string[];       // non-empty for 'select' fields
  max: number;             // non-zero for 'rating' fields
  targetSchema?: string;   // only meaningful for note_link fields
  showOnHover: boolean;
  allowedTypes: string[];  // MIME types; empty = all allowed; only meaningful for 'file' fields
  hasValidate: boolean;    // true if a validate closure is registered for this field
}

export interface FieldGroup {
  name: string;
  fields: FieldDefinition[];
  collapsed: boolean;
  hasVisibleClosure: boolean;
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
  allowedParentSchemas: string[];
  allowedChildrenSchemas: string[];
  isLeaf: boolean;
  hasViews: boolean;
  hasHover: boolean;
  allowAttachments: boolean;
  attachmentTypes: string[];
  fieldGroups: FieldGroup[];
}

export interface ViewInfo {
  label: string;
  displayFirst: boolean;
}

export interface ScriptWarning {
  scriptName: string;
  message: string;
}

export enum DeleteStrategy {
  DeleteAll = "DeleteAll",
  PromoteChildren = "PromoteChildren",
}

export interface DeleteResult {
  deletedCount: number;
  affectedIds: string[];
}

export type SaveResult =
  | { ok: Note }
  | {
      validationErrors: {
        fieldErrors: Record<string, string>;
        noteErrors: string[];
        previewTitle: string | null;
        previewFields: Record<string, FieldValue>;
      };
    };

export interface UserScript {
  id: string;
  name: string;
  description: string;
  sourceCode: string;
  loadOrder: number;
  enabled: boolean;
  createdAt: number;
  modifiedAt: number;
  category: string;
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
  timestampWallMs: number;  // milliseconds
  deviceId: string;
  operationType: string;
  targetName: string;
  authorKey: string;        // first 8 chars of base64 key, or ""
}

export interface AppSettings {
  workspaceDirectory: string;
  activeThemeMode?: string;
  lightTheme?: string;
  darkTheme?: string;
  language?: string;
}

export interface WorkspaceEntry {
  name: string;
  path: string;
  isOpen: boolean;
  lastModified: number;       // Unix timestamp (seconds)
  sizeBytes: number;
  createdAt: number | null;
  noteCount: number | null;
  attachmentCount: number | null;
  workspaceUuid: string | null;
  identityUuid: string | null;
  identityName: string | null;
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

export interface AttachmentMeta {
  id: string;
  noteId: string;
  filename: string;
  mimeType: string | null;
  sizeBytes: number;
  hashSha256: string;
  salt: string;
  createdAt: number;
}

export interface UndoResult {
  affectedNoteId: string | null;
}

export interface IdentityRef {
  uuid: string;
  displayName: string;
  file: string;
  lastUsed: string;  // ISO 8601
}

export interface WorkspaceBindingInfo {
  workspaceUuid: string;
  folderPath: string;
}

export type TrustLevel = 'Tofu' | 'CodeVerified' | 'Vouched' | 'VerifiedInPerson';

export interface ContactInfo {
  contactId: string;
  declaredName: string;
  localName: string | null;
  publicKey: string;
  fingerprint: string;
  trustLevel: TrustLevel;
  firstSeen: string; // ISO 8601
  notes: string | null;
}

export interface PeerInfo {
  peerDeviceId: string;
  peerIdentityId: string;
  displayName: string;
  fingerprint: string;
  trustLevel?: string;    // undefined if peer is not in the contact book
  contactId?: string;     // UUID string, undefined if not in contacts
  lastSync?: string;      // ISO 8601, undefined if never synced
  isOwner?: boolean;
  channelType: string;          // "relay" | "folder" | "manual"
  channelParams: string;        // JSON-encoded channel config, e.g. {"path":"/shared/folder"}
  syncStatus: string;           // "idle" | "syncing" | "error" | "auth_expired"
  syncStatusDetail: string | null;
  lastSyncError: string | null;
}

export interface SyncEvent {
  type: "delta_sent" | "bundle_applied" | "auth_expired" | "sync_error" | "ingest_error" | "unexpected_bundle_mode";
  workspaceId?: string;
  peerDeviceId?: string;
  opCount?: number;
  relayUrl?: string;
  error?: string;
  mode?: string;
}

export interface RelayAccountInfo {
  relayAccountId: string;
  relayUrl: string;
  email: string;
  sessionValid: boolean;
}

export interface InviteInfo {
  inviteId: string;
  workspaceId: string;
  workspaceName: string;
  createdAt: string;
  expiresAt: string | null;
  revoked: boolean;
  useCount: number;
}

export interface PendingPeer {
  inviteId: string;
  inviteePublicKey: string;
  inviteeDeclaredName: string;
  fingerprint: string;
}

export interface InviteFileData {
  inviteId: string;
  workspaceId: string;
  workspaceName: string;
  workspaceDescription: string | null;
  workspaceAuthorName: string | null;
  workspaceAuthorOrg: string | null;
  workspaceHomepageUrl: string | null;
  workspaceLicense: string | null;
  workspaceLanguage: string | null;
  workspaceTags: string[];
  inviterPublicKey: string;
  inviterDeclaredName: string;
  inviterFingerprint: string;
  expiresAt: string | null;
}

export interface SnapshotCreatedResult {
  savedPath: string;
  peerCount: number;
  asOfOperationId: string;
}

export interface GenerateDeltasResult {
  succeeded: string[];
  failed: [string, string][];
  filesWritten: string[];
}
