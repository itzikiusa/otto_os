// Design export helpers shared by Design Hall (⋯ → Export) and the Product
// Design Arena (Export ▾): PNG and SVG for Mermaid and Excalidraw (D2 renders
// in the Canvas module and hands its SVG to `svgTextToPng`), a PNG snapshot of
// a three.js viewport, and blob downloads. The Arena's attachment-specific
// helpers (`downloadSource`, `withExt`) stay beside it in
// modules/product/design/exporters.ts, which re-exports these.
import { renderMermaid } from '../render/mermaid';

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

/** A file-name stem from a title ("Hero v2 / dark" → "Hero-v2-dark"). */
export function fileStem(title: string): string {
  return title.replace(/[^\w.-]+/g, '-').replace(/^-+|-+$/g, '') || 'design';
}

/** Mermaid → standalone SVG file. */
export async function mermaidToSvg(source: string): Promise<string> {
  // Through the shared lazy loader (lib/render/mermaid.ts): mermaid stays out of
  // the page's chunk, and renders are serialized with every other one.
  const { svg, error } = await renderMermaid(`otto-design-export-${Date.now()}`, source);
  if (svg === undefined) throw new Error(error ?? 'Diagram error');
  return svg;
}

/** Rasterize an <svg> at `scale`× its viewBox onto a white PNG and download it. */
function svgElToPng(svgEl: SVGSVGElement, filename: string, scale = 2): void {
  const vb = svgEl.viewBox?.baseVal;
  const w = (vb && vb.width) || svgEl.getBoundingClientRect().width || 800;
  const h = (vb && vb.height) || svgEl.getBoundingClientRect().height || 600;
  const clone = svgEl.cloneNode(true) as SVGSVGElement;
  clone.setAttribute('width', String(w));
  clone.setAttribute('height', String(h));
  const xml = new XMLSerializer().serializeToString(clone);
  const img = new Image();
  img.onload = () => {
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(w * scale));
    canvas.height = Math.max(1, Math.round(h * scale));
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    // A white backing: a downloaded diagram should read dropped into a doc or slide.
    ctx.fillStyle = '#ffffff'; // ui-guards: allow — the exported image's own page colour
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
    canvas.toBlob((blob) => blob && downloadBlob(blob, filename), 'image/png');
  };
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(xml)}`;
}

/** A rendered diagram SVG → PNG download (rasterised offscreen). The SVG is
 *  agent/user-authored: it is parsed as `image/svg+xml` with DOMParser (an inert
 *  document — no scripts run, no `innerHTML` into the app origin), checked for a
 *  parser error, and only then adopted for measurement + rasterisation. */
export function svgTextToPng(svg: string, filename: string): void {
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
  holder.style.insetInlineStart = '-99999px';
  holder.appendChild(el);
  document.body.appendChild(holder);
  try {
    svgElToPng(el, filename, 2);
  } finally {
    // svgElToPng clones the node synchronously, so the holder can go.
    holder.remove();
  }
}

/** Mermaid → PNG download. */
export async function mermaidToPng(source: string, filename: string): Promise<void> {
  svgTextToPng(await mermaidToSvg(source), filename);
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
