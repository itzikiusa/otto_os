// Toolbar tool vocabulary — the shared seam between the Toolbar (which picks a
// tool) and the CanvasEditor (which inserts on the next pane click). A `Tool` is
// either a bare insert kind, the pointer ('select') / edge ('connector') mode,
// or a specific shape variant encoded as `shape:<variant>`.

import type { ShapeVariant } from './types';

export type Tool =
  | 'select'
  | 'connector'
  | 'sticky'
  | 'text'
  | `shape:${ShapeVariant}`
  | 'mermaid'
  | 'code'
  | 'json'
  | 'image'
  | 'frame'
  | 'freehand';

/** The shape variants offered in the Shape tool menu, in display order. */
export const SHAPE_VARIANTS: { variant: ShapeVariant; label: string }[] = [
  { variant: 'rect', label: 'Rectangle' },
  { variant: 'roundrect', label: 'Rounded' },
  { variant: 'ellipse', label: 'Ellipse' },
  { variant: 'diamond', label: 'Diamond' },
  { variant: 'triangle', label: 'Triangle' },
  { variant: 'cylinder', label: 'Cylinder' },
  { variant: 'parallelogram', label: 'Parallelogram' },
];

/** Is this tool a one-shot insert (vs. select/connector modes)? */
export function isInsertTool(t: Tool): boolean {
  return t !== 'select' && t !== 'connector';
}
