import { useState, useRef, useEffect, useCallback } from 'react';
import { Search, X } from 'lucide-react';
import { searchNotes } from '../utils/search';
import type { Note } from '../types';
import type { SearchResult } from '../utils/search';

interface SearchBarProps {
  notes: Note[];
  onSelect: (noteId: string) => void;
}

/** Truncates text around the match position to show context. */
function matchSnippet(value: string, query: string, maxLen = 60): string {
  const lower = value.toLowerCase();
  const idx = lower.indexOf(query.toLowerCase());
  if (idx === -1) return value.slice(0, maxLen);
  const start = Math.max(0, idx - 20);
  const end = Math.min(value.length, idx + query.length + 20);
  let snippet = value.slice(start, end);
  if (start > 0) snippet = '\u2026' + snippet;
  if (end < value.length) snippet = snippet + '\u2026';
  return snippet;
}

function SearchBar({ notes, onSelect }: SearchBarProps) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchResult[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [isFocused, setIsFocused] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const dropdownRef = useRef<HTMLDivElement>(null);

  // Debounced search
  useEffect(() => {
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      const found = searchNotes(notes, query);
      setResults(found);
      setActiveIndex(0);
    }, 150);
    return () => clearTimeout(debounceRef.current);
  }, [query, notes]);

  const showDropdown = isFocused && query.trim() !== '' && results.length > 0;

  const handleSelect = useCallback((noteId: string) => {
    onSelect(noteId);
    // Keep dropdown open — don't clear query
  }, [onSelect]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!showDropdown) return;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setActiveIndex(prev => Math.min(prev + 1, results.length - 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        setActiveIndex(prev => Math.max(prev - 1, 0));
        break;
      case 'Enter':
        e.preventDefault();
        if (results[activeIndex]) {
          handleSelect(results[activeIndex].note.id);
        }
        break;
      case 'Escape':
        e.preventDefault();
        setQuery('');
        inputRef.current?.blur();
        break;
    }
  };

  // Scroll active item into view within the dropdown
  useEffect(() => {
    if (!showDropdown || !dropdownRef.current) return;
    const active = dropdownRef.current.children[activeIndex] as HTMLElement | undefined;
    active?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex, showDropdown]);

  const handleClear = () => {
    setQuery('');
    inputRef.current?.focus();
  };

  return (
    <div className="relative px-2 py-2 border-b border-border">
      <div className="relative">
        <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" />
        <input
          ref={inputRef}
          type="text"
          placeholder="Search notes\u2026"
          value={query}
          onChange={e => setQuery(e.target.value)}
          onFocus={() => setIsFocused(true)}
          onBlur={() => {
            // Delay so click on dropdown result registers before closing
            setTimeout(() => setIsFocused(false), 200);
          }}
          onKeyDown={handleKeyDown}
          className="w-full pl-7 pr-7 py-1 text-sm bg-muted/50 border border-input rounded focus:outline-none focus:ring-1 focus:ring-primary"
        />
        {query && (
          <button
            onMouseDown={e => e.preventDefault()}
            onClick={handleClear}
            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      {/* Dropdown */}
      {showDropdown && (
        <div
          ref={dropdownRef}
          className="absolute left-2 right-2 top-full mt-1 bg-background border border-border rounded shadow-lg max-h-[300px] overflow-y-auto z-10"
        >
          {results.map((result, i) => (
            <div
              key={result.note.id}
              className={`px-3 py-2 cursor-pointer ${
                i === activeIndex ? 'bg-secondary' : 'hover:bg-secondary/50'
              }`}
              onMouseDown={e => e.preventDefault()}
              onClick={() => handleSelect(result.note.id)}
              onMouseEnter={() => setActiveIndex(i)}
            >
              <div className="text-sm font-medium truncate">{result.note.title}</div>
              {result.matchField !== 'title' && (
                <div className="text-xs text-muted-foreground truncate">
                  {result.matchField}: {matchSnippet(result.matchValue, query)}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* No results message */}
      {isFocused && query.trim() !== '' && results.length === 0 && (
        <div className="absolute left-2 right-2 top-full mt-1 bg-background border border-border rounded shadow-lg z-10 px-3 py-2 text-sm text-muted-foreground">
          No notes found.
        </div>
      )}
    </div>
  );
}

export default SearchBar;
