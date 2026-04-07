// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

import { useState, useEffect, type Dispatch, type SetStateAction } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openFilePicker } from '@tauri-apps/plugin-dialog';
import { confirm } from '@tauri-apps/plugin-dialog';
import { Paperclip, Trash2, FileText, Image } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { AttachmentMeta } from '../types';

function mimeToExtension(mime: string): string {
  const sub = mime.split('/')[1] ?? mime;
  const clean = sub.split(';')[0].trim();
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

interface AttachmentsSectionProps {
  noteId: string | null;
  allowedTypes: string[];   // MIME types; empty = all allowed
  refreshSignal?: number;
  recentlyDeleted: AttachmentMeta[];
  onRecentlyDeletedChange: Dispatch<SetStateAction<AttachmentMeta[]>>;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isImageMime(mime: string | null): boolean {
  return mime?.startsWith('image/') ?? false;
}

export default function AttachmentsSection({ noteId, allowedTypes, refreshSignal, recentlyDeleted, onRecentlyDeletedChange }: AttachmentsSectionProps) {
  const { t } = useTranslation();
  const [attachments, setAttachments] = useState<AttachmentMeta[]>([]);
  const [thumbnails, setThumbnails] = useState<Record<string, string>>({});
  const [error, setError] = useState('');
  const [dragging, setDragging] = useState(false);

  const loadAttachments = async () => {
    if (!noteId) { setAttachments([]); return; }
    try {
      const list = await invoke<AttachmentMeta[]>('get_attachments', { noteId });
      setAttachments(list);
      for (const att of list) {
        if (isImageMime(att.mimeType) && !thumbnails[att.id]) {
          invoke<{ data: string; mime_type: string | null }>('get_attachment_data', { attachmentId: att.id })
            .then(result => {
              setThumbnails(prev => ({ ...prev, [att.id]: `data:${att.mimeType};base64,${result.data}` }));
            })
            .catch(() => {});
        }
      }
    } catch (e) {
      setError(`${e}`);
    }
  };

  useEffect(() => {
    loadAttachments();
    setThumbnails({});
    setError('');
  }, [noteId, refreshSignal]);

  const handleDragOver = (e: React.DragEvent) => { e.preventDefault(); setDragging(true); };
  const handleDragLeave = () => setDragging(false);
  // dragDropEnabled is false in tauri.conf.json so DOM events reach us.
  // e.preventDefault() on drop stops WKWebView from opening/navigating to the file.
  // We read bytes via arrayBuffer() — no filesystem path needed.
  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (!noteId) return;
    const files = Array.from(e.dataTransfer.files);
    for (const file of files) {
      if (allowedTypes.length > 0 && !allowedTypes.includes(file.type)) {
        setError(t('attachments.fileTypeNotAllowed', { type: file.type || file.name }));
        continue;
      }
      try {
        const buffer = await file.arrayBuffer();
        // Encode filename as base64(UTF-8 bytes) — http headers are ASCII-only.
        const nameBytes = new TextEncoder().encode(file.name);
        let nameBinary = '';
        for (const b of nameBytes) nameBinary += String.fromCharCode(b);
        const filenameB64 = btoa(nameBinary);
        await invoke('attach_file_bytes', new Uint8Array(buffer), {
          headers: { 'x-note-id': noteId, 'x-filename': filenameB64 },
        });
      } catch (err) {
        setError(t('attachments.failedAttach', { name: file.name, error: String(err) }));
      }
    }
    await loadAttachments();
  };

  const handleAdd = async () => {
    if (!noteId) return;
    setError('');
    try {
      const filters = allowedTypes.length > 0
        ? [{ name: 'Allowed files', extensions: allowedTypes.flatMap(m => {
            const ext = mimeToExtension(m);
            return ext === 'jpeg' ? ['jpeg', 'jpg'] : [ext];
          }) }]
        : [];
      const selected = await openFilePicker({ multiple: true, filters });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      for (const filePath of paths) {
        await invoke('attach_file', { noteId, filePath });
      }
      await loadAttachments();
    } catch (e) {
      setError(t('attachments.failedAttachFile', { error: String(e) }));
    }
  };

  const handleOpen = async (att: AttachmentMeta) => {
    try {
      await invoke('open_attachment', { attachmentId: att.id, filename: att.filename });
    } catch (e) {
      setError(t('attachments.failedOpen', { error: String(e) }));
    }
  };

  const handleDelete = async (att: AttachmentMeta) => {
    const ok = await confirm(t('attachments.deleteConfirm', { name: att.filename }), { title: t('attachments.deleteTitle') });
    if (!ok) return;
    try {
      await invoke('delete_attachment', { attachmentId: att.id });
      setAttachments(prev => prev.filter(a => a.id !== att.id));
      setThumbnails(prev => { const copy = { ...prev }; delete copy[att.id]; return copy; });
      onRecentlyDeletedChange(prev => [...prev, att]);
    } catch (e) {
      setError(t('attachments.failedDelete', { error: String(e) }));
    }
  };

  const handleRestore = async (att: AttachmentMeta) => {
    try {
      await invoke('restore_attachment', { meta: att });
      onRecentlyDeletedChange(prev => prev.filter(a => a.id !== att.id));
      await loadAttachments();
    } catch (e) {
      setError(t('attachments.failedRestore', { error: String(e) }));
    }
  };

  if (!noteId) return null;

  return (
    <div
      className={`border-t border-border pt-3 mt-3 ${dragging ? 'ring-2 ring-primary rounded' : ''}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs font-semibold text-muted-foreground uppercase tracking-wide flex items-center gap-1">
          <Paperclip size={12} /> {t('attachments.title')} {attachments.length > 0 && `(${attachments.length})`}
        </span>
        <button
          onClick={handleAdd}
          className="text-xs text-primary hover:text-primary/80 px-2 py-1 rounded hover:bg-secondary"
        >
          {t('common.add')}
        </button>
      </div>

      {error && (
        <p className="text-xs text-red-500 mb-2">{error}</p>
      )}

      {attachments.length === 0 ? (
        <p className="text-xs text-muted-foreground italic">
          {dragging ? t('attachments.dropHere') : t('attachments.noAttachments')}
        </p>
      ) : (
        <div className="space-y-1">
          {attachments.map(att => (
            <div
              key={att.id}
              className="flex items-center gap-2 group rounded p-1 hover:bg-secondary/50"
            >
              {isImageMime(att.mimeType) && thumbnails[att.id] ? (
                <img
                  src={thumbnails[att.id]}
                  alt={att.filename}
                  className="w-10 h-10 object-cover rounded flex-shrink-0 cursor-pointer"
                  onClick={() => handleOpen(att)}
                />
              ) : (
                <div
                  className="w-10 h-10 rounded flex-shrink-0 bg-secondary flex items-center justify-center cursor-pointer"
                  onClick={() => handleOpen(att)}
                >
                  {isImageMime(att.mimeType)
                    ? <Image size={18} className="text-muted-foreground" />
                    : <FileText size={18} className="text-muted-foreground" />
                  }
                </div>
              )}
              <div className="flex-1 min-w-0 cursor-pointer" onClick={() => handleOpen(att)}>
                <p className="text-xs font-medium truncate">{att.filename}</p>
                <p className="text-xs text-muted-foreground">{formatBytes(att.sizeBytes)}</p>
              </div>
              <button
                onClick={() => handleDelete(att)}
                className="opacity-0 group-hover:opacity-100 p-1 rounded hover:bg-red-500/20 hover:text-red-500 flex-shrink-0"
                title={t('attachments.deleteTitle')}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}

      {recentlyDeleted.filter(a => a.noteId === noteId).length > 0 && (
        <div className="mt-2 pt-2 border-t border-dashed border-border">
          <p className="text-xs text-muted-foreground mb-1">{t('attachments.recentlyDeleted')}</p>
          <div className="space-y-1">
            {recentlyDeleted.filter(a => a.noteId === noteId).map(att => (
              <div key={att.id} className="flex items-center gap-2 rounded p-1 opacity-60">
                <div className="w-10 h-10 rounded flex-shrink-0 bg-secondary flex items-center justify-center">
                  {isImageMime(att.mimeType)
                    ? <Image size={18} className="text-muted-foreground" />
                    : <FileText size={18} className="text-muted-foreground" />
                  }
                </div>
                <div className="flex-1 min-w-0">
                  <p className="text-xs font-medium truncate line-through text-muted-foreground">{att.filename}</p>
                </div>
                <button
                  onClick={() => handleRestore(att)}
                  className="text-xs text-primary hover:text-primary/80 px-2 py-1 rounded hover:bg-secondary flex-shrink-0"
                >
                  {t('attachments.restore')}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
