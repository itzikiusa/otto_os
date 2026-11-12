// ⋯ → Export for one design, by format: PNG and SVG for Mermaid, D2 and
// Excalidraw boards, the source file for every text format, the bytes for a
// binary, and the Blender script for a 3D scene that came from a Product
// story (the daemon generates it from the story attachment; a scene born in
// Design Hall has no script route yet). All local downloads — nothing leaves
// the machine.
import { authedText } from '../../lib/api/client';
import type { DesignArtifact } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { downloadText } from '../../lib/components/exporters';
import { toastError } from '../../lib/toastError';
import {
  downloadBlob,
  excalidrawToPng,
  excalidrawToSvg,
  fileStem,
  mermaidToSvg,
  svgTextToPng,
} from '../../lib/design/exporters';
import { blenderSourceOf, sourceExt } from './exportNames';

export { blenderSourceOf, sourceExt };

/** Download the generated Blender script (`.py`) of a story-backed scene. */
export async function downloadBlenderScript(a: DesignArtifact): Promise<void> {
  const src = blenderSourceOf(a);
  if (!src) return;
  try {
    const py = await authedText(`/product/stories/${encodeURIComponent(src.storyId)}/design/${encodeURIComponent(src.attachmentId)}/blender-script`);
    downloadText(py, `${fileStem(a.title)}.blender.py`, 'text/x-python');
  } catch (e) {
    toastError('Couldn’t export the Blender script', e);
  }
}

function fail(e: unknown): void {
  toastError('Couldn’t export the design', e);
}

/** The Export submenu items for the shown design. `source` is the text shown
 *  (null for a binary, whose bytes are `blobUrl`); `d2ToSvg` is the stage's D2
 *  renderer (ArtifactStage's `d2Svg` — D2 renders in the Canvas module). */
export function exportItems(
  a: DesignArtifact,
  source: string | null,
  blobUrl: string | null,
  dark: boolean,
  d2ToSvg: (source: string) => Promise<string>,
): MenuItem[] {
  const stem = fileStem(a.title);
  const items: MenuItem[] = [];
  const svgDownload = (svg: string) => downloadText(svg, `${stem}.svg`, 'image/svg+xml');
  if (source != null && (a.format === 'mermaid' || a.format === 'd2')) {
    const toSvg = a.format === 'mermaid' ? mermaidToSvg : d2ToSvg;
    items.push(
      { label: 'PNG image', icon: 'image', action: () => void toSvg(source).then((s) => svgTextToPng(s, `${stem}.png`)).catch(fail) },
      { label: 'SVG image', icon: 'image', action: () => void toSvg(source).then(svgDownload).catch(fail) },
    );
  } else if (source != null && a.format === 'excalidraw') {
    items.push(
      { label: 'PNG image', icon: 'image', action: () => void excalidrawToPng(source, dark).then((b) => downloadBlob(b, `${stem}.png`)).catch(fail) },
      { label: 'SVG image', icon: 'image', action: () => void excalidrawToSvg(source, dark).then(svgDownload).catch(fail) },
    );
  }
  if (source != null) {
    items.push({ label: `Source (.${sourceExt(a.format)})`, icon: 'file', action: () => downloadText(source, `${stem}.${sourceExt(a.format)}`, a.mime || 'text/plain') });
  } else if (blobUrl) {
    items.push({
      label: 'Original file', icon: 'download',
      action: () => void fetch(blobUrl).then((r) => r.blob()).then((b) => downloadBlob(b, a.title)).catch(fail),
    });
  }
  if (blenderSourceOf(a)) items.push({ label: 'Blender script (.py)', icon: 'file', action: () => void downloadBlenderScript(a) });
  return items;
}
