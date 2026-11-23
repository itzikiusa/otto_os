// How a running Smart production reads while it is being made: which step a
// piece is on (in plain words, "step 2 of 3"), a clock that ticks from when
// it started, a bar that keeps moving inside a long step without ever going
// back or reaching the end before the step does, ONE time left that counts
// down between server updates, and the short state the "In progress" strip
// shows. Pure, unit-tested in unit/createProgress.test.ts.
import type { Production, ProductionItem, ProductionItemKind } from '../../../lib/api/types';

/** A production item with the live fields the server adds while making (step, working_since, waiting, has_preview…: types/media.ts). */
export type LiveItem = ProductionItem;

/** A production with its place in the queue (`queue_position`) and the agent that plans it (types/media.ts). */
export type LiveProduction = Production;

/** "0:07", "2:41", "1:02:05". */
export function clockText(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** Seconds a step usually takes, by its plain words (measured on this Mac, one piece at a time). */
const STEP_GUESS: [RegExp, number][] = [
  [/library/i, 2],
  [/picture|draw/i, 90],
  [/cut/i, 5],
  [/3d|shape/i, 170],
  [/ready|finish/i, 80],
  [/skeleton|joint/i, 300],
  [/move/i, 45],
  [/voice|say|speak|record/i, 4],
  [/sound|music/i, 100],
];

/** Seconds a whole piece takes (a generated object ≈ 5.5 min, a character ≈ 13 min). */
export const PIECE_SECONDS: Record<ProductionItemKind, number> = { character: 780, object: 330, voice_line: 5, sound: 100, music: 150 };

/** About how long this piece's current step lasts. */
export function stepSeconds(item: Pick<LiveItem, 'kind' | 'step'>): number {
  const hit = item.step && STEP_GUESS.find(([re]) => re.test(item.step!.label));
  if (hit) return hit[1];
  return PIECE_SECONDS[item.kind] / Math.max(1, item.step?.of ?? 1);
}

/**
 * Where a working piece's bar stands (0–100). It starts at the step's own
 * mark (or the server's percent, whichever is further), eases toward the end
 * of the step as the step goes on, and never gets closer than a tenth of the
 * step to that end: only the server finishing the step moves it past.
 */
export function barPct(item: Pick<LiveItem, 'kind' | 'step' | 'progress_pct'>, inStepSecs: number): number {
  const pct = Math.min(99, Math.max(0, item.progress_pct || 0));
  const s = item.step;
  const lo = s ? Math.max(pct, ((s.n - 1) / s.of) * 100) : pct;
  const hi = s ? Math.max(lo, (s.n / s.of) * 100) : Math.min(99, lo + 25);
  const t = Math.max(0, inStepSecs) / Math.max(1, stepSeconds(item));
  return Math.min(99, lo + (hi - lo) * 0.9 * (1 - Math.exp(-t)));
}

/** Remembers when each piece's step was first seen, so a step's clock and bar keep going between polls. */
export class StepClock {
  private seen = new Map<string, number>();
  /** Epoch ms the piece's current step started: its working_since for the first step, else when it was first seen. */
  start(item: Pick<LiveItem, 'id' | 'step' | 'working_since'>, nowMs: number): number {
    const key = `${item.id}:${item.step?.n ?? 0}`;
    const since = item.working_since ? Date.parse(item.working_since) : NaN;
    if (!this.seen.has(key)) this.seen.set(key, (item.step?.n ?? 1) <= 1 && Number.isFinite(since) ? Math.min(since, nowMs) : nowMs);
    return this.seen.get(key)!;
  }
}

/** The one time left: the server's estimate and when it was given. */
export interface Eta {
  secs: number;
  at: number;
}
export const etaLeft = (e: Eta, nowMs: number): number => Math.max(0, e.secs - (nowMs - e.at) / 1000);

/**
 * The server re-says its estimate on every poll; it only changes when a step
 * ends. Keep counting down from the last change, take a lower one at once,
 * and only go up when the server says it is really longer (two minutes or more).
 */
export function nextEta(prev: Eta | null, secs: number, nowMs: number): Eta {
  if (!prev) return { secs, at: nowMs };
  if (prev.secs === secs) return prev;
  const left = etaLeft(prev, nowMs);
  if (secs > left && secs - left < 120) return prev;
  return { secs, at: nowMs };
}

const isMade = (i: ProductionItem) => i.status === 'done';
const isSound = (i: ProductionItem) => i.kind === 'sound' || i.kind === 'music';

/** The pieces that will really be made (what the plan shows; no sounds without the sound maker). */
export function madeItems(p: Production, soundReady: boolean): ProductionItem[] {
  return (p.plan?.items ?? []).filter((i) => i.enabled && (soundReady || !isSound(i)));
}

/** The short state of a creation on the "In progress" strip and in My creations. */
export function stripState(p: LiveProduction, soundReady: boolean): string {
  if (p.status === 'planning') return 'Planning';
  if (p.status === 'done') return 'Ready to look';
  if (p.status === 'ready') return 'Plan ready';
  if (p.status !== 'running') return '';
  if ((p.queue_position ?? 0) > 0) return `Waiting, ${p.queue_position} ahead`;
  const made = madeItems(p, soundReady);
  const done = made.filter(isMade).length;
  return `Making ${Math.min(made.length, done + 1)} of ${made.length}`;
}

/** The picture that stands for a creation while it is made: its first person or thing with a picture (made, or being drawn). */
export function firstPicture(p: Production): { artifact: string | null; preview: string | null } | null {
  const it = ((p.plan?.items ?? []) as LiveItem[]).find((i) => (i.kind === 'character' || i.kind === 'object') && (i.result?.artifact_id || i.has_preview));
  if (!it) return null;
  return { artifact: it.result?.artifact_id ?? null, preview: it.has_preview ? `${p.id}/${it.id}/${it.preview_rev ?? 0}` : null };
}

/** The scene can be looked at while the rest is still coming: a scene exists, some pieces are made and some are not. */
export function sceneSoFar(p: Production, soundReady: boolean): boolean {
  const made = madeItems(p, soundReady);
  // A person already standing in the scene while its skeleton is still being added counts as there.
  const there = (i: ProductionItem) => !!i.result?.artifact_id && (isMade(i) || i.status === 'working' || i.made === 'mesh');
  return p.status === 'running' && !!p.scene_artifact_id && made.some(there) && made.some((i) => !isMade(i));
}
