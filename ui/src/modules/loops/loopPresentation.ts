import type { GoalLoop } from '../../lib/api/types';
import type { IconName } from '../../lib/components/Icon.svelte';

export const PHASES = ['planning', 'executing', 'evaluating', 'digesting'] as const;
export const PHASE_LABEL: Record<(typeof PHASES)[number], string> = {
  planning: 'Plan', executing: 'Execute', evaluating: 'Evaluate', digesting: 'Digest',
};

/** Succeeded marks each phase done. Only a currently running phase pulses. */
export function stepVisual(loop: GoalLoop, i: number): { past: boolean; active: boolean; current: boolean } {
  if (loop.status === 'succeeded') return { past: true, active: false, current: false };
  const phase = loop.phase === 'waiting' ? 'executing' : loop.phase;
  let index = PHASES.indexOf(phase as (typeof PHASES)[number]);
  if (index < 0) index = PHASES.length - 1;
  const current = index === i;
  return { past: index > i, active: loop.status === 'running' && current, current };
}

export function progressFill(status: string): string {
  switch (status) {
    case 'running': return 'running';
    case 'blocked': case 'exhausted': return 'warn';
    case 'failed': return 'bad';
    case 'succeeded': return 'ok';
    default: return 'dim';
  }
}

export function criterionMark(met: boolean | undefined): { icon: IconName; label: string; kind: string } {
  if (met === true) return { icon: 'check', label: 'Met', kind: 'met' };
  if (met === false) return { icon: 'x', label: 'Not met', kind: 'unmet' };
  return { icon: 'minus', label: 'Not checked', kind: 'unchecked' };
}
