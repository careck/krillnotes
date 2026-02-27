import { Fragment, useRef, useLayoutEffect, useState } from 'react';
import { createPortal } from 'react-dom';
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

function renderFieldValue(value: FieldValue | undefined): string {
  if (!value) return '\u2014';
  if ('Text' in value) return value.Text || '\u2014';
  if ('Number' in value) return String(value.Number);
  if ('Boolean' in value) return value.Boolean ? 'Yes' : 'No';
  if ('Date' in value) return value.Date ?? '\u2014';
  if ('Email' in value) return value.Email || '\u2014';
  if ('NoteLink' in value) return value.NoteLink ? '(linked note)' : '\u2014';
  return '\u2014';
}

export default function HoverTooltip({
  note, schema, hoverHtml, anchorY, treeWidth, visible,
}: HoverTooltipProps) {
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

  if (!visible) return null;

  // hoverHtml is already sanitized by DOMPurify below before being set as innerHTML.
  // This is the same pattern used in InfoPanel for on_view hook output.
  const sanitizedHtml = hoverHtml !== null
    ? DOMPurify.sanitize(hoverHtml, { ADD_ATTR: ['data-note-id'] })
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
                  {renderFieldValue(note.fields[f.name])}
                </span>
              </Fragment>
            ))}
        </div>
      )}
    </div>,
    document.body,
  );
}
