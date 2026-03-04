/**
 * Converts a workspace display name into a filesystem/window-label-safe slug.
 * Lowercases, replaces runs of non-alphanumeric characters with a single
 * hyphen, and strips leading/trailing hyphens.
 *
 * Used by both NewWorkspaceDialog and WorkspaceManagerDialog to ensure
 * the folder name is always a valid Tauri window label.
 */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}
