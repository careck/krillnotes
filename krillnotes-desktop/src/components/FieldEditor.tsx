import type { FieldValue } from '../types';

interface FieldEditorProps {
  fieldName: string;
  fieldType: string;
  value: FieldValue;
  required: boolean;
  onChange: (value: FieldValue) => void;
}

function FieldEditor({ fieldName, fieldType: _fieldType, value, required, onChange }: FieldEditorProps) {
  const renderEditor = () => {
    if ('Text' in value) {
      return (
        <textarea
          value={value.Text}
          onChange={(e) => onChange({ Text: e.target.value })}
          className="w-full p-2 bg-background border border-border rounded-md min-h-[100px] resize-y"
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
