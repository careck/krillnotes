import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { NoteSearchResult } from '../types';

interface Props {
  value: string | null;
  targetType?: string;
  onChange: (id: string | null) => void;
}

export function NoteLinkEditor({ value, targetType, onChange }: Props) {
  const [displayTitle, setDisplayTitle] = useState<string>('');
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<NoteSearchResult[]>([]);
  const [isOpen, setIsOpen] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Resolve UUID → title on mount and when value changes
  useEffect(() => {
    if (!value) {
      setDisplayTitle('');
      return;
    }
    invoke<{ id: string; title: string }>('get_note', { noteId: value })
      .then(n => setDisplayTitle(n.title))
      .catch(() => setDisplayTitle('(deleted)'));
  }, [value]);

  function handleInput(e: React.ChangeEvent<HTMLInputElement>) {
    const q = e.target.value;
    setQuery(q);
    setIsOpen(true);

    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!q.trim()) {
      setResults([]);
      return;
    }
    debounceRef.current = setTimeout(async () => {
      try {
        const r = await invoke<NoteSearchResult[]>('search_notes', {
          query: q,
          targetType: targetType ?? null,
        });
        setResults(r);
      } catch {
        setResults([]);
      }
    }, 300);
  }

  function handleSelect(result: NoteSearchResult) {
    onChange(result.id);
    setQuery('');
    setResults([]);
    setIsOpen(false);
  }

  function handleClear(e: React.MouseEvent) {
    e.stopPropagation();
    onChange(null);
    setQuery('');
    setResults([]);
    setIsOpen(false);
  }

  const inputValue = isOpen ? query : displayTitle;

  return (
    <div className="relative">
      <div className="flex gap-1">
        <input
          type="text"
          value={inputValue}
          placeholder="Search for a note…"
          onChange={handleInput}
          onFocus={() => { setIsOpen(true); setQuery(''); }}
          onBlur={() => setTimeout(() => setIsOpen(false), 150)}
          className="flex-1 p-2 bg-background border border-border rounded-md"
        />
        {value && (
          <button
            type="button"
            onClick={handleClear}
            title="Clear link"
            className="px-2 py-1 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
          >
            ✕
          </button>
        )}
      </div>
      {isOpen && results.length > 0 && (
        <ul className="absolute top-full left-0 right-0 z-[100] bg-background border border-border rounded-md p-0 m-0 list-none max-h-[200px] overflow-y-auto">
          {results.map(r => (
            <li
              key={r.id}
              onMouseDown={() => handleSelect(r)}
              className="px-2.5 py-1.5 cursor-pointer hover:bg-secondary"
            >
              {r.title}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export default NoteLinkEditor;
