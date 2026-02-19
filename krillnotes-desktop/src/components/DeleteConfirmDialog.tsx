import { useState, useEffect } from 'react';
import { DeleteStrategy } from '../types';

interface DeleteConfirmDialogProps {
  noteTitle: string;
  childCount: number;
  onConfirm: (strategy: DeleteStrategy) => void;
  onCancel: () => void;
  disabled?: boolean;
}

function DeleteConfirmDialog({ noteTitle, childCount, onConfirm, onCancel, disabled = false }: DeleteConfirmDialogProps) {
  const [strategy, setStrategy] = useState<DeleteStrategy>(DeleteStrategy.DeleteAll);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onCancel();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onCancel]);

  const handleConfirm = () => {
    onConfirm(strategy);
  };

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background border border-border rounded-lg p-6 max-w-md w-full">
        <h2 className="text-xl font-bold mb-4">Delete Note</h2>

        {childCount === 0 ? (
          <p className="mb-6">
            Are you sure you want to delete <strong>{noteTitle}</strong>?
          </p>
        ) : (
          <>
            <p className="mb-4">
              Delete <strong>{noteTitle}</strong>? This note has <strong>{childCount}</strong> {childCount === 1 ? 'child' : 'children'}.
            </p>
            <div className="space-y-3 mb-6">
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="deleteStrategy"
                  checked={strategy === DeleteStrategy.DeleteAll}
                  onChange={() => setStrategy(DeleteStrategy.DeleteAll)}
                  className="mt-1"
                />
                <div>
                  <div className="font-medium">Delete this note and all descendants</div>
                </div>
              </label>
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="radio"
                  name="deleteStrategy"
                  checked={strategy === DeleteStrategy.PromoteChildren}
                  onChange={() => setStrategy(DeleteStrategy.PromoteChildren)}
                  className="mt-1"
                />
                <div>
                  <div className="font-medium">Delete this note and promote children</div>
                  <div className="text-sm text-muted-foreground">
                    Children will be moved to parent level
                  </div>
                </div>
              </label>
            </div>
          </>
        )}

        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            disabled={disabled}
            className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={disabled}
            className="px-4 py-2 bg-red-500 text-white rounded-md hover:bg-red-600 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  );
}

export default DeleteConfirmDialog;
