// A tiny dependency-free force-directed layout for the Vault graph view.
//
// Deliberately small: a naive O(n²) charge step (fine for the few-hundred-node
// graphs the Vault produces) plus Hooke springs on edges and a gentle pull to
// the canvas centre, integrated with velocity damping and a cooling `alpha`
// (mirrors d3-force's alpha decay so the layout settles and the rAF loop can
// stop). No external deps — see AGENTS.md ("No new npm dependencies").

export interface SimNode {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** When non-null the node is pinned here (dragging / explicit fix). */
  fx: number | null;
  fy: number | null;
  /** Edge degree — used by callers for size-by-degree and orphan filtering. */
  degree: number;
}

export interface SimEdge {
  source: string;
  target: string;
}

export interface ForceParams {
  /** Pull toward the canvas centre (0 = none). */
  centerStrength: number;
  /** Repulsion magnitude between every pair of nodes. */
  repel: number;
  /** Rest length of the edge springs. */
  linkDistance: number;
  /** Spring stiffness (0–1). */
  linkStrength: number;
}

export const DEFAULT_PARAMS: ForceParams = {
  centerStrength: 0.04,
  repel: 1400,
  linkDistance: 80,
  linkStrength: 0.25,
};

export class ForceSim {
  nodes: SimNode[] = [];
  edges: SimEdge[] = [];
  byId = new Map<string, SimNode>();

  width = 800;
  height = 600;
  params: ForceParams = { ...DEFAULT_PARAMS };

  // Cooling schedule (d3-force defaults).
  alpha = 1;
  private alphaMin = 0.005;
  private alphaDecay = 0.0228;
  private velocityDecay = 0.4;

  /**
   * Replace the graph, preserving positions for ids that survive so the layout
   * doesn't jump when filters toggle. New nodes are seeded on a ring around the
   * centre. Recomputes per-node degree.
   */
  setGraph(nodes: { id: string }[], edges: SimEdge[]): void {
    const prev = this.byId;
    const next = new Map<string, SimNode>();
    const cx = this.width / 2;
    const cy = this.height / 2;
    const r = Math.min(this.width, this.height) / 3;
    nodes.forEach((n, i) => {
      const existing = prev.get(n.id);
      if (existing) {
        existing.degree = 0;
        next.set(n.id, existing);
      } else {
        const a = (2 * Math.PI * i) / Math.max(nodes.length, 1);
        next.set(n.id, {
          id: n.id,
          x: cx + r * Math.cos(a) + (Math.random() - 0.5) * 40,
          y: cy + r * Math.sin(a) + (Math.random() - 0.5) * 40,
          vx: 0,
          vy: 0,
          fx: null,
          fy: null,
          degree: 0,
        });
      }
    });
    // Only keep edges whose endpoints both exist; tally degree.
    const liveEdges: SimEdge[] = [];
    for (const e of edges) {
      const s = next.get(e.source);
      const t = next.get(e.target);
      if (s && t) {
        s.degree++;
        t.degree++;
        liveEdges.push(e);
      }
    }
    this.byId = next;
    this.nodes = [...next.values()];
    this.edges = liveEdges;
  }

  /** Re-heat the layout (e.g. on drag or a param change). */
  reheat(target = 1): void {
    this.alpha = Math.max(this.alpha, target);
  }

  resize(w: number, h: number): void {
    this.width = w;
    this.height = h;
  }

  /** Advance one tick. Returns false once the layout has cooled (caller stops). */
  step(): boolean {
    if (this.alpha < this.alphaMin) return false;
    this.alpha += (0 - this.alpha) * this.alphaDecay;

    const a = this.alpha;
    const { repel, linkDistance, linkStrength, centerStrength } = this.params;
    const ns = this.nodes;
    const n = ns.length;

    // Repulsion — every pair (O(n²), cooled by alpha).
    for (let i = 0; i < n; i++) {
      const ni = ns[i];
      for (let j = i + 1; j < n; j++) {
        const nj = ns[j];
        let dx = nj.x - ni.x;
        let dy = nj.y - ni.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) {
          // Coincident nodes — nudge apart deterministically-ish.
          dx = (Math.random() - 0.5) * 0.5;
          dy = (Math.random() - 0.5) * 0.5;
          d2 = dx * dx + dy * dy + 0.01;
        }
        const dist = Math.sqrt(d2);
        const force = (repel * a) / d2;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        ni.vx -= fx;
        ni.vy -= fy;
        nj.vx += fx;
        nj.vy += fy;
      }
    }

    // Springs on edges (Hooke toward linkDistance).
    for (const e of this.edges) {
      const s = this.byId.get(e.source);
      const t = this.byId.get(e.target);
      if (!s || !t) continue;
      const dx = t.x - s.x;
      const dy = t.y - s.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 0.5;
      const diff = ((dist - linkDistance) / dist) * linkStrength * a;
      const fx = dx * diff;
      const fy = dy * diff;
      s.vx += fx;
      s.vy += fy;
      t.vx -= fx;
      t.vy -= fy;
    }

    // Gentle centering.
    const cx = this.width / 2;
    const cy = this.height / 2;
    for (const node of ns) {
      node.vx += (cx - node.x) * centerStrength * a;
      node.vy += (cy - node.y) * centerStrength * a;
    }

    // Integrate with damping; pinned nodes hold their fixed position.
    for (const node of ns) {
      if (node.fx != null) {
        node.x = node.fx;
        node.vx = 0;
      } else {
        node.vx *= 1 - this.velocityDecay;
        node.x += node.vx;
      }
      if (node.fy != null) {
        node.y = node.fy;
        node.vy = 0;
      } else {
        node.vy *= 1 - this.velocityDecay;
        node.y += node.vy;
      }
    }

    return true;
  }
}
