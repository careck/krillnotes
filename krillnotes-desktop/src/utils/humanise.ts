/**
 * Converts a snake_case field key to a Title Case display label.
 *
 * Examples:
 *   "first_name"        → "First Name"
 *   "note_title"        → "Note Title"
 *   "email"             → "Email"
 *   "first_name (legacy)" → "First Name (legacy)"
 */
export function humaniseKey(key: string): string {
  return key
    .split('_')
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}
