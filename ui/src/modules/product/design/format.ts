// Design Arena — format plumbing shared by the arena, the asset list, the
// assistant and the templates (design/product-design-arena.md §2.2). ONE place
// maps a `DesignFormat` to its file name / mime / label / arena group, so the
// Rust `DesignFormat` enum and this table can be diffed side by side.
import type { DesignFormat, ProductAttachment } from '../types';
import { designFormatOf, metaOf } from '../../../lib/stores/mockup-assist.svelte';
import { SCENE3D_MIME } from './scene3d';

export const EXCALIDRAW_MIME = 'application/vnd.excalidraw+json';
export const GLB_MIME = 'model/gltf-binary';
export const GLTF_MIME = 'model/gltf+json';

/** What the viewport renders for an attachment. `model` = a standalone glb/gltf
 *  (shown through a one-object scene), `image` = `<img>`, `other` = unsupported. */
export type ArtifactKind = DesignFormat | 'image' | 'model' | 'other';

export interface FormatInfo {
  format: DesignFormat;
  /** Menu label ("Screen", "Board"…). */
  label: string;
  /** One-line menu hint. */
  hint: string;
  /** Default file name for a blank artifact (mirrors `mockup_assist::file_name`). */
  filename: string;
  mime: string;
  /** Arena asset group (`meta.group` overrides). */
  group: string;
  icon: string;
}

export const FORMATS: Record<DesignFormat, FormatInfo> = {
  html: {
    format: 'html',
    label: 'Screen',
    hint: 'HTML screen — a self-contained UI mockup',
    filename: 'design.html',
    mime: 'text/html',
    group: 'Screens',
    icon: 'square',
  },
  excalidraw: {
    format: 'excalidraw',
    label: 'Board',
    hint: 'Freeform Excalidraw board — frames, shapes, arrows',
    filename: 'design.excalidraw',
    mime: EXCALIDRAW_MIME,
    group: 'Boards',
    icon: 'edit',
  },
  mermaid: {
    format: 'mermaid',
    label: 'Diagram',
    hint: 'A Mermaid flow / sequence / model',
    filename: 'design.mmd',
    mime: 'text/vnd.mermaid',
    group: 'Diagrams',
    icon: 'branch',
  },
  scene3d: {
    format: 'scene3d',
    label: '3D scene',
    hint: 'Viewport + hierarchy + inspector; exports to Blender',
    filename: 'scene.json',
    mime: SCENE3D_MIME,
    group: '3D',
    icon: 'box',
  },
};

/** Arena group order (design §4 sketch). Unknown `meta.group`s append after. */
export const GROUP_ORDER = ['Screens', 'Boards', 'Diagrams', '3D', 'Images', 'Files'];

/** Attachments the arena lists: `mockup` + `design` kinds, images, and anything
 *  whose bytes classify as a design format or a 3D model (a pre-`design` upload
 *  of a `.mmd` / `.html` / `.glb` still shows up). */
export function isDesignAttachment(a: ProductAttachment): boolean {
  if (a.kind === 'mockup' || a.kind === 'design') return true;
  return kindOf(a) !== 'other';
}

export function kindOf(a: ProductAttachment): ArtifactKind {
  const fmt = designFormatOf(a);
  if (fmt) return fmt;
  const mime = (a.mime || '').toLowerCase();
  const name = (a.filename || '').toLowerCase();
  if (mime === GLB_MIME || mime === GLTF_MIME || name.endsWith('.glb') || name.endsWith('.gltf')) return 'model';
  if (mime.startsWith('image/')) return 'image';
  return 'other';
}

/** The arena group of an attachment: `meta.group` wins, else by kind. */
export function groupOf(a: ProductAttachment): string {
  const g = metaOf(a).group;
  if (g && g.trim()) return g.trim();
  const k = kindOf(a);
  if (k === 'image') return 'Images';
  if (k === 'model') return '3D';
  if (k === 'other') return 'Files';
  return FORMATS[k].group;
}

/** Short type badge for a list row. */
export function typeLabel(a: ProductAttachment): string {
  const k = kindOf(a);
  switch (k) {
    case 'html': return 'HTML';
    case 'mermaid': return 'Mermaid';
    case 'excalidraw': return 'Board';
    case 'scene3d': return '3D';
    case 'model': return (a.filename || '').toLowerCase().endsWith('.gltf') ? 'glTF' : 'GLB';
    case 'image': return (a.mime || '').toLowerCase() === 'image/svg+xml' ? 'SVG' : 'Image';
    default: return a.mime || 'File';
  }
}

/** Formats whose bytes are text the arena can edit in place / save via PUT. */
export function isTextKind(k: ArtifactKind): k is DesignFormat {
  return k === 'html' || k === 'mermaid' || k === 'excalidraw' || k === 'scene3d';
}

/** Map a picked file to an allow-listed mime (best-effort by extension) — the
 *  browser often leaves `File.type` empty for `.mmd` / `.excalidraw` / `.glb`. */
export function mimeForFile(f: File): string {
  const n = f.name.toLowerCase();
  if (n.endsWith('.mmd')) return 'text/vnd.mermaid';
  if (n.endsWith('.excalidraw')) return EXCALIDRAW_MIME;
  if (n.endsWith('.scene3d.json')) return SCENE3D_MIME;
  if (n.endsWith('.glb')) return GLB_MIME;
  if (n.endsWith('.gltf')) return GLTF_MIME;
  if (f.type) return f.type;
  if (n.endsWith('.html') || n.endsWith('.htm')) return 'text/html';
  if (n.endsWith('.svg')) return 'image/svg+xml';
  if (n.endsWith('.json')) return 'application/json';
  return 'application/octet-stream';
}

/** `accept` for the Import picker (design §4.1). */
export const IMPORT_ACCEPT =
  'image/*,.svg,.png,.jpg,.jpeg,.webp,.gif,.html,.htm,.mmd,.excalidraw,.glb,.gltf,.json,' +
  `text/html,text/vnd.mermaid,${EXCALIDRAW_MIME},${SCENE3D_MIME},${GLB_MIME},${GLTF_MIME}`;

/** The `kind` a new upload carries (Track A's convention, §8): `mockup` for
 *  html / mermaid / images, `design` for excalidraw / scene3d / glb / gltf. */
export function kindForUpload(mime: string, filename = ''): string {
  const m = mime.toLowerCase();
  const n = filename.toLowerCase();
  if (m === EXCALIDRAW_MIME || m === SCENE3D_MIME || m === GLB_MIME || m === GLTF_MIME) return 'design';
  if (n.endsWith('.excalidraw') || n.endsWith('.glb') || n.endsWith('.gltf') || n.endsWith('.scene3d.json')) return 'design';
  if (m === 'text/html' || m === 'text/vnd.mermaid' || m.startsWith('image/') || n.endsWith('.mmd') || n.endsWith('.html')) return 'mockup';
  return 'file';
}

/** Read a Blob → base64 (no data-URL prefix). */
export function fileToB64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const result = reader.result as string;
      const idx = result.indexOf(',');
      resolve(idx >= 0 ? result.slice(idx + 1) : result);
    };
    reader.readAsDataURL(blob);
  });
}

/** A unique-enough file name for a fresh artifact: `design-3.html`. */
export function nextFilename(format: DesignFormat, existing: ProductAttachment[]): string {
  const base = FORMATS[format].filename;
  const dot = base.lastIndexOf('.');
  const stem = base.slice(0, dot);
  const ext = base.slice(dot);
  const names = new Set(existing.map((a) => (a.filename || '').toLowerCase()));
  if (!names.has(base.toLowerCase())) return base;
  for (let i = 2; i < 1000; i++) {
    const cand = `${stem}-${i}${ext}`;
    if (!names.has(cand.toLowerCase())) return cand;
  }
  return `${stem}-${Date.now()}${ext}`;
}

/** Wrap an SVG string in a minimal sandbox-safe doc for an iframe `srcdoc`
 *  (shared by the viewer, the live preview and the arena). */
export function svgDoc(svg: string): string {
  return (
    `<!doctype html><html><head><meta charset="utf-8">` +
    `<style>html,body{margin:0;padding:8px;background:#fff;}` +
    `svg{max-width:100%;height:auto;display:block;margin:0 auto;}</style></head>` +
    `<body>${svg}</body></html>`
  );
}
