/**
 * Returns a deterministic HSL background color for a tag.
 * Hue is derived from the sum of the tag's char codes, giving the same
 * tag the same color across renders and sessions.
 */
export function tagColor(tag: string): string {
  const hue = [...tag].reduce((acc, c) => acc + c.charCodeAt(0), 0) % 360;
  return `hsl(${hue}, 40%, 88%)`;
}
