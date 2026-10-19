interface Point { x: number; y: number }
interface Transform { scale: number; tx: number; ty: number }

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
    const scale = Math.max(0.1, Math.min(6, current.scale * (oldDistance > 0 ? distance(after) / oldDistance : 1)));
    const ratio = scale / current.scale;
    return { scale, tx: b.x - (a.x - current.tx) * ratio, ty: b.y - (a.y - current.ty) * ratio };
  }
}
