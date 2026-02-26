import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Check, X } from 'lucide-react';
import type { FieldValue, FieldType } from '../types';
import { humaniseKey } from '../utils/humanise';

function NoteLinkDisplay({ noteId }: { noteId: string }) {
  const [title, setTitle] = useState<string | null>(null);

  useEffect(() => {
    invoke<{ id: string; title: string }>('get_note', { noteId })
      .then(n => setTitle(n.title))
      .catch(() => setTitle('(deleted)'));
  }, [noteId]);

  if (title === null) return <span>…</span>;

  return (
    <a
      className="kn-view-link text-primary underline cursor-pointer"
      data-note-id={noteId}
    >
      {title}
    </a>
  );
}

interface FieldDisplayProps {
  fieldName: string;
  fieldType: FieldType;
  value: FieldValue;
  max?: number;
}

function FieldDisplay({ fieldName, fieldType, value, max = 5 }: FieldDisplayProps) {
  const renderValue = () => {
    if ('Number' in value && fieldType === 'rating') {
      const starCount = max > 0 ? max : 5;
      const filled = Math.min(Math.round(value.Number), starCount);
      if (filled === 0) return <p className="text-muted-foreground italic">Not rated</p>;
      const stars = '★'.repeat(filled) + '☆'.repeat(Math.max(0, starCount - filled));
      return <p className="text-yellow-400 text-lg leading-none">{stars}</p>;
    }
    if ('Text' in value) {
      return <p className="whitespace-pre-wrap break-words">{value.Text}</p>;
    } else if ('Number' in value) {
      return <p>{value.Number}</p>;
    } else if ('Boolean' in value) {
      return (
        <span className="inline-flex items-center" aria-label={value.Boolean ? 'Yes' : 'No'}>
          {value.Boolean
            ? <Check size={18} className="text-green-500" aria-hidden="true" />
            : <X size={18} className="text-red-500" aria-hidden="true" />}
        </span>
      );
    } else if ('Email' in value) {
      return <a href={`mailto:${value.Email}`} className="text-primary underline">{value.Email}</a>;
    } else if ('Date' in value) {
      if (value.Date === null) return <p className="text-muted-foreground italic">—</p>;
      const formatted = new Date(`${value.Date}T00:00:00`).toLocaleDateString(undefined, {
        year: 'numeric', month: 'long', day: 'numeric',
      });
      return <p>{formatted}</p>;
    }
    if (fieldType === 'note_link') {
      if (!value || !('NoteLink' in value) || (value as { NoteLink: string | null }).NoteLink === null) {
        return <span>—</span>;
      }
      return <NoteLinkDisplay noteId={(value as { NoteLink: string | null }).NoteLink as string} />;
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
  };

  return (
    <>
      <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">
        {humaniseKey(fieldName)}
      </dt>
      <dd className="m-0 text-foreground">
        {renderValue()}
      </dd>
    </>
  );
}

export default FieldDisplay;
