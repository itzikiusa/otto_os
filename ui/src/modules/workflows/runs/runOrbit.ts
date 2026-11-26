// The run ring's geometry (ux.md §6): one arc per top-level slot of the step
// list in execution order (stepOrder.ts builds both), clockwise from 12
// o'clock with equal angles; a parallel group as concentric lanes (4 at most,
// the 4th standing for the rest); a fan-out as one tick per child up to 12,
// above that one segment per state sized by count. Colour is state only (the
// component maps a state to a token). Pure; unit-tested in
// ui/unit/workflowRunOrbit.test.ts.
import type { ChildThread, NodeRunState, RunState } from '../../../lib/api/types';
import { entrySteps, type StepEntry } from './stepOrder.ts';

export type ArcState = 'done' | 'working' | 'waiting' | 'needs' | 'failed' | 'pending' | 'skipped' | 'canceled';

export const VIEW = 120;
export const STROKE = 10;
export const SLOT_GAP = 5;
export const TICK_GAP = 1.6;
export const MAX_TICKS = 12;
const OUTER = 59;
const LANE_GAP = 1.5;
const C = VIEW / 2;

export interface Arc {
  d: string;
  state: ArcState;
  width: number;
  /** Index of the slot (entry) the arc belongs to. */
  slot: number;
}

export interface Orbit {
  arcs: Arc[];
  slots: number;
  /** Children of the largest fan-out slot (0 when none). */
  children: number;
  /** That fan-out is drawn as state segments (more than 12 children). */
  grouped: boolean;
}

export function nodeArcState(n: NodeRunState, run: RunState): ArcState {
  switch (n.status) {
    case 'success': return 'done';
    case 'error': return 'failed';
    case 'skipped': return n.error_class === 'canceled' ? 'canceled' : 'skipped';
    case 'running':
      if (n.phase === 'needs_you' || n.attention) return 'needs';
      return n.phase === 'waiting' || n.phase === 'queued' ? 'waiting' : 'working';
    default: return run === 'canceled' ? 'canceled' : 'pending';
  }
}

export function childArcState(c: ChildThread): ArcState {
  switch (c.status) {
    case 'success': return 'done';
    case 'error': return 'failed';
    case 'needs_you': return 'needs';
    case 'running': return 'working';
    case 'skipped': return 'skipped';
    default: return 'pending';
  }
}

/** Segment order for a grouped fan-out. */
const STATE_ORDER: ArcState[] = ['done', 'failed', 'needs', 'working', 'waiting', 'pending', 'skipped', 'canceled'];
/** Which state the 4th lane shows for the lanes it stands for. */
const URGENCY: ArcState[] = ['failed', 'needs', 'working', 'waiting', 'pending', 'canceled', 'skipped', 'done'];

/** [start, end] angles (degrees from 12 o'clock) of `n` equal slots; the
 *  slots and the gaps between them make the full circle. */
export function slotAngles(n: number): [number, number][] {
  if (n <= 0) return [];
  const span = (360 - n * SLOT_GAP) / n;
  return Array.from({ length: n }, (_, i) => {
    const a0 = i * (span + SLOT_GAP) + SLOT_GAP / 2;
    return [a0, a0 + span];
  });
}

const r2 = (v: number) => Math.round(v * 100) / 100;

function arcPath(r: number, a0: number, a1: number): string {
  const p = (a: number) => {
    const rad = (a * Math.PI) / 180;
    return `${r2(C + r * Math.sin(rad))} ${r2(C - r * Math.cos(rad))}`;
  };
  return `M ${p(a0)} A ${r2(r)} ${r2(r)} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${p(a1)}`;
}

/** Split [a0, a1] into parts sized by `weights`, TICK_GAP apart. */
function split(a0: number, a1: number, weights: number[]): [number, number][] {
  const total = weights.reduce((s, w) => s + w, 0);
  const room = a1 - a0 - (weights.length - 1) * TICK_GAP;
  let at = a0;
  return weights.map((w) => {
    const seg: [number, number] = [at, at + (room * w) / total];
    at = seg[1] + TICK_GAP;
    return seg;
  });
}

export function buildOrbit(entries: readonly StepEntry[], run: RunState): Orbit {
  const angles = slotAngles(entries.length);
  const arcs: Arc[] = [];
  let children = 0;
  const mid = OUTER - STROKE / 2;
  entries.forEach((e, slot) => {
    const [a0, a1] = angles[slot];
    if (e.type === 'group') {
      let lanes = e.steps.map((s) => nodeArcState(s.node, run));
      if (lanes.length > 4) {
        const rest = lanes.slice(3);
        lanes = [...lanes.slice(0, 3), URGENCY.find((s) => rest.includes(s)) ?? rest[0]];
      }
      const k = lanes.length;
      const w = (STROKE - (k - 1) * LANE_GAP) / k;
      lanes.forEach((state, j) => arcs.push({ d: arcPath(OUTER - j * (w + LANE_GAP) - w / 2, a0, a1), state, width: r2(w), slot }));
      return;
    }
    const kids = e.step.node.children ?? [];
    if (kids.length === 0) {
      arcs.push({ d: arcPath(mid, a0, a1), state: nodeArcState(e.step.node, run), width: STROKE, slot });
      return;
    }
    children = Math.max(children, kids.length);
    const states = kids.map(childArcState);
    if (kids.length <= MAX_TICKS) {
      split(a0, a1, states.map(() => 1)).forEach(([b0, b1], i) => arcs.push({ d: arcPath(mid, b0, b1), state: states[i], width: STROKE, slot }));
      return;
    }
    const present = STATE_ORDER.filter((s) => states.includes(s));
    split(a0, a1, present.map((s) => states.filter((x) => x === s).length)).forEach(([b0, b1], i) =>
      arcs.push({ d: arcPath(mid, b0, b1), state: present[i], width: STROKE, slot }),
    );
  });
  return { arcs, slots: entries.length, children, grouped: children > MAX_TICKS };
}

/** The ring's accessible name: the steps done and each fan-out's progress, from
 *  the same entries as the step list. */
export function orbitLabel(entries: readonly StepEntry[], word: string): string {
  const steps = entries.flatMap(entrySteps);
  const done = steps.filter((s) => s.node.status === 'success').length;
  const parts = [`${word}: ${done} of ${steps.length} steps done`];
  for (const s of steps) {
    const kids = s.node.children ?? [];
    if (kids.length) parts.push(`${s.name} ${kids.filter((c) => c.status === 'success').length} of ${kids.length} done`);
  }
  return parts.join('; ');
}
