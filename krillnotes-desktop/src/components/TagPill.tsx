import { useTranslation } from 'react-i18next';
import { tagColor } from '../utils/tagColor';

interface TagPillProps {
  tag: string;
  onRemove?: () => void;   // if provided, shows × button (edit mode)
  onClick?: () => void;    // if provided, pill is clickable (tag cloud)
}

export default function TagPill({ tag, onRemove, onClick }: TagPillProps) {
  const { t } = useTranslation();
  return (
    <span
      className={`kn-tag-pill${onClick ? ' kn-tag-pill--clickable' : ''}`}
      style={{ backgroundColor: tagColor(tag) }}
      onClick={onClick}
      title={tag}
    >
      {tag}
      {onRemove && (
        <button
          className="kn-tag-pill__remove"
          onClick={e => { e.stopPropagation(); onRemove(); }}
          aria-label={t('tags.remove', { tag })}
        >
          ×
        </button>
      )}
    </span>
  );
}
