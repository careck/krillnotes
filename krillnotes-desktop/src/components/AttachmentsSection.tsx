import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open as openFilePicker } from '@tauri-apps/plugin-dialog';
import { confirm } from '@tauri-apps/plugin-dialog';
import { Paperclip, Trash2, FileText, Image } from 'lucide-react';
import type { AttachmentMeta } from '../types';

interface AttachmentsSectionProps {
  noteId: string | null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function isImageMime(mime: string | null): boolean {
  return mime?.startsWith('image/') ?? false;
}

export default function AttachmentsSection({ noteId }: AttachmentsSectionProps) {
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
  }, [noteId]);

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
      try {
        const buffer = await file.arrayBuffer();
        const data = Array.from(new Uint8Array(buffer));
        await invoke('attach_file_bytes', { noteId, filename: file.name, data });
      } catch (err) {
        setError(`Failed to attach ${file.name}: ${err}`);
      }
    }
    await loadAttachments();
  };

  const handleAdd = async () => {
    if (!noteId) return;
    setError('');
    try {
      const selected = await openFilePicker({ multiple: true });
      if (!selected) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      for (const filePath of paths) {
        await invoke('attach_file', { noteId, filePath });
      }
      await loadAttachments();
    } catch (e) {
      setError(`Failed to attach: ${e}`);
    }
  };

  const handleOpen = async (att: AttachmentMeta) => {
    try {
      await invoke('open_attachment', { attachmentId: att.id, filename: att.filename });
    } catch (e) {
      setError(`Failed to open: ${e}`);
    }
  };

  const handleDelete = async (att: AttachmentMeta) => {
    const ok = await confirm(`Delete attachment "${att.filename}"?`, { title: 'Delete Attachment' });
    if (!ok) return;
    try {
      await invoke('delete_attachment', { attachmentId: att.id });
      setAttachments(prev => prev.filter(a => a.id !== att.id));
      setThumbnails(prev => { const copy = { ...prev }; delete copy[att.id]; return copy; });
    } catch (e) {
      setError(`Failed to delete: ${e}`);
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
          <Paperclip size={12} /> Attachments {attachments.length > 0 && `(${attachments.length})`}
        </span>
        <button
          onClick={handleAdd}
          className="text-xs text-primary hover:text-primary/80 px-2 py-1 rounded hover:bg-secondary"
        >
          + Add
        </button>
      </div>

      {error && (
        <p className="text-xs text-red-500 mb-2">{error}</p>
      )}

      {attachments.length === 0 ? (
        <p className="text-xs text-muted-foreground italic">
          {dragging ? 'Drop files here' : 'No attachments — drop files or click Add'}
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
                title="Delete attachment"
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
