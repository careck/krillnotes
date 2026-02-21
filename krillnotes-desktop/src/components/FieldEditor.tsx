import type { FieldValue, FieldType } from '../types';

interface FieldEditorProps {
  fieldName: string;
  fieldType: FieldType;
  value: FieldValue;
  required: boolean;
  onChange: (value: FieldValue) => void;
}

function FieldEditor({ fieldName, fieldType, value, required, onChange }: FieldEditorProps) {
  const renderEditor = () => {
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
      return (
        <input
          type="text"
          value={value.Text}
          onChange={(e) => onChange({ Text: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md"
          required={required}
        />
      );
    } else if ('Number' in value) {
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
    return <span className="text-red-500">Unknown field type</span>;
  };

  return (
    <div className="mb-4">
      <label className="block text-sm font-medium mb-1">
        {fieldName}
        {required && <span className="text-red-500 ml-1">*</span>}
      </label>
      {renderEditor()}
    </div>
  );
}

export default FieldEditor;
