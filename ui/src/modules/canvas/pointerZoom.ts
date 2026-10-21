// Pan / zoom / fit for the diagram previews (MermaidCanvas, D2Canvas). The
// canvas keeps the transform in a rune and renders it; everything that moves
// it — fit, the zoom buttons, wheel/pinch, arrow keys, drag and two-finger
// pinch — lives here, so the two canvases keep only their rendering code.
import { panDelta } from './panKeys.ts';

interface Point { x: number; y: number }
export interface Transform { scale: number; tx: number; ty: number }

/** The scale every gesture clamps into. */
export const MIN_SCALE = 0.1;
export const MAX_SCALE = 6;
/** Fit never blows a small diagram up past this. */
export const FIT_MAX_SCALE = 4;
/** Fit leaves an 8 % margin around the diagram. */
const FIT_MARGIN = 0.92;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Scale `t` by `factor` about the surface point (cx, cy), so that point stays put. */
export function zoomAt(t: Transform, factor: number, cx: number, cy: number): Transform {
  const scale = clamp(t.scale * factor, MIN_SCALE, MAX_SCALE);
  const ratio = scale / t.scale;
  return { scale, tx: cx - (cx - t.tx) * ratio, ty: cy - (cy - t.ty) * ratio };
}

/** Centre a `natW`×`natH` diagram in a `width`×`height` surface. */
export function fitTransform(width: number, height: number, natW: number, natH: number): Transform {
  const scale = clamp(Math.min(width / natW, height / natH) * FIT_MARGIN, MIN_SCALE, FIT_MAX_SCALE);
  return { scale, tx: (width - natW * scale) / 2, ty: (height - natH * scale) / 2 };
}

/** Pointer coordinates are relative to the preview surface, not the page. */
export class PointerZoom {
  private points = new Map<number, Point>();

  get active(): boolean { return this.points.size > 0; }
  down(id: number, x: number, y: number): void { this.points.set(id, { x, y }); }
  up(id: number): void { this.points.delete(id); }

  move(id: number, x: number, y: number, current: Transform): Transform {
    const previous = this.points.get(id);
    if (!previous) return current;
    const before = [...this.points.values()].slice(0, 2);
    this.points.set(id, { x, y });
    if (before.length === 1) {
      return { ...current, tx: current.tx + x - previous.x, ty: current.ty + y - previous.y };
    }
    const after = [...this.points.values()].slice(0, 2);
    const distance = (p: Point[]) => Math.hypot(p[1].x - p[0].x, p[1].y - p[0].y);
    const midpoint = (p: Point[]) => ({ x: (p[0].x + p[1].x) / 2, y: (p[0].y + p[1].y) / 2 });
    const a = midpoint(before), b = midpoint(after);
    const oldDistance = distance(before);
    const scale = clamp(current.scale * (oldDistance > 0 ? distance(after) / oldDistance : 1), MIN_SCALE, MAX_SCALE);
    const ratio = scale / current.scale;
    return { scale, tx: b.x - (a.x - current.tx) * ratio, ty: b.y - (a.y - current.ty) * ratio };
  }
}

/** How a canvas hands its surface + transform rune to `PanZoom`. */
export interface PanZoomHost {
  /** The preview surface (the element carrying the handlers). */
  surface: () => HTMLElement | null;
  get: () => Transform;
  set: (t: Transform) => void;
  /** The canvas's "the user panned or zoomed" flag: re-renders and resizes
   *  keep their view instead of re-fitting while it is set. Gestures set it,
   *  `fitView()` clears it (and so does the canvas on a fresh generate). */
  adjusted: () => boolean;
  setAdjusted: (on: boolean) => void;
  /** A drag started / ended (the grabbing cursor). */
  onDragging?: (on: boolean) => void;
}

export class PanZoom {
  private natW = 800;
  private natH = 600;
  private dragging = false;
  private pointers = new PointerZoom();
  private host: PanZoomHost;

  constructor(host: PanZoomHost) {
    this.host = host;
  }

  /** Pin the rendered `<svg>` to its natural (viewBox) size so the CSS
   *  transform is the only scaling, then fit unless the user has moved it. */
  sizeSvg(content: HTMLElement | null | undefined): void {
    const svg = content?.querySelector('svg');
    if (!svg) return;
    const vb = svg.viewBox?.baseVal;
    this.natW = (vb && vb.width) || svg.getBoundingClientRect().width || 800;
    this.natH = (vb && vb.height) || svg.getBoundingClientRect().height || 600;
    svg.style.maxWidth = 'none';
    svg.style.width = `${this.natW}px`;
    svg.style.height = `${this.natH}px`;
    if (!this.host.adjusted()) this.fit();
  }

  fit(): void {
    const r = this.host.surface()?.getBoundingClientRect();
    if (!r || !r.width || !r.height) return;
    this.host.set(fitTransform(r.width, r.height, this.natW, this.natH));
  }

  fitView(): void {
    this.host.setAdjusted(false);
    this.fit();
  }

  /** The zoom buttons: about the surface centre. */
  zoomBy(factor: number): void {
    const r = this.host.surface()?.getBoundingClientRect();
    if (!r) return;
    this.host.setAdjusted(true);
    this.host.set(zoomAt(this.host.get(), factor, r.width / 2, r.height / 2));
  }

  /** Pinch (ctrl+wheel on macOS) zooms about the cursor; a plain two-finger
   *  scroll pans. */
  wheel(e: WheelEvent): void {
    e.preventDefault();
    const r = this.host.surface()?.getBoundingClientRect();
    if (!r) return;
    this.host.setAdjusted(true);
    const t = this.host.get();
    if (!e.ctrlKey) {
      this.host.set({ ...t, tx: t.tx - e.deltaX, ty: t.ty - e.deltaY });
      return;
    }
    this.host.set(zoomAt(t, Math.exp(-e.deltaY * 0.0015), e.clientX - r.left, e.clientY - r.top));
  }

  /** Arrow-key pan while the surface itself has focus (same state as a drag). */
  key(e: KeyboardEvent): void {
    const d = panDelta(e);
    if (!d) return;
    e.preventDefault();
    this.host.setAdjusted(true);
    const t = this.host.get();
    this.host.set({ ...t, tx: t.tx - d.dx, ty: t.ty - d.dy });
  }

  pointerDown(e: PointerEvent): void {
    const surface = this.host.surface();
    if ((e.target as HTMLElement | null)?.closest?.('a') || !surface) return;
    const r = surface.getBoundingClientRect();
    this.pointers.down(e.pointerId, e.clientX - r.left, e.clientY - r.top);
    this.setDragging(true);
    this.host.setAdjusted(true);
    (e.currentTarget as HTMLElement | null)?.setPointerCapture?.(e.pointerId);
  }

  pointerMove(e: PointerEvent): void {
    const surface = this.host.surface();
    if (!surface || !this.dragging) return;
    const r = surface.getBoundingClientRect();
    this.host.set(this.pointers.move(e.pointerId, e.clientX - r.left, e.clientY - r.top, this.host.get()));
  }

  /** Up and cancel alike. */
  pointerUp(e: PointerEvent): void {
    this.pointers.up(e.pointerId);
    this.setDragging(this.pointers.active);
    const target = e.currentTarget as HTMLElement | null;
    if (target?.hasPointerCapture?.(e.pointerId)) target.releasePointerCapture(e.pointerId);
  }

  private setDragging(on: boolean): void {
    if (on === this.dragging) return;
    this.dragging = on;
    this.host.onDragging?.(on);
  }
}
