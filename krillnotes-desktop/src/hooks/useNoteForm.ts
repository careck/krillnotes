// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { confirm } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import type { Note, FieldValue, SchemaInfo, FieldDefinition, SaveResult } from '../types';
import { defaultValueForFieldType } from '../utils/fieldValue';

export function useNoteForm(
  selectedNote: Note | null,
  schemaInfo: SchemaInfo,
  schemaCallbacks: {
    activeTab: string;
    setActiveTab: (tab: string) => void;
    previousTab: string | null;
    setPreviousTab: (tab: string | null) => void;
    setViewHtml: React.Dispatch<React.SetStateAction<Record<string, string>>>;
  },
  onNoteUpdated: () => void,
  onEditDone: () => void,
  // isEditing / setIsEditing are lifted to InfoPanel to break the circular dependency:
  // useSchema needs isEditing as a parameter, and useNoteForm needs schemaInfo from useSchema.
  isEditingExternal: boolean,
  setIsEditingExternal: React.Dispatch<React.SetStateAction<boolean>>,
) {
  const { t } = useTranslation();
  const { activeTab, setActiveTab, previousTab, setPreviousTab, setViewHtml } = schemaCallbacks;

  const isEditing = isEditingExternal;
  const setIsEditing = setIsEditingExternal;
  const [editedTitle, setEditedTitle] = useState('');
  const [editedFields, setEditedFields] = useState<Record<string, FieldValue>>({});
  const [isDirty, setIsDirty] = useState(false);
  const [editedTags, setEditedTags] = useState<string[]>([]);
  const [allTags, setAllTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [tagSuggestions, setTagSuggestions] = useState<string[]>([]);
  const [groupCollapsed, setGroupCollapsed] = useState<Record<string, boolean>>({});
  const [groupVisible, setGroupVisible] = useState<Record<string, boolean>>({});
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [noteErrors, setNoteErrors] = useState<string[]>([]);

  const titleInputRef = useRef<HTMLInputElement>(null);

  // Effect 2: Reset form state when selected note changes
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
    } else {
      setIsEditing(false);
    }
  }, [selectedNote?.id]);

  // Effect 7: Auto-focus title input when edit mode activates
  useEffect(() => {
    if (!isEditing) return;
    const rafId = requestAnimationFrame(() => {
      titleInputRef.current?.focus();
    });
    return () => cancelAnimationFrame(rafId);
  }, [isEditing]);

  const evaluateGroupVisibility = useCallback(async (fields: Record<string, FieldValue>) => {
    if (!selectedNote || !schemaInfo.fieldGroups.some(g => g.hasVisibleClosure)) return;
    try {
      const vis = await invoke<Record<string, boolean>>('evaluate_group_visibility', {
        schemaName: selectedNote.schema,
        fields,
      });
      setGroupVisible(vis);
    } catch {
      // ignore — groups default to visible
    }
  }, [selectedNote, schemaInfo.fieldGroups]);

  const handleEdit = useCallback(() => {
    invoke<string[]>('get_all_tags').then(setAllTags).catch(console.error);
    setPreviousTab(activeTab);
    setActiveTab('fields');
    setIsEditing(true);
  }, [activeTab, setPreviousTab, setActiveTab, setIsEditing]);

  const addTag = useCallback((tag: string) => {
    const normalised = tag.trim().toLowerCase();
    if (!normalised || editedTags.includes(normalised)) return;
    setEditedTags(prev => [...prev, normalised].sort());
    setTagInput('');
    setTagSuggestions([]);
    setIsDirty(true);
  }, [editedTags, setEditedTags, setTagInput, setTagSuggestions]);

  const removeTag = useCallback((tag: string) => {
    setEditedTags(prev => prev.filter(t => t !== tag));
    setIsDirty(true);
  }, [setEditedTags]);

  const handleTagInputChange = useCallback((value: string) => {
    setTagInput(value);
    if (!value.trim()) {
      setTagSuggestions([]);
      return;
    }
    const lower = value.trim().toLowerCase();
    setTagSuggestions(
      allTags.filter(t => t.includes(lower) && !editedTags.includes(t)).slice(0, 8)
    );
  }, [allTags, editedTags, setTagSuggestions, setTagInput]);

  const handleFieldBlur = useCallback(async (fieldName: string, fieldDef: FieldDefinition) => {
    if (!selectedNote || !fieldDef.hasValidate) return;
    try {
      const error = await invoke<string | null>('validate_field', {
        schemaName: selectedNote.schema,
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
  }, [selectedNote, editedFields, setFieldErrors]);

  const handleCancel = useCallback(async () => {
    if (isDirty) {
      if (!await confirm(t('notes.discardChanges'))) {
        return;
      }
    }
    // selectedNote may have changed while the confirm dialog was open
    if (!selectedNote) return;
    setIsEditing(false);
    if (previousTab) {
      setActiveTab(previousTab);
      setPreviousTab(null);
    }
    setEditedTitle(selectedNote.title);
    setEditedFields({ ...selectedNote.fields });
    setEditedTags(selectedNote.tags ?? []);
    setTagInput('');
    setTagSuggestions([]);
    setIsDirty(false);
    setFieldErrors({});
    setNoteErrors([]);
    onEditDone();
  }, [isDirty, selectedNote, previousTab, setIsEditing, setActiveTab, setPreviousTab,
      setEditedTitle, setEditedFields, setEditedTags, setTagInput, setTagSuggestions,
      setIsDirty, setFieldErrors, setNoteErrors, onEditDone, t]);

  const handleSave = useCallback(async () => {
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
      // Restore the tab that was active before editing
      const restoreTab = previousTab;
      if (restoreTab) {
        setActiveTab(restoreTab);
        setPreviousTab(null);
      }
      onNoteUpdated();
      onEditDone();
      // Re-fetch view HTML after save — on_save may have changed field values.
      // Clear cached HTML so the render_view effect re-fetches.
      setViewHtml({});
    } catch (err) {
      alert(t('notes.saveFailed', { error: String(err) }));
    }
  }, [selectedNote, editedTitle, editedFields, editedTags, previousTab,
      setActiveTab, setPreviousTab, setIsEditing, setIsDirty, setViewHtml,
      setEditedTitle, setEditedFields, setEditedTags, setFieldErrors, setNoteErrors,
      onNoteUpdated, onEditDone, t]);

  // handleFormKeyDown depends on handleCancel and handleSave — defined after them
  const handleFormKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!isEditing) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      handleCancel();
    } else if (e.key === 'Enter' && !(e.target instanceof HTMLTextAreaElement)) {
      e.preventDefault();
      handleSave();
    }
  }, [isEditing, handleCancel, handleSave]);

  const handleFieldChange = useCallback((fieldName: string, value: FieldValue) => {
    setEditedFields(prev => {
      const next = { ...prev, [fieldName]: value };
      evaluateGroupVisibility(next);
      return next;
    });
    setIsDirty(true);
  }, [setEditedFields, evaluateGroupVisibility]);

  return {
    isEditing,
    setIsEditing,
    editedTitle,
    setEditedTitle,
    editedFields,
    setEditedFields,
    isDirty,
    setIsDirty,
    editedTags,
    setEditedTags,
    allTags,
    tagInput,
    tagSuggestions,
    groupCollapsed,
    setGroupCollapsed,
    groupVisible,
    fieldErrors,
    noteErrors,
    titleInputRef,
    handleFormKeyDown,
    handleEdit,
    handleCancel,
    handleSave,
    handleFieldChange,
    handleFieldBlur,
    addTag,
    removeTag,
    handleTagInputChange,
  };
}
