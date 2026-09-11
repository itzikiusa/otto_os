// The catalogue of Home boxes. Each kind is a self-contained live tile (its
// own data fetch + refresh cadence) rendered by HomeBox → boxes/<Kind>Box.svelte.
// A kind is offered in the "Add box" picker only when the user can VIEW the
// feature behind it — the same RBAC gate the module itself sits behind.

import type { Feature } from '../../lib/api/types';

export type HomeBoxKind =
  | 'sessions'
  | 'mission-control'
  | 'db-dashboard'
  | 'k8s'
  | 'insights'
  | 'usage';

export interface HomeBoxKindDef {
  kind: HomeBoxKind;
  label: string;
  /** One line for the picker. */
  blurb: string;
  /** Icon name (see Icon.svelte). */
  icon: string;
  /** RBAC gate (checked at 'view'). */
  feature: Feature;
  /** Default footprint on the 12-column grid. */
  w: number;
  h: number;
  /** Module route the box's "open" affordance jumps to. */
  route: string;
}

export const HOME_KINDS: HomeBoxKindDef[] = [
  {
    kind: 'sessions',
    label: 'Agents',
    blurb: 'Live agent sessions — who is working, idle, or waiting on you.',
    icon: 'terminal',
    feature: 'agents',
    w: 4,
    h: 4,
    route: 'agents',
  },
  {
    kind: 'mission-control',
    label: 'Mission Control',
    blurb: 'Summary of every running piece of work across the workspace.',
    icon: 'radar',
    feature: 'mission_control',
    w: 8,
    h: 4,
    route: 'mission-control',
  },
  {
    kind: 'db-dashboard',
    label: 'DB dashboard',
    blurb: 'Any Database Explorer dashboard, live, with its own refresh cadence.',
    icon: 'db',
    feature: 'database',
    w: 12,
    h: 5,
    route: 'connections',
  },
  {
    kind: 'k8s',
    label: 'Kubernetes',
    blurb: 'Cluster health at a glance: pods, restarts, memory, rps, errors.',
    icon: 'helm',
    feature: 'kubernetes',
    w: 6,
    h: 4,
    route: 'kubernetes',
  },
  {
    kind: 'insights',
    label: 'Insights',
    blurb: 'The latest daily / weekly / monthly report summaries.',
    icon: 'gauge',
    feature: 'insights',
    w: 6,
    h: 4,
    route: 'insights',
  },
  {
    kind: 'usage',
    label: 'Usage',
    blurb: 'Spend, tokens and events per provider for the last N days.',
    icon: 'chart',
    feature: 'usage',
    w: 6,
    h: 4,
    route: 'usage',
  },
];

export function kindDef(kind: HomeBoxKind): HomeBoxKindDef {
  return HOME_KINDS.find((k) => k.kind === kind) ?? HOME_KINDS[0];
}

export function isHomeKind(s: unknown): s is HomeBoxKind {
  return typeof s === 'string' && HOME_KINDS.some((k) => k.kind === s);
}
