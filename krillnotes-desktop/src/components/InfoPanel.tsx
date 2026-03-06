// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { memo, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { confirm } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import DOMPurify from 'dompurify';
import type { Note, FieldValue, SchemaInfo, FieldDefinition, AttachmentMeta, SaveResult } from '../types';
import FieldDisplay from './FieldDisplay';
import FieldEditor from './FieldEditor';
import TagPill from './TagPill';
import AttachmentsSection from './AttachmentsSection';
import { ChevronRight } from 'lucide-react';

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
}

function defaultValueForFieldType(fieldType: string): FieldValue {
  switch (fieldType) {
    case 'boolean': return { Boolean: false };
    case 'number':  return { Number: 0 };
    case 'rating':  return { Number: 0 };
    case 'date':      return { Date: null };
    case 'email':     return { Email: '' };
    case 'note_link': return { NoteLink: null };
    case 'file':      return { File: null };
    default:          return { Text: '' }; // covers 'text', 'textarea', 'select'
  }
}

function isEmptyFieldValue(value: FieldValue): boolean {
  if ('Text' in value)     return value.Text === '';
  if ('Email' in value)    return value.Email === '';
  if ('Date' in value)     return value.Date === null;
  if ('NoteLink' in value) return value.NoteLink === null;
  if ('File' in value)     return value.File === null;
  return false; // Number and Boolean are never empty
}

function InfoPanel({ selectedNote, onNoteUpdated, onDeleteRequest, requestEditMode, onEditDone, onLinkNavigate, onBack, backNoteTitle, refreshSignal }: InfoPanelProps) {
  const { t } = useTranslation();
  const [schemaInfo, setSchemaInfo] = useState<SchemaInfo>({
    fields: [],
    titleCanView: true,
    titleCanEdit: true,
    childrenSort: 'none',
    allowedParentTypes: [],
    allowedChildrenTypes: [],
    hasViewHook: false,
    hasHoverHook: false,
    allowAttachments: false,
    attachmentTypes: [],
    fieldGroups: [],
  });
  const [customViewHtml, setCustomViewHtml] = useState<string | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [editedTitle, setEditedTitle] = useState('');
  const [editedFields, setEditedFields] = useState<Record<string, FieldValue>>({});
  const [isDirty, setIsDirty] = useState(false);
  const [editedTags, setEditedTags] = useState<string[]>([]);
  const [allTags, setAllTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [tagSuggestions, setTagSuggestions] = useState<string[]>([]);
  const [recentlyDeleted, setRecentlyDeleted] = useState<AttachmentMeta[]>([]);
  const [groupCollapsed, setGroupCollapsed] = useState<Record<string, boolean>>({});
  const [groupVisible, setGroupVisible] = useState<Record<string, boolean>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [noteErrors, setNoteErrors] = useState<string[]>([]);
  const titleInputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const viewHtmlRef = useRef<HTMLDivElement>(null);
  const pendingEditModeRef = useRef(false);
  // Tracks whether the schema fetch for the current note has already resolved.
  // Used by the requestEditMode effect to enter edit mode immediately when the
  // schema is already available, rather than waiting for a .then() that already ran.
  const schemaLoadedRef = useRef(false);

  const emptySchemaInfo: SchemaInfo = {
    fields: [], titleCanView: true, titleCanEdit: true, childrenSort: 'none',
    allowedParentTypes: [], allowedChildrenTypes: [], hasViewHook: false, hasHoverHook: false,
    allowAttachments: false, attachmentTypes: [], fieldGroups: [],
  };

  useEffect(() => {
    schemaLoadedRef.current = false;
    if (!selectedNote) {
      setSchemaInfo(emptySchemaInfo);
      setCustomViewHtml(null);
      setIsEditing(false);
      pendingEditModeRef.current = false;
      return;
    }

    invoke<SchemaInfo>('get_schema_fields', { nodeType: selectedNote.nodeType })
      .then(info => {
        setSchemaInfo(info);
        setEditedFields(prev => {
          const merged = { ...prev };
          for (const field of info.fields) {
            if (!(field.name in merged)) {
              merged[field.name] = defaultValueForFieldType(field.fieldType);
            }
          }
          return merged;
        });
        schemaLoadedRef.current = true;
        if (pendingEditModeRef.current) {
          setIsEditing(true);
          pendingEditModeRef.current = false;
        }
        // Always fetch the view HTML; the backend generates a default view
        // for notes without an on_view hook (textarea fields render as markdown).
        invoke<string>('get_note_view', { noteId: selectedNote.id })
          .then(html => setCustomViewHtml(html))
          .catch(err => { alert(String(err)); setCustomViewHtml(null); });
      })
      .catch(err => {
        console.error('Failed to fetch schema fields:', err);
        setSchemaInfo(emptySchemaInfo);
        setCustomViewHtml(null);
        schemaLoadedRef.current = true;
        if (pendingEditModeRef.current) {
          setIsEditing(true);
          pendingEditModeRef.current = false;
        }
      });
  }, [selectedNote?.id]);

  useEffect(() => {
    if (selectedNote) {
      setIsEditing(false);
      setEditedTitle(selectedNote.title);
      setEditedFields({ ...selectedNote.fields });
      setEditedTags(selectedNote.tags ?? []);
      setTagInput('');
      setTagSuggestions([]);
      setIsDirty(false);
      setGroupCollapsed({});
      setGroupVisible({});
      setFieldErrors({});
      setNoteErrors([]);
    }
  }, [selectedNote?.id]);

  // Enter edit mode when WorkspaceView requests it (e.g. via context menu, note creation).
  // NOTE: This effect must be declared AFTER the selectedNote?.id effects above.
  // Two cases are handled to avoid a race between the schema IPC fetch and the
  // requestEditMode increment:
  // - Schema already loaded: enter edit mode immediately (schemaLoadedRef is true).
  // - Schema still loading: set pendingEditModeRef so the schema .then() picks it up.
  // This prevents both the "title flash" (entering edit mode before titleCanEdit arrives)
  // and the inverse race where the schema resolves before requestEditMode fires.
  useEffect(() => {
    if (requestEditMode > 0 && selectedNote) {
      if (schemaLoadedRef.current) {
        setIsEditing(true);
      } else {
        pendingEditModeRef.current = true;
      }
    }
  }, [requestEditMode]);

  // Hydrate img[data-kn-attach-id] placeholders with real base64 data after the view HTML renders
  useEffect(() => {
    const container = viewHtmlRef.current;
    if (!container || !customViewHtml) return;

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
          span.textContent = 'Image not found';
          img.replaceWith(span);
        }
      })
    ).catch(err => console.error('Image hydration error:', err));
  }, [customViewHtml]);

  // Hydrate [data-kn-embed-type] sentinels into click-to-play media cards
  useEffect(() => {
    const container = viewHtmlRef.current;
    if (!container || !customViewHtml) return;

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
        label.textContent = 'Open on Instagram ↗';
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
  }, [customViewHtml]);

  // Focus first editable field whenever edit mode activates
  useEffect(() => {
    if (!isEditing) return;
    const rafId = requestAnimationFrame(() => {
      if (titleInputRef.current) {
        titleInputRef.current.focus();
      } else {
        panelRef.current?.querySelector<HTMLElement>('input, textarea, select')?.focus();
      }
    });
    return () => cancelAnimationFrame(rafId);
  }, [isEditing]);

  const handleFormKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!isEditing) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      handleCancel();
    } else if (e.key === 'Enter' && !(e.target instanceof HTMLTextAreaElement)) {
      e.preventDefault();
      handleSave();
    }
  };

  const handleEdit = () => {
    // No need to clear customViewHtml — the HTML panel is hidden in edit mode
    // by the !isEditing condition, so the old HTML stays ready for when the
    // user cancels without saving.
    invoke<string[]>('get_all_tags').then(setAllTags).catch(console.error);
    setIsEditing(true);
  };

  function addTag(tag: string) {
    const normalised = tag.trim().toLowerCase();
    if (!normalised || editedTags.includes(normalised)) return;
    setEditedTags(prev => [...prev, normalised].sort());
    setTagInput('');
    setTagSuggestions([]);
    setIsDirty(true);
  }

  function removeTag(tag: string) {
    setEditedTags(prev => prev.filter(t => t !== tag));
    setIsDirty(true);
  }

  function handleTagInputChange(value: string) {
    setTagInput(value);
    if (!value.trim()) {
      setTagSuggestions([]);
      return;
    }
    const lower = value.trim().toLowerCase();
    setTagSuggestions(
      allTags.filter(t => t.includes(lower) && !editedTags.includes(t)).slice(0, 8)
    );
  }

  const evaluateGroupVisibility = async (fields: Record<string, FieldValue>) => {
    if (!selectedNote || !schemaInfo.fieldGroups.some(g => g.hasVisibleClosure)) return;
    try {
      const vis = await invoke<Record<string, boolean>>('evaluate_group_visibility', {
        noteId: selectedNote.id,
        fields,
      });
      setGroupVisible(vis);
    } catch {
      // ignore — groups default to visible
    }
  };

  const handleFieldBlur = async (fieldName: string, fieldDef: FieldDefinition) => {
    if (!selectedNote || !fieldDef.hasValidate) return;
    try {
      const error = await invoke<string | null>('validate_field', {
        noteId: selectedNote.id,
        fieldName,
        value: editedFields[fieldName] ?? defaultValueForFieldType(fieldDef.fieldType),
      });
      setFieldErrors(prev => {
        const next = { ...prev };
        if (error) { next[fieldName] = error; } else { delete next[fieldName]; }
        return next;
      });
    } catch {
      // ignore validation errors silently
    }
  };

  const handleCancel = async () => {
    if (isDirty) {
      if (!await confirm(t('notes.discardChanges'))) {
        return;
      }
    }
    setIsEditing(false);
    setEditedTitle(selectedNote!.title);
    setEditedFields({ ...selectedNote!.fields });
    setEditedTags(selectedNote!.tags ?? []);
    setTagInput('');
    setTagSuggestions([]);
    setIsDirty(false);
    setFieldErrors({});
    setNoteErrors([]);
    onEditDone();
  };

  const handleSave = async () => {
    if (!selectedNote) return;
    setFieldErrors({});
    setNoteErrors([]);

    try {
      const result = await invoke<SaveResult>('save_note', {
        noteId: selectedNote.id,
        title: editedTitle,
        fields: editedFields,
      });

      if ('validationErrors' in result) {
        setFieldErrors(result.validationErrors.fieldErrors);
        setNoteErrors(result.validationErrors.noteErrors);
        if (result.validationErrors.previewTitle !== null) {
          setEditedTitle(result.validationErrors.previewTitle);
        }
        setEditedFields(prev => ({ ...prev, ...result.validationErrors.previewFields }));
        return;
      }

      const updatedNote = result.ok;
      await invoke('update_note_tags', { noteId: selectedNote.id, tags: editedTags });
      setEditedTitle(updatedNote.title);
      setEditedFields({ ...updatedNote.fields });
      setEditedTags(editedTags);
      setIsEditing(false);
      setIsDirty(false);
      onNoteUpdated();
      onEditDone();
      // Re-fetch view HTML after save — on_save may have changed field values.
      invoke<string>('get_note_view', { noteId: selectedNote.id })
        .then(html => setCustomViewHtml(html))
        .catch(err => { alert(String(err)); setCustomViewHtml(null); });
    } catch (err) {
      alert(t('notes.saveFailed', { error: String(err) }));
    }
  };

  const handleFieldChange = (fieldName: string, value: FieldValue) => {
    setEditedFields(prev => {
      const next = { ...prev, [fieldName]: value };
      evaluateGroupVisibility(next);
      return next;
    });
    setIsDirty(true);
  };

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
              <button
                onClick={handleEdit}
                className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90"
              >
                {t('common.edit')}
              </button>
              <button
                onClick={() => onDeleteRequest(selectedNote.id)}
                className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600"
              >
                {t('common.delete')}
              </button>
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

      {/* Fields Section */}
      <div className="mb-6">
        {/* Custom view rendered by an on_view hook — shown only in view mode */}
        {!isEditing && customViewHtml && (
          <div
            ref={viewHtmlRef}
            dangerouslySetInnerHTML={{ __html: DOMPurify.sanitize(customViewHtml, { ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id', 'data-kn-embed-type', 'data-kn-embed-id', 'data-kn-embed-url'], ALLOWED_URI_REGEXP: /^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp):|data:image\/|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i }) }}
            onClick={(e) => {
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
                  .catch(err => alert(String(err)));
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
        {(isEditing || !customViewHtml) && <h2 className="text-xl font-semibold mb-4">{t('notes.fields')}</h2>}

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
                targetType={field.targetType}
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
                    {!isVisible && <span className="text-xs text-muted-foreground ml-2 font-normal">(hidden — data exists)</span>}
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
                          targetType={field.targetType}
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
        ) : (!customViewHtml && (() => {
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

        {(!customViewHtml || isEditing) && legacyFieldNames.length > 0 && (() => {
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

        {!isEditing && !customViewHtml &&
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

      {/* Metadata Section */}
      <details className="bg-secondary rounded-lg">
        <summary className="px-6 py-4 cursor-pointer list-none flex items-center gap-2 text-sm font-medium text-muted-foreground select-none">
          <ChevronRight size={16} className="[details[open]_&]:rotate-90 transition-transform" />
          {t('notes.info')}
        </summary>
        <div className="px-6 pb-6 space-y-4">
          <div>
            <p className="text-sm text-muted-foreground">{t('notes.type')}</p>
            <p className="text-lg">{selectedNote.nodeType}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">{t('notes.created')}</p>
            <p className="text-sm">{formatTimestamp(selectedNote.createdAt)}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">{t('notes.modified')}</p>
            <p className="text-sm">{formatTimestamp(selectedNote.modifiedAt)}</p>
          </div>
          <div>
            <p className="text-sm text-muted-foreground">{t('notes.id')}</p>
            <p className="text-xs font-mono">{selectedNote.id}</p>
          </div>
        </div>
      </details>
    </div>
  );
}

// Prevent re-renders when WorkspaceView re-renders due to hover/drag/dialog state changes that
// only produce new callback references. Without this guard, React re-processes the view HTML
// prop on every re-render, which can reset the DOM and wipe the hydrated img.src values set
// by the image hydration effect (which does not re-run because customViewHtml is unchanged).
export default memo(InfoPanel, (prev, next) =>
  prev.selectedNote === next.selectedNote &&
  prev.requestEditMode === next.requestEditMode &&
  prev.backNoteTitle === next.backNoteTitle &&
  prev.refreshSignal === next.refreshSignal,
);
