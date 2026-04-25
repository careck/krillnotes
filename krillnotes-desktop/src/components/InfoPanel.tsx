// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import DOMPurify from 'dompurify';
import type { Note, FieldValue, SchemaInfo, AttachmentMeta, EffectiveRoleInfo, PermissionGrantRow, InheritedGrant } from '../types';
import FieldDisplay from './FieldDisplay';
import FieldEditor from './FieldEditor';
import TagPill from './TagPill';
import AttachmentsSection from './AttachmentsSection';
import { ChevronRight } from 'lucide-react';
import { defaultValueForFieldType, isEmptyFieldValue } from '../utils/fieldValue';
import { useSchema } from '../hooks/useSchema';
import { useNoteForm } from '../hooks/useNoteForm';

interface InfoPanelProps {
  selectedNote: Note | null;
  onNoteUpdated: () => void;
  onDeleteRequest: (noteId: string) => void;
  requestEditMode: number;
  onEditDone: () => void;
  onLinkNavigate: (noteId: string) => void;
  onBack: () => void;
  backNoteTitle?: string;
  refreshSignal?: number;
  effectiveRole?: EffectiveRoleInfo | null;
  onShareSubtree?: (noteId: string) => void;
  onRoleChange?: (noteId: string, userId: string, newRole: string, oldRole: string) => void;
  onRevokeGrant?: (noteId: string, userId: string) => void;
}

function InfoPanel({ selectedNote, onNoteUpdated, onDeleteRequest, requestEditMode, onEditDone, onLinkNavigate, onBack, backNoteTitle, refreshSignal, effectiveRole: effectiveRoleProp, onShareSubtree, onRoleChange, onRevokeGrant }: InfoPanelProps) {
  const { t } = useTranslation();
  const [recentlyDeleted, setRecentlyDeleted] = useState<AttachmentMeta[]>([]);
  const [authorNames, setAuthorNames] = useState<{ createdBy: string | null; modifiedBy: string | null }>({ createdBy: null, modifiedBy: null });
  const [verifiedBy, setVerifiedBy] = useState<string>('');
  const [roleInfo, setRoleInfo] = useState<EffectiveRoleInfo | null>(null);
  const [anchoredGrants, setAnchoredGrants] = useState<PermissionGrantRow[]>([]);
  const [inheritedGrants, setInheritedGrants] = useState<InheritedGrant[]>([]);
  const [nameMap, setNameMap] = useState<Record<string, string>>({});
  // isEditing is kept in InfoPanel (not in useNoteForm) to break the circular dependency:
  // useSchema needs isEditing, and useNoteForm needs schemaInfo from useSchema.
  const [isEditing, setIsEditing] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const viewHtmlRef = useRef<HTMLDivElement>(null);
  const pendingEditModeRef = useRef(false);

  // Stable ref so handleSchemaLoaded (defined before useNoteForm) can call
  // setEditedFields that comes from useNoteForm (defined after useSchema).
  const setEditedFieldsRef = useRef<React.Dispatch<React.SetStateAction<Record<string, FieldValue>>> | null>(null);

  const handleSchemaLoaded = useCallback((schema: SchemaInfo) => {
    setEditedFieldsRef.current?.(prev => {
      const merged = { ...prev };
      for (const field of schema.fields) {
        if (!(field.name in merged)) {
          merged[field.name] = defaultValueForFieldType(field.fieldType);
        }
      }
      return merged;
    });
    if (pendingEditModeRef.current) {
      pendingEditModeRef.current = false;
      setIsEditing(true);
    }
  }, []);

  const { schemaInfo, views, activeTab, setActiveTab, viewHtml, setViewHtml, previousTab, setPreviousTab, schemaLoadedRef } =
    useSchema(selectedNote, isEditing, handleSchemaLoaded);

  const {
    editedTitle, setEditedTitle, editedFields, setEditedFields,
    setIsDirty, editedTags, tagInput, tagSuggestions,
    groupCollapsed, setGroupCollapsed, groupVisible, fieldErrors, noteErrors, titleInputRef,
    handleFormKeyDown, handleEdit, handleCancel, handleSave,
    handleFieldChange, handleFieldBlur, addTag, removeTag, handleTagInputChange,
  } = useNoteForm(
    selectedNote, schemaInfo,
    { activeTab, setActiveTab, previousTab, setPreviousTab, setViewHtml },
    onNoteUpdated, onEditDone,
    isEditing, setIsEditing,
  );

  // Wire the stable ref so handleSchemaLoaded can call setEditedFields from the hook
  setEditedFieldsRef.current = setEditedFields;

  // Enter edit mode when WorkspaceView requests it (e.g. via context menu, note creation).
  // NOTE: This effect must be declared AFTER the selectedNote?.id effects above.
  // Two cases are handled to avoid a race between the schema IPC fetch and the
  // requestEditMode increment:
  // - Schema already loaded: enter edit mode immediately (schemaLoadedRef is true).
  // - Schema still loading: set pendingEditModeRef so the schema .then() picks it up.
  // This prevents both the "title flash" (entering edit mode before titleCanEdit arrives)
  // and the inverse race where the schema resolves before requestEditMode fires.
  //
  // IMPORTANT: selectedNote is intentionally omitted from the dep array.
  // Including it would cause this effect to re-fire on every note selection while
  // requestEditMode > 0 (i.e. after the user has ever pressed Enter), spuriously
  // setting pendingEditModeRef = true and forcing every subsequently selected note
  // into edit mode — which also blocks the render_view effect (!isEditing guard).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (requestEditMode > 0 && selectedNote) {
      if (schemaLoadedRef.current) {
        setIsEditing(true);
      } else {
        pendingEditModeRef.current = true;
      }
    }
  }, [requestEditMode]);

  // Fetch HTML for the active custom view tab
  const activeViewHtml = activeTab !== 'fields' ? viewHtml[activeTab] ?? null : null;

  // Hydrate img[data-kn-attach-id] placeholders with real base64 data after the view HTML renders
  useEffect(() => {
    const container = viewHtmlRef.current;
    if (!container || !activeViewHtml) return;

    const imgs = Array.from(
      container.querySelectorAll<HTMLImageElement>('img[data-kn-attach-id]')
    );
    Promise.all(
      imgs.map(async (img) => {
        const attachmentId = img.getAttribute('data-kn-attach-id')!;
        const widthAttr = img.getAttribute('data-kn-width');
        try {
          const result = await invoke<{ data: string; mime_type: string | null }>('get_attachment_data', { attachmentId });
          const mime = result.mime_type ?? 'image/png';
          img.src = `data:${mime};base64,${result.data}`;
          if (widthAttr && parseInt(widthAttr, 10) > 0) {
            img.style.maxWidth = `${widthAttr}px`;
            img.style.height = 'auto';
          }
          img.removeAttribute('data-kn-attach-id');
          img.removeAttribute('data-kn-width');
        } catch {
          const span = document.createElement('span');
          span.className = 'kn-image-error';
          span.textContent = t('fields.imageNotFound');
          img.replaceWith(span);
        }
      })
    ).catch(err => console.error('Image hydration error:', err));
  }, [activeViewHtml]);

  // Hydrate [data-kn-embed-type] sentinels into click-to-play media cards
  useEffect(() => {
    const container = viewHtmlRef.current;
    if (!container || !activeViewHtml) return;

    const sentinels = Array.from(
      container.querySelectorAll<HTMLElement>('[data-kn-embed-type]')
    );

    sentinels.forEach((el) => {
      const type = el.getAttribute('data-kn-embed-type');
      const id   = el.getAttribute('data-kn-embed-id') ?? '';
      const url  = el.getAttribute('data-kn-embed-url') ?? '';

      const card = document.createElement('div');

      if (type === 'youtube' && id) {
        card.className = 'kn-media-thumbnail';
        const img = document.createElement('img');
        img.src = `https://img.youtube.com/vi/${id}/hqdefault.jpg`;
        img.alt = 'Video thumbnail';
        const play = document.createElement('div');
        play.className = 'kn-media-play-btn';
        play.textContent = '▶';
        card.appendChild(img);
        card.appendChild(play);
      } else if (type === 'instagram') {
        card.className = 'kn-media-card kn-media-card--instagram';
        const label = document.createElement('span');
        label.className = 'kn-media-card-label';
        label.textContent = t('fields.openOnInstagram');
        card.appendChild(label);
      } else {
        return; // unknown type — leave sentinel in place
      }

      card.addEventListener('click', () => {
        if (url.startsWith('https://') || url.startsWith('http://')) {
          openUrl(url);
        }
      });
      el.replaceWith(card);
    });
  }, [activeViewHtml]);

  useEffect(() => {
    if (!selectedNote) {
      setAuthorNames({ createdBy: null, modifiedBy: null });
      return;
    }
    const { createdBy, modifiedBy } = selectedNote;
    Promise.all([
      createdBy ? invoke<string | null>('resolve_identity_name', { publicKey: createdBy }) : Promise.resolve(null),
      modifiedBy ? invoke<string | null>('resolve_identity_name', { publicKey: modifiedBy }) : Promise.resolve(null),
    ]).then(([cb, mb]) => setAuthorNames({ createdBy: cb, modifiedBy: mb }));
  }, [selectedNote?.createdBy, selectedNote?.modifiedBy]);

  useEffect(() => {
    if (!selectedNote) { setVerifiedBy(''); return; }
    invoke<string>('get_note_verified_by', { noteId: selectedNote.id })
      .then(setVerifiedBy)
      .catch(() => setVerifiedBy(''));
  }, [selectedNote?.id, selectedNote?.modifiedAt]);

  useEffect(() => {
    if (!selectedNote) { setRoleInfo(null); return; }
    invoke<EffectiveRoleInfo>('get_effective_role', { noteId: selectedNote.id })
      .then(setRoleInfo)
      .catch(() => setRoleInfo(null));
  }, [selectedNote?.id, refreshSignal]);

  const activeRole = effectiveRoleProp ?? roleInfo;

  useEffect(() => {
    if (!selectedNote || !activeRole || (activeRole.role !== 'owner' && activeRole.role !== 'root_owner')) {
      setAnchoredGrants([]);
      setInheritedGrants([]);
      return;
    }
    const load = async () => {
      try {
        const [anchored, inherited] = await Promise.all([
          invoke<PermissionGrantRow[]>('get_note_permissions', { noteId: selectedNote.id }),
          invoke<InheritedGrant[]>('get_inherited_permissions', { noteId: selectedNote.id }),
        ]);
        setAnchoredGrants(anchored);
        setInheritedGrants(inherited);

        const allKeys = new Set<string>();
        anchored.forEach(g => { allKeys.add(g.userId); allKeys.add(g.grantedBy); });
        inherited.forEach(g => { allKeys.add(g.grant.userId); allKeys.add(g.grant.grantedBy); });
        if (activeRole?.grantedBy) allKeys.add(activeRole.grantedBy);

        const names: Record<string, string> = {};
        await Promise.all(
          Array.from(allKeys).map(async (key) => {
            try {
              names[key] = await invoke<string>('resolve_identity_name', { publicKey: key });
            } catch {
              names[key] = key.slice(0, 8) + '…';
            }
          })
        );
        setNameMap(names);
      } catch {
        setAnchoredGrants([]);
        setInheritedGrants([]);
      }
    };
    load();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNote?.id, activeRole?.role, refreshSignal]);

  if (!selectedNote) {
    return (
      <div className="flex items-center justify-center h-full text-muted-foreground">
        {t('notes.selectNote')}
      </div>
    );
  }

  const formatTimestamp = (timestamp: number) => {
    return new Date(timestamp * 1000).toLocaleString();
  };

  const schemaFieldNames = new Set([
    ...schemaInfo.fields.map(f => f.name),
    ...schemaInfo.fieldGroups.flatMap(g => g.fields.map(f => f.name)),
  ]);
  const allFieldNames = Object.keys(selectedNote.fields);
  const legacyFieldNames = allFieldNames.filter(name => !schemaFieldNames.has(name));

  const canEdit = !activeRole || activeRole.role === 'owner' || activeRole.role === 'root_owner' || activeRole.role === 'writer';

  return (
    <div ref={panelRef} className={`p-6 ${isEditing ? 'border-2 border-primary rounded-lg' : ''}`} onKeyDown={handleFormKeyDown}>
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        {isEditing ? (
          schemaInfo.titleCanEdit ? (
            <input
              ref={titleInputRef}
              type="text"
              value={editedTitle}
              onChange={(e) => {
                setEditedTitle(e.target.value);
                setIsDirty(true);
              }}
              className="text-4xl font-bold bg-background border border-border rounded-md px-2 py-1 flex-1"
              autoCorrect="off"
              autoCapitalize="off"
              spellCheck={false}
            />
          ) : (
            <div className="flex-1" />
          )
        ) : (
          schemaInfo.titleCanView ? (
            <h1 className="text-4xl font-bold">{selectedNote.title}</h1>
          ) : null
        )}
        <div className="flex gap-2 ml-4">
          {isEditing ? (
            <>
              <button
                onClick={handleSave}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                {t('common.save')}
              </button>
              <button
                onClick={handleCancel}
                className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
              >
                {t('common.cancel')}
              </button>
            </>
          ) : (
            <>
              {canEdit && (
                <button
                  onClick={handleEdit}
                  className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
                >
                  {t('common.edit')}
                </button>
              )}
              {canEdit && (
                <button
                  onClick={() => onDeleteRequest(selectedNote.id)}
                  className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
                >
                  {t('common.delete')}
                </button>
              )}
            </>
          )}
        </div>
      </div>

      {/* Back navigation — shown whenever history is non-empty, regardless of view type */}
      {!isEditing && backNoteTitle !== undefined && (
        <div className="kn-view-back">
          <button onClick={onBack}>{t('notes.backTo', { title: backNoteTitle })}</button>
        </div>
      )}

      {/* Tab bar — only when registered views exist */}
      {views.length > 0 && !isEditing && (
        <div className="flex border-b border-border mb-4">
          {[...views]
            .sort((a, b) => (b.displayFirst ? 1 : 0) - (a.displayFirst ? 1 : 0))
            .map(v => (
              <button
                key={v.label}
                className={`px-3 py-1.5 text-sm ${activeTab === v.label
                  ? 'border-b-2 border-primary font-medium'
                  : 'text-muted-foreground hover:text-foreground'}`}
                onClick={() => setActiveTab(v.label)}
              >
                {v.label}
              </button>
            ))
          }
          <button
            className={`px-3 py-1.5 text-sm ${activeTab === 'fields'
              ? 'border-b-2 border-primary font-medium'
              : 'text-muted-foreground hover:text-foreground'}`}
            onClick={() => setActiveTab('fields')}
          >
            {t('notes.fields')}
          </button>
        </div>
      )}

      {/* Fields Section */}
      <div className="mb-6">
        {/* Custom view HTML — shown only in view mode when a custom tab is active */}
        {!isEditing && activeViewHtml && (
          <div
            ref={viewHtmlRef}
            dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(activeViewHtml, { ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id', 'data-kn-embed-type', 'data-kn-embed-id', 'data-kn-embed-url'], ALLOWED_URI_REGEXP: /^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp):|data:image\/|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i }) }}
            onClick={(e) => {
              setDownloadError(null);
              const target = e.target as Element;

              const downloadLink = target.closest('[data-kn-download-id]');
              if (downloadLink) {
                e.preventDefault();
                const attachmentId = downloadLink.getAttribute('data-kn-download-id')!;
                const filename = downloadLink.textContent?.trim() ?? 'download';
                invoke<{ data: string; mime_type: string | null }>('get_attachment_data', { attachmentId })
                  .then(result => {
                    const bytes = Uint8Array.from(atob(result.data), c => c.charCodeAt(0));
                    const blob = new Blob([bytes]);
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement('a');
                    a.href = url;
                    a.download = filename;
                    a.click();
                    setTimeout(() => URL.revokeObjectURL(url), 100);
                  })
                  .catch(err => setDownloadError(String(err)));
                return;
              }

              const noteLink = target.closest('.kn-view-link');
              if (noteLink) {
                e.preventDefault();
                const noteId = noteLink.getAttribute('data-note-id');
                if (noteId) onLinkNavigate(noteId);
                return;
              }

              const anchor = target.closest('a[href]') as HTMLAnchorElement | null;
              if (anchor) {
                e.preventDefault();
                openUrl(anchor.href);
              }
            }}
          />
        )}

        {downloadError && (
          <p className="text-xs text-red-500 px-1 py-1">{downloadError}</p>
        )}

        {/* Tag pills — shown only in view mode */}
        {!isEditing && selectedNote.tags.length > 0 && (
          <div className="kn-view-tags">
            {selectedNote.tags.map(tag => (
              <TagPill key={tag} tag={tag} />
            ))}
          </div>
        )}

        {/* Tag editor — shown only in edit mode */}
        {isEditing && (
          <div className="kn-tag-editor">
            <div className="kn-tag-editor__pills">
              {editedTags.map(tag => (
                <TagPill key={tag} tag={tag} onRemove={() => removeTag(tag)} />
              ))}
            </div>
            <div className="kn-tag-editor__input-wrap">
              <input
                className="kn-tag-editor__input"
                placeholder={t('tags.addPlaceholder')}
                value={tagInput}
                onChange={e => handleTagInputChange(e.target.value)}
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                onKeyDown={e => {
                  if (e.key === 'Enter' || e.key === 'Tab') {
                    e.preventDefault();
                    if (tagSuggestions.length > 0) addTag(tagSuggestions[0]);
                    else if (tagInput.trim()) addTag(tagInput);
                  }
                }}
              />
              {tagSuggestions.length > 0 && (
                <ul className="kn-tag-editor__suggestions">
                  {tagSuggestions.map(t => (
                    <li key={t} onMouseDown={() => addTag(t)}>{t}</li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        )}

        {/* Default field rendering — shown in edit mode, or when no custom view exists */}
        {(isEditing || activeTab === 'fields') && <h2 className="text-xl font-semibold mb-4">{t('notes.fields')}</h2>}

        {/* Note-level validation errors banner */}
        {isEditing && noteErrors.length > 0 && (
          <div className="mb-4 p-3 bg-red-50 border border-red-300 rounded-md">
            {noteErrors.map((msg, i) => (
              <p key={i} className="text-sm text-red-600">{msg}</p>
            ))}
          </div>
        )}

        {isEditing ? (
          <>
            {/* Top-level fields */}
            {schemaInfo.fields.filter(field => field.canEdit).map(field => (
              <FieldEditor
                key={field.name}
                fieldName={field.name}
                fieldType={field.fieldType}
                value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
                required={field.required}
                options={field.options}
                max={field.max}
                targetSchema={field.targetSchema}
                noteId={selectedNote.id}
                fieldDef={field}
                error={fieldErrors[field.name]}
                onBlur={() => handleFieldBlur(field.name, field)}
                onChange={(value) => handleFieldChange(field.name, value)}
              />
            ))}

            {/* Field groups */}
            {schemaInfo.fieldGroups.map(group => {
              const isVisible = !group.hasVisibleClosure || (groupVisible[group.name] !== false);
              const hasData = group.fields.some(f =>
                !isEmptyFieldValue(editedFields[f.name] ?? defaultValueForFieldType(f.fieldType))
              );
              if (!isVisible && !hasData) return null;

              const isCollapsed = groupCollapsed[group.name] ?? group.collapsed;
              return (
                <div key={group.name} className={`mt-4 border border-border rounded-lg ${!isVisible ? 'opacity-50' : ''}`}>
                  <button
                    type="button"
                    className="w-full px-4 py-2 text-left flex items-center gap-2 font-medium text-sm select-none"
                    onClick={() => setGroupCollapsed(prev => ({ ...prev, [group.name]: !isCollapsed }))}
                  >
                    <ChevronRight size={14} className={`transition-transform ${isCollapsed ? '' : 'rotate-90'}`} />
                    {group.name}
                    {!isVisible && <span className="text-xs text-muted-foreground ml-2 font-normal">{t('fields.hiddenDataExists')}</span>}
                  </button>
                  {!isCollapsed && (
                    <div className="px-4 pb-2 pt-1">
                      {group.fields.filter(f => f.canEdit).map(field => (
                        <FieldEditor
                          key={field.name}
                          fieldName={field.name}
                          fieldType={field.fieldType}
                          value={editedFields[field.name] ?? defaultValueForFieldType(field.fieldType)}
                          required={field.required}
                          options={field.options}
                          max={field.max}
                          targetSchema={field.targetSchema}
                          noteId={selectedNote.id}
                          fieldDef={field}
                          error={fieldErrors[field.name]}
                          onBlur={() => handleFieldBlur(field.name, field)}
                          onChange={(value) => handleFieldChange(field.name, value)}
                        />
                      ))}
                    </div>
                  )}
                </div>
              );
            })}
          </>
        ) : (activeTab === 'fields' && (() => {
          const visibleTopFields = schemaInfo.fields
            .filter(field => field.canView)
            .filter(field => !isEmptyFieldValue(selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)));

          const visibleGroups = schemaInfo.fieldGroups.filter(group => {
            if (group.hasVisibleClosure && groupVisible[group.name] === false) return false;
            return group.fields.some(f =>
              f.canView && !isEmptyFieldValue(selectedNote.fields[f.name] ?? defaultValueForFieldType(f.fieldType))
            );
          });

          if (visibleTopFields.length === 0 && visibleGroups.length === 0) return null;
          return (
            <>
              {visibleTopFields.length > 0 && (
                <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
                  {visibleTopFields.map(field => (
                    <FieldDisplay
                      key={field.name}
                      fieldName={field.name}
                      fieldType={field.fieldType}
                      value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
                      max={field.max}
                      noteId={selectedNote.id}
                    />
                  ))}
                </dl>
              )}
              {visibleGroups.map(group => {
                const groupVisibleFields = group.fields
                  .filter(f => f.canView)
                  .filter(f => !isEmptyFieldValue(selectedNote.fields[f.name] ?? defaultValueForFieldType(f.fieldType)));
                if (groupVisibleFields.length === 0) return null;
                const isCollapsed = groupCollapsed[group.name] ?? group.collapsed;
                return (
                  <div key={group.name} className="mt-4 border border-border rounded-lg">
                    <button
                      type="button"
                      className="w-full px-4 py-2 text-left flex items-center gap-2 font-medium text-sm select-none"
                      onClick={() => setGroupCollapsed(prev => ({ ...prev, [group.name]: !isCollapsed }))}
                    >
                      <ChevronRight size={14} className={`transition-transform ${isCollapsed ? '' : 'rotate-90'}`} />
                      {group.name}
                    </button>
                    {!isCollapsed && (
                      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 px-4 pb-3">
                        {groupVisibleFields.map(field => (
                          <FieldDisplay
                            key={field.name}
                            fieldName={field.name}
                            fieldType={field.fieldType}
                            value={selectedNote.fields[field.name] ?? defaultValueForFieldType(field.fieldType)}
                            max={field.max}
                            noteId={selectedNote.id}
                          />
                        ))}
                      </dl>
                    )}
                  </div>
                );
              })}
            </>
          );
        })())}

        {(activeTab === 'fields' || isEditing) && legacyFieldNames.length > 0 && (() => {
          if (isEditing) {
            return (
              <>
                <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
                  {t('notes.legacyFields')}
                </h3>
                {legacyFieldNames.map(name => (
                  <FieldEditor
                    key={name}
                    fieldName={`${name}${t('notes.legacySuffix')}`}
                    fieldType="text"
                    value={editedFields[name] ?? { Text: '' }}
                    required={false}
                    options={[]}
                    max={0}
                    onChange={(value) => handleFieldChange(name, value)}
                  />
                ))}
              </>
            );
          }
          const visibleLegacy = legacyFieldNames.filter(
            name => !isEmptyFieldValue(selectedNote.fields[name])
          );
          if (visibleLegacy.length === 0) return null;
          return (
            <>
              <h3 className="text-lg font-medium text-muted-foreground mt-6 mb-3">
                {t('notes.legacyFields')}
              </h3>
              <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
                {visibleLegacy.map(name => (
                  <FieldDisplay
                    key={name}
                    fieldName={`${name}${t('notes.legacySuffix')}`}
                    fieldType="text"
                    value={selectedNote.fields[name]}
                    noteId={selectedNote.id}
                  />
                ))}
              </dl>
            </>
          );
        })()}

        {!isEditing && activeTab === 'fields' &&
          schemaInfo.fields.filter(f =>
            f.canView && !isEmptyFieldValue(selectedNote.fields[f.name] ?? defaultValueForFieldType(f.fieldType))
          ).length === 0 &&
          schemaInfo.fieldGroups.every(g =>
            g.fields.every(f => !f.canView || isEmptyFieldValue(selectedNote.fields[f.name] ?? defaultValueForFieldType(f.fieldType)))
          ) &&
          legacyFieldNames.filter(n => !isEmptyFieldValue(selectedNote.fields[n])).length === 0 && (
            <p className="text-muted-foreground italic">{t('notes.noFields')}</p>
          )
        }
        {isEditing && schemaInfo.fields.length === 0 && schemaInfo.fieldGroups.length === 0 && legacyFieldNames.length === 0 && (
          <p className="text-muted-foreground italic">{t('notes.noFields')}</p>
        )}
      </div>

      {/* Attachments */}
      {schemaInfo?.allowAttachments && (
        <AttachmentsSection
          noteId={selectedNote?.id ?? null}
          allowedTypes={schemaInfo.attachmentTypes}
          refreshSignal={refreshSignal}
          recentlyDeleted={recentlyDeleted}
          onRecentlyDeletedChange={setRecentlyDeleted}
        />
      )}

      {/* Metadata Section — always visible when no custom views; only on Fields tab when views exist */}
      {(views.length === 0 || activeTab === 'fields') && <details className="bg-secondary rounded-lg">
        <summary className="px-6 py-4 cursor-pointer list-none flex items-center gap-2 text-sm font-medium text-muted-foreground select-none">
          <ChevronRight size={16} className="[details[open]_&]:rotate-90 transition-transform" />
          {t('notes.info')}
        </summary>
        <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 px-6 pb-6">
          <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('notes.type')}</dt>
          <dd className="m-0 text-foreground text-sm">{selectedNote.schema}</dd>

          <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('notes.created')}</dt>
          <dd className="m-0 text-foreground text-sm">
            {formatTimestamp(selectedNote.createdAt)}
            {authorNames.createdBy && <span className="text-muted-foreground"> · {authorNames.createdBy}</span>}
          </dd>

          <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('notes.modified')}</dt>
          <dd className="m-0 text-foreground text-sm">
            {formatTimestamp(selectedNote.modifiedAt)}
            {authorNames.modifiedBy && <span className="text-muted-foreground"> · {authorNames.modifiedBy}</span>}
          </dd>

          {verifiedBy && (
            <>
              <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('info.verifiedBy')}</dt>
              <dd className="m-0 text-foreground text-sm">{verifiedBy}</dd>
            </>
          )}

          <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('notes.id')}</dt>
          <dd className="m-0 text-foreground text-xs font-mono break-all">{selectedNote.id}</dd>

          {activeRole && activeRole.role !== 'none' && (
            <>
              <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">{t('info.yourRole', 'Your role')}</dt>
              <dd className="m-0">
                <div className="flex flex-col">
                  <span className="flex items-center gap-1 text-sm">
                    <span className={
                      activeRole.role === 'owner' || activeRole.role === 'root_owner' ? 'text-green-500' :
                      activeRole.role === 'writer' ? 'text-orange-500' :
                      activeRole.role === 'reader' ? 'text-yellow-500' : ''
                    }>●</span>
                    <span className="capitalize">
                      {activeRole.role === 'root_owner'
                        ? t('info.roleRootOwner', 'Owner (Root)')
                        : t(`roles.${activeRole.role}Short`, activeRole.role)}
                    </span>
                  </span>
                  {activeRole.inheritedFrom && activeRole.inheritedFromTitle && (
                    <span className="text-muted-foreground text-xs">
                      {t('info.inheritedFrom', 'Inherited from')}{' '}
                      <button
                        className="text-blue-500 hover:underline"
                        onClick={() => onLinkNavigate(activeRole.inheritedFrom!)}
                      >
                        {activeRole.inheritedFromTitle}
                      </button>
                    </span>
                  )}
                </div>
              </dd>
            </>
          )}
        </dl>

        {/* Shared with section — owners only */}
        {(activeRole?.role === 'owner' || activeRole?.role === 'root_owner') && (
          <div className="mt-3 border-t dark:border-zinc-700 pt-2 px-6 pb-4">
            {/* Anchored grants */}
            {anchoredGrants.length > 0 && (
              <div className="mb-2">
                <p className="text-xs font-medium text-zinc-500 mb-1">
                  {t('info.sharedAnchored', 'Shared with — anchored here')}
                </p>
                {anchoredGrants.map(grant => (
                  <div key={grant.userId} className="flex items-center gap-1.5 py-0.5 text-xs">
                    <span className={
                      grant.role === 'owner' ? 'text-green-500' :
                      grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
                    }>●</span>
                    <span className="flex-1 truncate">{nameMap[grant.userId] ?? grant.userId.slice(0, 8)}</span>
                    <select
                      className="text-xs border rounded px-1 py-0.5 dark:bg-zinc-800 dark:border-zinc-600"
                      value={grant.role}
                      onChange={(e) => onRoleChange?.(selectedNote.id, grant.userId, e.target.value, grant.role)}
                    >
                      {(activeRole?.role === 'root_owner' || activeRole?.role === 'owner') && (
                        <option value="owner">{t('roles.ownerShort', 'Owner')}</option>
                      )}
                      <option value="writer">{t('roles.writerShort', 'Writer')}</option>
                      <option value="reader">{t('roles.readerShort', 'Reader')}</option>
                    </select>
                    <button
                      onClick={() => onRevokeGrant?.(selectedNote.id, grant.userId)}
                      className="text-red-400 hover:text-red-600 px-1"
                      title={t('info.revoke', 'Revoke')}
                    >✕</button>
                  </div>
                ))}
              </div>
            )}

            {/* Inherited grants */}
            {inheritedGrants.length > 0 && (
              <div className="mb-2">
                <p className="text-xs font-medium text-zinc-500 mb-1">
                  {t('info.accessFromParent', 'Access from parent grants')}
                </p>
                {inheritedGrants.map(ig => (
                  <div key={`${ig.grant.userId}-${ig.anchorNoteId}`} className="flex items-center gap-1.5 py-0.5 text-xs opacity-60">
                    <span className={
                      ig.grant.role === 'owner' ? 'text-green-500' :
                      ig.grant.role === 'writer' ? 'text-orange-500' : 'text-yellow-500'
                    }>●</span>
                    <span className="flex-1 truncate">{nameMap[ig.grant.userId] ?? ig.grant.userId.slice(0, 8)}</span>
                    <span className="text-zinc-400">{ig.grant.role}</span>
                    <button
                      onClick={() => onLinkNavigate(ig.anchorNoteId)}
                      className="text-blue-500 hover:underline text-[11px]"
                    >
                      {t('info.via', 'via')} {ig.anchorNoteTitle ?? ig.anchorNoteId.slice(0, 8)}
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Share button */}
            <button
              onClick={() => onShareSubtree?.(selectedNote.id)}
              className="text-xs text-blue-600 hover:underline mt-1"
            >
              + {t('info.shareSubtree', 'Share this subtree...')}
            </button>
          </div>
        )}
      </details>}
    </div>
  );
}

// Prevent re-renders when WorkspaceView re-renders due to hover/drag/dialog state changes that
// only produce new callback references. Without this guard, React re-processes the view HTML
// prop on every re-render, which can reset the DOM and wipe the hydrated img.src values set
// by the image hydration effect (which does not re-run because activeViewHtml is unchanged).
export default memo(InfoPanel, (prev, next) =>
  prev.selectedNote === next.selectedNote &&
  prev.requestEditMode === next.requestEditMode &&
  prev.backNoteTitle === next.backNoteTitle &&
  prev.refreshSignal === next.refreshSignal &&
  prev.effectiveRole === next.effectiveRole &&
  prev.onDeleteRequest === next.onDeleteRequest &&
  prev.onEditDone === next.onEditDone &&
  prev.onLinkNavigate === next.onLinkNavigate &&
  prev.onBack === next.onBack &&
  prev.onShareSubtree === next.onShareSubtree &&
  prev.onRoleChange === next.onRoleChange &&
  prev.onRevokeGrant === next.onRevokeGrant,
);
