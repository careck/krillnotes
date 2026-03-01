import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Paperclip, X } from 'lucide-react';
import type { AttachmentMeta, FieldValue } from '../types';

interface FileFieldProps {
  attachmentId: string | null;
  allowedTypes: string[];        // MIME types; empty = all
  isEditing: boolean;
  noteId: string;
  onValueChange: (newValue: FieldValue) => void;
}

function isImageMime(mime: string | null | undefined): boolean {
  return mime?.startsWith('image/') ?? false;
}

function mimeToExtension(mime: string): string {
  const sub = mime.split('/')[1] ?? mime;
  // Strip parameters
  const clean = sub.split(';')[0].trim();
  // Known special cases
  const special: Record<string, string> = {
    'svg+xml': 'svg',
    'x-matroska': 'mkv',
    'vnd.openxmlformats-officedocument.wordprocessingml.document': 'docx',
    'vnd.openxmlformats-officedocument.spreadsheetml.sheet': 'xlsx',
    'vnd.openxmlformats-officedocument.presentationml.presentation': 'pptx',
    'x-m4v': 'mp4',
    'quicktime': 'mov',
  };
  return special[clean] ?? clean.replace(/\+.*$/, '').replace(/^x-/, '');
}

export function FileField({
  attachmentId, allowedTypes, isEditing, noteId, onValueChange,
}: FileFieldProps) {
  const [meta, setMeta] = useState<AttachmentMeta | null>(null);
  const [thumbSrc, setThumbSrc] = useState<string | null>(null);

  useEffect(() => {
    if (!attachmentId) { setMeta(null); setThumbSrc(null); return; }
    invoke<AttachmentMeta[]>('get_attachments', { noteId })
      .then(list => {
        const found = list.find(a => a.id === attachmentId) ?? null;
        setMeta(found);
        if (found && isImageMime(found.mimeType)) {
          invoke<string>('get_attachment_data', { attachmentId: found.id })
            .then(b64 => setThumbSrc(`data:${found.mimeType};base64,${b64}`))
            .catch(() => setThumbSrc(null));
        } else {
          setThumbSrc(null);
        }
      })
      .catch(() => { setMeta(null); setThumbSrc(null); });
  }, [attachmentId, noteId]);

  async function handlePick() {
    // Build extension filters from allowedTypes MIME list.
    // e.g. ["image/png", "image/jpeg"] → extensions: ["png", "jpeg"]
    const filters = allowedTypes.length > 0
      ? [{ name: 'Allowed files', extensions: allowedTypes.map(mimeToExtension) }]
      : [];
    const selected = await open({ multiple: false, filters });
    if (!selected || typeof selected !== 'string') return;

    const filePath = selected;

    try {
      const newMeta = await invoke<AttachmentMeta>('attach_file', {
        noteId,
        filePath,
      });
      // Only delete old attachment after new one is safely stored
      if (attachmentId) {
        await invoke('delete_attachment', { attachmentId }).catch(() => {});
      }
      onValueChange({ File: newMeta.id });
    } catch (err) {
      alert(`Failed to attach file: ${String(err)}`);
    }
  }

  async function handleClear() {
    if (attachmentId) {
      await invoke('delete_attachment', { attachmentId }).catch(() => {});
    }
    onValueChange({ File: null });
  }

  // View mode
  if (!isEditing) {
    if (!meta) return <span className="text-muted-foreground text-sm">—</span>;
    return (
      <div className="flex items-center gap-2">
        {thumbSrc
          ? <img src={thumbSrc} alt={meta.filename} className="w-10 h-10 object-cover rounded" />
          : <Paperclip className="w-4 h-4 text-muted-foreground" />}
        <span className="text-sm">{meta.filename}</span>
      </div>
    );
  }

  // Edit mode
  return (
    <div className="flex items-center gap-2 flex-wrap">
      {meta && (
        <div className="flex items-center gap-1 text-sm">
          {thumbSrc && (
            <img src={thumbSrc} alt={meta.filename} className="w-8 h-8 object-cover rounded" />
          )}
          <span>{meta.filename}</span>
          <button
            type="button"
            onClick={handleClear}
            className="text-muted-foreground hover:text-destructive ml-1"
            title="Remove file"
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      )}
      <button
        type="button"
        onClick={handlePick}
        className="text-xs underline text-muted-foreground hover:text-foreground"
      >
        {meta ? 'Replace…' : 'Choose file…'}
      </button>
    </div>
  );
}
