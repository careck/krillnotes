import type { Note, FieldValue } from '../types';

export interface SearchResult {
  note: Note;
  matchField: string;   // "title" or the field name that matched
  matchValue: string;    // the text value that contained the match
}

/**
 * Extracts the string value from a FieldValue if it is a text-like type.
 * Returns null for Number, Boolean, and Date fields.
 */
function textContent(fv: FieldValue): string | null {
  if ('Text' in fv) return fv.Text;
  if ('Email' in fv) return fv.Email;
  return null;
}

/**
 * Searches notes by matching a query against the title and all text-like
 * field values (Text, Email). Case-insensitive substring match.
 *
 * Returns at most one SearchResult per note (first match wins, title checked first).
 * Returns an empty array if query is empty or whitespace-only.
 */
export function searchNotes(notes: Note[], query: string): SearchResult[] {
  const trimmed = query.trim().toLowerCase();
  if (trimmed === '') return [];

  const results: SearchResult[] = [];

  for (const note of notes) {
    // Check title first
    if (note.title.toLowerCase().includes(trimmed)) {
      results.push({ note, matchField: 'title', matchValue: note.title });
      continue;
    }

    // Check text-like fields
    let matched = false;
    for (const [fieldName, fieldValue] of Object.entries(note.fields)) {
      const text = textContent(fieldValue);
      if (text !== null && text.toLowerCase().includes(trimmed)) {
        results.push({ note, matchField: fieldName, matchValue: text });
        matched = true;
        break;
      }
    }
    if (matched) continue;
  }

  return results;
}
