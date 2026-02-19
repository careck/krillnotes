import type { FieldValue } from '../types';

interface FieldDisplayProps {
  fieldName: string;
  fieldType: string;
  value: FieldValue;
}

function FieldDisplay({ fieldName, fieldType: _fieldType, value }: FieldDisplayProps) {
  const renderValue = () => {
    if ('Text' in value) {
      return (
        <p className="whitespace-pre-wrap break-words">
          {value.Text || <span className="text-muted-foreground italic">(empty)</span>}
        </p>
      );
    } else if ('Number' in value) {
      return <p>{value.Number}</p>;
    } else if ('Boolean' in value) {
      return (
        <div className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={value.Boolean}
            disabled
            className="rounded"
          />
          <span>{value.Boolean ? 'Yes' : 'No'}</span>
        </div>
      );
    }
    return <span className="text-muted-foreground italic">(unknown type)</span>;
  };

  return (
    <div className="mb-4">
      <label className="block text-sm font-medium text-muted-foreground mb-1">
        {fieldName}
      </label>
      <div className="text-foreground">
        {renderValue()}
      </div>
    </div>
  );
}

export default FieldDisplay;
