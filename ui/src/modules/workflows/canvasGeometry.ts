// Workflow canvas geometry (pure): card sizes, loop-card height and the edge
// paths between ports, in graph coordinates. Split out of WorkflowCanvas.svelte.
import type { WorkflowNode } from '../../lib/api/types';

export const NODE_W = 210;
const NODE_H = 62;
const HEAD_H = 60; // header row height (icon + title) inside any node
const STEP_H = 22; // one inner loop step row
const LOOP_FOOT = 24; // the "until … · max …" footer row

export interface LoopStep {
  name: string;
  kind: string;
}
export function loopSteps(n: WorkflowNode): LoopStep[] {
  const p = n.params as { steps?: { name?: string; kind?: string }[] } | null;
  const steps = Array.isArray(p?.steps) ? p!.steps : [];
  return steps.map((s, i) => ({ name: s?.name || `step ${i + 1}`, kind: s?.kind || '?' }));
}
export function loopUntil(n: WorkflowNode): string {
  return (n.params as { until?: string } | null)?.until ?? '';
}
export function loopMax(n: WorkflowNode): number | undefined {
  return (n.params as { max_iterations?: number } | null)?.max_iterations;
}
/** The actual card height — loop nodes grow to show their inner steps. */
export function nodeHeight(n: WorkflowNode): number {
  if (n.kind === 'loop') {
    const c = loopSteps(n).length;
    return HEAD_H + c * STEP_H + (loopUntil(n) ? LOOP_FOOT : 0) + 8;
  }
  return NODE_H;
}

// Bezier path between an output port and an input port (graph coords).
export function edgePath(s: WorkflowNode, t: WorkflowNode): string {
  const x1 = s.x + NODE_W;
  const y1 = s.y + nodeHeight(s) / 2;
  const x2 = t.x;
  const y2 = t.y + nodeHeight(t) / 2;
  const dx = Math.max(40, Math.abs(x2 - x1) * 0.5);
  return `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}`;
}

/** Midpoint of an edge (for the condition badge). */
export function edgeMid(s: WorkflowNode, t: WorkflowNode): { x: number; y: number } {
  return {
    x: (s.x + NODE_W + t.x) / 2,
    y: (s.y + nodeHeight(s) / 2 + t.y + nodeHeight(t) / 2) / 2,
  };
}
export function condLabel(c: string): string {
  return c.length > 18 ? `${c.slice(0, 17)}…` : c;
}
