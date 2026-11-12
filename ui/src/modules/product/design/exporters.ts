// Design Arena — Export ▾ helpers (design §4.1). The format exporters moved to
// lib/design/exporters.ts (Design Hall's ⋯ → Export uses them too); this file
// re-exports them for the Arena and keeps its attachment-specific helpers.
import { downloadText } from '../../../lib/components/exporters';
import type { ProductAttachment } from '../types';
import { FORMATS, kindOf } from '../../../lib/design/format';

export {
  canvasToPng,
  downloadBlob,
  excalidrawToPng,
  excalidrawToSvg,
  mermaidToPng,
  mermaidToSvg,
} from '../../../lib/design/exporters';

/** Download the artifact's source verbatim, under its own file name + mime. */
export function downloadSource(att: ProductAttachment, source: string): void {
  const k = kindOf(att);
  const mime = k === 'html' || k === 'mermaid' || k === 'excalidraw' || k === 'scene3d' ? FORMATS[k].mime : att.mime || 'text/plain';
  downloadText(source, att.filename || 'design.txt', mime);
}

/** Derive `stem.ext` from the attachment's file name. */
export function withExt(att: ProductAttachment, ext: string): string {
  const name = att.filename || 'design';
  const dot = name.lastIndexOf('.');
  return `${dot > 0 ? name.slice(0, dot) : name}.${ext}`;
}
