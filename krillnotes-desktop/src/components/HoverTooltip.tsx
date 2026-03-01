import { Fragment, useRef, useLayoutEffect, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import DOMPurify from 'dompurify';
import type { Note, SchemaInfo, FieldValue } from '../types';

interface HoverTooltipProps {
  note: Note;
  schema: SchemaInfo | null;
  hoverHtml: string | null;       // from on_hover hook; null = use field flags
  anchorY: number;                // viewport Y of hovered row center
  treeWidth: number;              // right edge of tree panel
  visible: boolean;
}

function renderFieldValue(value: FieldValue | undefined, linkedNoteLabel: string): string {
  if (!value) return '\u2014';
  if ('Text' in value) return value.Text || '\u2014';
  if ('Number' in value) return String(value.Number);
  if ('Boolean' in value) return value.Boolean ? 'Yes' : 'No';
  if ('Date' in value) return value.Date ?? '\u2014';
  if ('Email' in value) return value.Email || '\u2014';
  if ('NoteLink' in value) return value.NoteLink ? linkedNoteLabel : '\u2014';
  return '\u2014';
}

export default function HoverTooltip({
  note, schema, hoverHtml, anchorY, treeWidth, visible,
}: HoverTooltipProps) {
  const { t } = useTranslation();
  const tooltipRef = useRef<HTMLDivElement>(null);
  const [computedTop, setComputedTop] = useState(anchorY);
  const [spikeOffset, setSpikeOffset] = useState('50%');

  const left = treeWidth + 14;

  useLayoutEffect(() => {
    if (!visible || !tooltipRef.current) return;
    const h = tooltipRef.current.offsetHeight;
    const rawTop = anchorY - h / 2;
    const clampedTop = Math.max(8, Math.min(rawTop, window.innerHeight - h - 8));
    setComputedTop(clampedTop);
    setSpikeOffset(`${anchorY - clampedTop}px`);
  }, [anchorY, treeWidth, visible, hoverHtml]);

  // Hydrate img[data-kn-attach-id] sentinels inside the tooltip after it renders.
  useEffect(() => {
    const container = tooltipRef.current;
    if (!visible || !container || !hoverHtml) return;
    const imgs = Array.from(container.querySelectorAll<HTMLImageElement>('img[data-kn-attach-id]'));
    imgs.forEach(async (img) => {
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
    });
  }, [visible, hoverHtml]);

  if (!visible) return null;

  // hoverHtml is already sanitized by DOMPurify below before being set as innerHTML.
  // This is the same pattern used in InfoPanel for on_view hook output.
  const sanitizedHtml = hoverHtml !== null
    ? DOMPurify.sanitize(hoverHtml, { ADD_ATTR: ['data-note-id', 'data-kn-attach-id', 'data-kn-width', 'data-kn-download-id'], ALLOWED_URI_REGEXP: /^(?:(?:(?:f|ht)tps?|mailto|tel|callto|sms|cid|xmpp):|data:image\/|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i })
    : null;

  return createPortal(
    <div
      ref={tooltipRef}
      className="kn-hover-tooltip"
      style={{ top: computedTop, left, ['--spike-offset' as string]: spikeOffset } as React.CSSProperties}
    >
      {sanitizedHtml !== null ? (
        // eslint-disable-next-line react/no-danger
        <div dangerouslySetInnerHTML={{ __html: sanitizedHtml }} />
      ) : (
        <div className="kn-hover-tooltip__row">
          {schema?.fields
            .filter(f => f.showOnHover)
            .map(f => (
              <Fragment key={f.name}>
                <span className="kn-hover-tooltip__label">{f.name}</span>
                <span className="kn-hover-tooltip__value">
                  {renderFieldValue(note.fields[f.name], t('fields.linkedNote'))}
                </span>
              </Fragment>
            ))}
        </div>
      )}
    </div>,
    document.body,
  );
}
