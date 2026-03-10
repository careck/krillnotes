// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useTranslation } from 'react-i18next';
import type { FieldValue, FieldType, FieldDefinition } from '../types';
import { humaniseKey } from '../utils/humanise';
import NoteLinkEditor from './NoteLinkEditor';
import { FileField } from './FileField';

interface FieldEditorProps {
  fieldName: string;
  fieldType: FieldType;
  value: FieldValue;
  required: boolean;
  options: string[];
  max: number;
  targetSchema?: string;
  noteId?: string;
  fieldDef?: FieldDefinition;
  error?: string;
  onBlur?: () => void;
  onChange: (value: FieldValue) => void;
}

function FieldEditor({ fieldName, fieldType, value, required, options, max, targetSchema, noteId, fieldDef, error, onBlur, onChange }: FieldEditorProps) {
  const { t } = useTranslation();
  const renderEditor = () => {
    if (fieldType === 'file') {
      const currentId = value && 'File' in value ? (value as { File: string | null }).File : null;
      return (
        <FileField
          attachmentId={currentId}
          allowedTypes={fieldDef?.allowedTypes ?? []}
          isEditing={true}
          noteId={noteId ?? ''}
          onValueChange={onChange}
        />
      );
    }
    if (fieldType === 'note_link') {
      const currentId = value && 'NoteLink' in value ? (value as { NoteLink: string | null }).NoteLink : null;
      return (
        <NoteLinkEditor
          value={currentId}
          targetSchema={targetSchema}
          onChange={(id) => onChange({ NoteLink: id })}
        />
      );
    }
    if ('Text' in value) {
      if (fieldType === 'textarea') {
        return (
          <textarea
            value={value.Text}
            onChange={(e) => onChange({ Text: e.target.value })}
            className="w-full p-2 bg-background border border-border rounded-md min-h-[100px] resize-y"
            required={required}
          />
        );
      }
      if (fieldType === 'select') {
        if (options.length === 0) {
          return <p className="text-sm text-muted-foreground italic p-2">{t('fields.noOptions')}</p>;
        }
        return (
          <select
            value={value.Text}
            onChange={(e) => onChange({ Text: e.target.value })}
            className="w-full p-2 bg-background border border-border rounded-md"
            required={required}
          >
            <option value="">{t('fields.selectPlaceholder')}</option>
            {options.map(opt => (
              <option key={opt} value={opt}>{opt}</option>
            ))}
          </select>
        );
      }
      return (
        <input
          type="text"
          value={value.Text}
          onChange={(e) => onChange({ Text: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
      );
    } else if ('Number' in value) {
      if (fieldType === 'rating') {
        const current = value.Number;
        const starCount = max > 0 ? max : 5;
        return (
          <div className="flex gap-1">
            {Array.from({ length: starCount }, (_, i) => i + 1).map(star => (
              <button
                key={star}
                type="button"
                onClick={() => onChange({ Number: star === current ? 0 : star })}
                className="text-2xl leading-none text-yellow-400 hover:scale-110 transition-transform"
                aria-label={t('fields.starRating', { count: star })}
                aria-pressed={star <= current}
              >
                {star <= current ? '★' : '☆'}
              </button>
            ))}
          </div>
        );
      }
      return (
        <input
          type="number"
          value={value.Number}
          onChange={(e) => onChange({ Number: parseFloat(e.target.value) || 0 })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Boolean' in value) {
      return (
        <input
          type="checkbox"
          checked={value.Boolean}
          onChange={(e) => onChange({ Boolean: e.target.checked })}
          className="rounded"
        />
      );
    } else if ('Email' in value) {
      return (
        <input
          type="email"
          value={value.Email}
          onChange={(e) => onChange({ Email: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
        />
      );
    } else if ('Date' in value) {
      return (
        <input
          type="date"
          value={value.Date ?? ''}
          onChange={(e) => onChange({ Date: e.target.value || null })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    }
    return <span className="text-red-500">{t('fields.unknownFieldType')}</span>;
  };

  return (
    <div
      className="mb-4"
      onBlur={(e) => {
        if (onBlur && !e.currentTarget.contains(e.relatedTarget as Node)) {
          onBlur();
        }
      }}
    >
      <label className="block text-sm font-medium mb-1">
        {humaniseKey(fieldName)}
        {required && <span className="text-red-500 ml-1">*</span>}
      </label>
      {renderEditor()}
      {error && <p className="text-xs text-red-500 mt-1">{error}</p>}
    </div>
  );
}

export default FieldEditor;
