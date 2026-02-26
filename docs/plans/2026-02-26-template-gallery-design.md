# Template Gallery — Grid Cards Design

Date: 2026-02-26

## Problem

The templates list page (`/templates/`) renders a plain link list — just a `<strong>` title and a description line per item. It is visually inconsistent with the rest of the site and does not showcase templates well.

## Goal

Replace the plain list with a card grid that shows a screenshot thumbnail, the template title, description, and a "View template" link. The design should match the site's visual language and scale from 1 to many templates.

## Approved Design

### Card layout

Each card is a vertical stack:

1. **Screenshot thumbnail** — full card width, 180px tall, `object-fit: cover`, subtle top border-radius matching the card.
2. **Title** — `var(--ocean-deep)` color, medium weight.
3. **Description** — `var(--sand-400)` color, 0.9rem.
4. **"View template →" link** — `var(--krill-primary)` color, at the bottom.

If no screenshot is set, the thumbnail area shows a tinted placeholder background.

### Grid

`repeat(auto-fit, minmax(280px, 1fr))` — same rhythm as the homepage features grid.

- 1 card → full width
- 2 cards → side by side
- 3+ cards → 3-column grid

### Hover

`translateY(-4px)` lift + increased `box-shadow` — consistent with `.feature-card`.

## Files Changed

| File | Change |
|---|---|
| `layouts/templates/list.html` | New Hugo section-specific list layout |
| `content/templates/book-collection.md` | Add `screenshot` frontmatter param |
| `static/css/style.css` | Add `.template-grid` + `.template-card` styles |

## Implementation Notes

- Hugo resolves `layouts/templates/list.html` before `layouts/_default/list.html` for the templates section — no changes needed to other layouts.
- Each template page exposes its screenshot via `{{ .Params.screenshot }}` in the layout.
- The `.template-card` styles are new (not reusing `.feature-card`) because the image thumbnail at the top requires different padding/overflow handling.
