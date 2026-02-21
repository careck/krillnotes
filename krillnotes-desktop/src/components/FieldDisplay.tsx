import { Check, X } from 'lucide-react';
import type { FieldValue, FieldType } from '../types';

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
    return <span className="text-muted-foreground italic">(unknown type)</span>;
  };

  return (
    <>
      <dt className="text-sm font-medium text-muted-foreground self-start pt-0.5 whitespace-nowrap">
        {fieldName}
      </dt>
      <dd className="m-0 text-foreground">
        {renderValue()}
      </dd>
    </>
  );
}

export default FieldDisplay;
