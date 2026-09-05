// Design Arena — Export ▾ helpers (design §4.1): PNG (board / viewport snapshot),
// SVG (mermaid / excalidraw), the raw source file, and the server-generated
// Blender script. GLB export of a scene3d lives with the 3D viewport (Track C's
// `scene3d/exportGlb.ts`); the arena wires it when that module lands.
import mermaid from 'mermaid';
import { downloadText } from '../../../lib/components/exporters';
import { svgToPngDownload } from '../../canvas/export';
import type { ProductAttachment } from '../types';
import { FORMATS, kindOf } from './format';

/** Trigger a download of a Blob. */
export function downloadBlob(blob: Blob, filename: string): void {
  if (typeof document === 'undefined') return;
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

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

/** Mermaid → standalone SVG file. */
export async function mermaidToSvg(source: string): Promise<string> {
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict' });
  const { svg } = await mermaid.render(`otto-design-export-${Date.now()}`, source);
  return svg;
}

/** Mermaid → PNG download (rasterises the rendered SVG offscreen). The SVG is
 *  agent/user-authored: it is parsed as `image/svg+xml` with DOMParser (an inert
 *  document — no scripts run, no `innerHTML` into the app origin), checked for a
 *  parser error, and only then adopted for measurement + rasterisation. */
export async function mermaidToPng(source: string, filename: string): Promise<void> {
  const svg = await mermaidToSvg(source);
  const parsed = new DOMParser().parseFromString(svg, 'image/svg+xml');
  if (parsed.querySelector('parsererror')) throw new Error('Rendered diagram is not valid SVG');
  const rootEl = parsed.documentElement;
  if (!(rootEl instanceof SVGSVGElement) && rootEl.nodeName.toLowerCase() !== 'svg') {
    throw new Error('Rendered diagram has no <svg> root');
  }
  // Strip anything executable before it touches the live document (belt and
  // braces — mermaid's strict mode already refuses scripts).
  for (const bad of Array.from(parsed.querySelectorAll('script, foreignObject'))) bad.remove();
  const el = document.adoptNode(rootEl) as unknown as SVGSVGElement;
  const holder = document.createElement('div');
  holder.style.position = 'fixed';
  holder.style.left = '-99999px';
  holder.appendChild(el);
  document.body.appendChild(holder);
  try {
    svgToPngDownload(el, filename, 2);
  } finally {
    // svgToPngDownload clones the node synchronously, so the holder can go.
    holder.remove();
  }
}

/** Excalidraw scene source → PNG blob via the library's exporter (lazy import). */
export async function excalidrawToPng(source: string, dark: boolean): Promise<Blob> {
  const Ex = await import('@excalidraw/excalidraw');
  const raw = JSON.parse(source) as { elements?: unknown[]; appState?: Record<string, unknown>; files?: unknown };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const anyEx = Ex as any;
  return anyEx.exportToBlob({
    elements: raw.elements ?? [],
    appState: { ...(raw.appState ?? {}), exportWithDarkMode: dark, exportBackground: true },
    files: raw.files ?? null,
    mimeType: 'image/png',
    exportPadding: 24,
  }) as Promise<Blob>;
}

/** Excalidraw scene source → SVG string via the library's exporter (lazy import). */
export async function excalidrawToSvg(source: string, dark: boolean): Promise<string> {
  const Ex = await import('@excalidraw/excalidraw');
  const raw = JSON.parse(source) as { elements?: unknown[]; appState?: Record<string, unknown>; files?: unknown };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const anyEx = Ex as any;
  const svg = (await anyEx.exportToSvg({
    elements: raw.elements ?? [],
    appState: { ...(raw.appState ?? {}), exportWithDarkMode: dark, exportBackground: true },
    files: raw.files ?? null,
    exportPadding: 24,
  })) as SVGSVGElement;
  return new XMLSerializer().serializeToString(svg);
}

/** Snapshot the first `<canvas>` inside `root` (the three.js viewport) as PNG.
 *  Requires the renderer to keep its drawing buffer or to have just rendered;
 *  a black/blank result means `preserveDrawingBuffer` is off in the viewport. */
export function canvasToPng(root: HTMLElement, filename: string): boolean {
  const canvas = root.querySelector('canvas');
  if (!canvas) return false;
  canvas.toBlob((blob) => {
    if (blob) downloadBlob(blob, filename);
  }, 'image/png');
  return true;
}
