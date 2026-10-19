// The assist response carries the scene's CURRENT format and the source for
// that format only. A Mermaid editor can receive a D2 stop result when the
// stored scene changed during the turn; the originating component must not
// assume its own format.
import type { AssistResult, CanvasDoc, CanvasFormat } from './types';

/** Build the canvas doc the response actually committed. `fallback` is used
 *  only when an older response omits `format`. An empty source for the named
 *  format is not filled from another format's field. */
export function docFromAssist(res: AssistResult, fallback: CanvasFormat, sketch?: boolean): CanvasDoc | null {
  const format: CanvasFormat =
    res.format === 'mermaid' || res.format === 'd2' || res.format === 'excalidraw' ? res.format : fallback;
  let source = '';
  if (format === 'excalidraw') {
    if (res.excalidraw == null) return null;
    source = typeof res.excalidraw === 'string' ? res.excalidraw : JSON.stringify(res.excalidraw);
  } else if (format === 'd2') {
    source = res.d2 ?? '';
  } else {
    source = res.mermaid ?? '';
  }
  if (!source.trim()) return null;
  const doc: CanvasDoc = { type: 'otto-canvas', version: 1, format, source };
  if (format === 'd2' && typeof sketch === 'boolean') doc.sketch = sketch;
  return doc;
}
