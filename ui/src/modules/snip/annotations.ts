// Annotation object model + canvas renderer for the snip editor.
//
// Pure TS (no Svelte imports) so the geometry/rendering layer is testable and
// the component stays presentational. All coordinates live in IMAGE-PIXEL
// space — the component maps pointer events through its CSS scale factor, and
// `flatten()` re-renders at the image's natural resolution, so exports stay
// crisp on retina captures regardless of on-screen zoom.

export type Tool =
  | 'select'
  | 'rect'
  | 'ellipse'
  | 'arrow'
  | 'line'
  | 'pen'
  | 'highlight'
  | 'text'
  | 'pixelate'
  | 'badge';

export interface Pt {
  x: number;
  y: number;
}

export interface Anno {
  id: number;
  tool: Exclude<Tool, 'select'>;
  /** Bounding pair — for pen/highlight it tracks the path's bbox. */
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  /** Freehand path (pen/highlight). */
  points?: Pt[];
  /** Text content (text tool). */
  text?: string;
  color: string;
  /** Stroke width in image px. */
  stroke: number;
  /** Font size in image px (text). */
  font: number;
  /** Badge ordinal (badge tool). */
  n?: number;
}

export const PALETTE = [
  '#e5484d', // red (default)
  '#f76b15', // orange
  '#ffe629', // yellow
  '#46a758', // green
  '#0090ff', // blue
  '#8e4ec6', // purple
  '#1c2024', // near-black
  '#ffffff', // white
] as const;

/** Stroke widths (S/M/L) in image px; scaled up-front for retina images. */
export const STROKES = [3, 5, 9] as const;
/** Font sizes (S/M/L) in image px. */
export const FONTS = [16, 24, 36] as const;

const HANDLE_HIT = 10;

function norm(a: Anno): { x: number; y: number; w: number; h: number } {
  const x = Math.min(a.x1, a.x2);
  const y = Math.min(a.y1, a.y2);
  return { x, y, w: Math.abs(a.x2 - a.x1), h: Math.abs(a.y2 - a.y1) };
}

/** Text is measured at render time; keep the last box so hit-testing works. */
const textBoxes = new Map<number, { x: number; y: number; w: number; h: number }>();

export function badgeRadius(a: Anno): number {
  return Math.max(12, a.font * 0.75);
}

// ── Rendering ────────────────────────────────────────────────────────────────

/** Draw the base image + all annotations onto `ctx` (canvas must already be
 *  sized to the image's natural dimensions). */
export function render(
  ctx: CanvasRenderingContext2D,
  img: CanvasImageSource & { width: number; height: number },
  annos: Anno[],
): void {
  ctx.clearRect(0, 0, img.width, img.height);
  ctx.drawImage(img, 0, 0);
  for (const a of annos) drawAnno(ctx, img, a);
}

function drawAnno(
  ctx: CanvasRenderingContext2D,
  img: CanvasImageSource & { width: number; height: number },
  a: Anno,
): void {
  ctx.save();
  ctx.strokeStyle = a.color;
  ctx.fillStyle = a.color;
  ctx.lineWidth = a.stroke;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  const { x, y, w, h } = norm(a);
  switch (a.tool) {
    case 'rect':
      ctx.strokeRect(x, y, w, h);
      break;
    case 'ellipse':
      ctx.beginPath();
      ctx.ellipse(x + w / 2, y + h / 2, w / 2, h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    case 'line':
      ctx.beginPath();
      ctx.moveTo(a.x1, a.y1);
      ctx.lineTo(a.x2, a.y2);
      ctx.stroke();
      break;
    case 'arrow': {
      drawArrow(ctx, a);
      break;
    }
    case 'pen':
    case 'highlight': {
      const pts = a.points ?? [];
      if (pts.length < 2) break;
      if (a.tool === 'highlight') {
        ctx.globalAlpha = 0.4;
        ctx.globalCompositeOperation = 'multiply';
        ctx.lineWidth = Math.max(a.stroke * 4, 14);
      }
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (const p of pts.slice(1)) ctx.lineTo(p.x, p.y);
      ctx.stroke();
      break;
    }
    case 'text': {
      const lines = (a.text ?? '').split('\n');
      ctx.font = `600 ${a.font}px ui-sans-serif, system-ui, sans-serif`;
      ctx.textBaseline = 'top';
      // Subtle contrast halo so text reads on any background.
      let maxW = 0;
      for (const line of lines) maxW = Math.max(maxW, ctx.measureText(line).width);
      const lh = a.font * 1.25;
      textBoxes.set(a.id, { x: a.x1, y: a.y1, w: maxW, h: lh * lines.length });
      ctx.lineWidth = Math.max(3, a.font / 6);
      ctx.strokeStyle = a.color === '#ffffff' ? 'rgba(0,0,0,.75)' : 'rgba(255,255,255,.85)';
      lines.forEach((line, i) => {
        ctx.strokeText(line, a.x1, a.y1 + i * lh);
        ctx.fillText(line, a.x1, a.y1 + i * lh);
      });
      break;
    }
    case 'pixelate': {
      if (w < 2 || h < 2) break;
      // Mosaic: draw the covered region through a tiny offscreen canvas.
      const block = Math.max(8, Math.round(Math.max(w, h) / 24));
      const tw = Math.max(1, Math.round(w / block));
      const th = Math.max(1, Math.round(h / block));
      const off = document.createElement('canvas');
      off.width = tw;
      off.height = th;
      const octx = off.getContext('2d');
      if (!octx) break;
      octx.imageSmoothingEnabled = true;
      octx.drawImage(img, x, y, w, h, 0, 0, tw, th);
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(off, 0, 0, tw, th, x, y, w, h);
      ctx.imageSmoothingEnabled = true;
      break;
    }
    case 'badge': {
      const r = badgeRadius(a);
      ctx.beginPath();
      ctx.arc(a.x1, a.y1, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.font = `700 ${Math.round(r)}px ui-sans-serif, system-ui, sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = a.color === '#ffffff' || a.color === '#ffe629' ? '#1c2024' : '#ffffff';
      ctx.fillText(String(a.n ?? 1), a.x1, a.y1 + r * 0.05);
      break;
    }
  }
  ctx.restore();
}

function drawArrow(ctx: CanvasRenderingContext2D, a: Anno): void {
  const dx = a.x2 - a.x1;
  const dy = a.y2 - a.y1;
  const len = Math.hypot(dx, dy);
  if (len < 1) return;
  const head = Math.max(12, a.stroke * 3.5);
  const angle = Math.atan2(dy, dx);
  // Shaft stops where the head begins so the tip stays sharp.
  const bx = a.x2 - Math.cos(angle) * head * 0.8;
  const by = a.y2 - Math.sin(angle) * head * 0.8;
  ctx.beginPath();
  ctx.moveTo(a.x1, a.y1);
  ctx.lineTo(bx, by);
  ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(a.x2, a.y2);
  ctx.lineTo(a.x2 - head * Math.cos(angle - Math.PI / 7), a.y2 - head * Math.sin(angle - Math.PI / 7));
  ctx.lineTo(a.x2 - head * Math.cos(angle + Math.PI / 7), a.y2 - head * Math.sin(angle + Math.PI / 7));
  ctx.closePath();
  ctx.fill();
}

/** Selection chrome: dashed bbox + corner handles (drawn ABOVE the objects). */
export function drawSelection(ctx: CanvasRenderingContext2D, a: Anno): void {
  const b = bounds(a);
  ctx.save();
  ctx.strokeStyle = '#0090ff';
  ctx.lineWidth = 1.5;
  ctx.setLineDash([6, 4]);
  ctx.strokeRect(b.x - 4, b.y - 4, b.w + 8, b.h + 8);
  ctx.setLineDash([]);
  ctx.fillStyle = '#0090ff';
  for (const [hx, hy] of corners(b)) {
    ctx.fillRect(hx - 4, hy - 4, 8, 8);
  }
  ctx.restore();
}

// ── Geometry / hit-testing ───────────────────────────────────────────────────

export function bounds(a: Anno): { x: number; y: number; w: number; h: number } {
  if (a.tool === 'text') {
    const t = textBoxes.get(a.id);
    if (t) return t;
    const lines = (a.text ?? '').split('\n');
    return { x: a.x1, y: a.y1, w: a.font * 6, h: a.font * 1.25 * lines.length };
  }
  if (a.tool === 'badge') {
    const r = badgeRadius(a);
    return { x: a.x1 - r, y: a.y1 - r, w: r * 2, h: r * 2 };
  }
  if ((a.tool === 'pen' || a.tool === 'highlight') && a.points?.length) {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const p of a.points) {
      minX = Math.min(minX, p.x);
      minY = Math.min(minY, p.y);
      maxX = Math.max(maxX, p.x);
      maxY = Math.max(maxY, p.y);
    }
    return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
  }
  return norm(a);
}

function corners(b: { x: number; y: number; w: number; h: number }): [number, number][] {
  return [
    [b.x - 4, b.y - 4],
    [b.x + b.w + 4, b.y - 4],
    [b.x - 4, b.y + b.h + 4],
    [b.x + b.w + 4, b.y + b.h + 4],
  ];
}

/** Which resize corner (0..3) of the selection is under (x,y), if any. */
export function hitHandle(a: Anno, x: number, y: number): number | null {
  const b = bounds(a);
  const cs = corners(b);
  for (let i = 0; i < cs.length; i++) {
    if (Math.abs(x - cs[i][0]) <= HANDLE_HIT && Math.abs(y - cs[i][1]) <= HANDLE_HIT) return i;
  }
  return null;
}

/** Topmost annotation under (x,y) — expanded bbox hit, newest first. */
export function hitTest(annos: Anno[], x: number, y: number): Anno | null {
  for (let i = annos.length - 1; i >= 0; i--) {
    const b = bounds(annos[i]);
    const pad = Math.max(6, annos[i].stroke);
    if (x >= b.x - pad && x <= b.x + b.w + pad && y >= b.y - pad && y <= b.y + b.h + pad) {
      return annos[i];
    }
  }
  return null;
}

export function moveAnno(a: Anno, dx: number, dy: number): Anno {
  const moved: Anno = { ...a, x1: a.x1 + dx, y1: a.y1 + dy, x2: a.x2 + dx, y2: a.y2 + dy };
  if (a.points) moved.points = a.points.map((p) => ({ x: p.x + dx, y: p.y + dy }));
  return moved;
}

/** Resize by dragging corner `handle` to (x,y). Point-based and single-point
 *  tools (pen paths keep their shape) just move instead. */
export function resizeAnno(a: Anno, handle: number, x: number, y: number): Anno {
  if (a.tool === 'text' || a.tool === 'badge' || a.points) return a;
  const b = norm(a);
  let { x: nx, y: ny } = b;
  let nw = b.w;
  let nh = b.h;
  if (handle === 0 || handle === 2) {
    nw = b.x + b.w - x;
    nx = x;
  } else {
    nw = x - b.x;
  }
  if (handle === 0 || handle === 1) {
    nh = b.y + b.h - y;
    ny = y;
  } else {
    nh = y - b.y;
  }
  if (nw < 4 || nh < 4) return a;
  return { ...a, x1: nx, y1: ny, x2: nx + nw, y2: ny + nh };
}

// ── Export ───────────────────────────────────────────────────────────────────

/** Flatten image + annotations to a PNG blob at natural resolution. */
export async function flatten(
  img: CanvasImageSource & { width: number; height: number },
  annos: Anno[],
): Promise<Blob> {
  const canvas = document.createElement('canvas');
  canvas.width = img.width;
  canvas.height = img.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('canvas 2d context unavailable');
  render(ctx, img, annos);
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (b) => (b ? resolve(b) : reject(new Error('PNG encode failed'))),
      'image/png',
    );
  });
}

export async function blobToB64(blob: Blob): Promise<string> {
  const buf = new Uint8Array(await blob.arrayBuffer());
  let bin = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < buf.length; i += CHUNK) {
    bin += String.fromCharCode(...buf.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}
