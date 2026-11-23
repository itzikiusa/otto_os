// Create — the plain-words front door for making characters, things, voices
// and sounds. Pure logic shared by the screens (and unit-tested): what each
// plain choice means technically, how a running job reads as child-sized
// steps, honest time estimates, and the words we show for the Smart plan.

import type {
  CreateProductionReq,
  Media3dJob,
  Media3dPipelineStage,
  ProductionItem,
  ProductionItemKind,
  ProductionSummary,
  ProductionUse,
  TtsVoice,
} from '../../../lib/api/types';
import { PIECE_SECONDS } from './progress.ts';

export type CreateKind = 'character' | 'object' | 'voice' | 'sound';
export type UseWhere = ProductionUse;

/**
 * "Where will you use it?" → the technical settings. The ONE table for it.
 * Today only `triangles` (and the `preset`) reach Otto Forge: `forge
 * generate` always runs its game-ready pass with 3 levels of detail and
 * collision shapes, and Otto's argv template cannot pass a texture size. The
 * other columns record the intent (and feed providers that read `preset`).
 */
export const WHERE: Record<UseWhere, { label: string; hint: string; triangles: number; texture: number; lods: number; collision: boolean; preset: string }> = {
  phone: { label: 'Phone game', hint: 'Small and quick', triangles: 5_000, texture: 1024, lods: 3, collision: true, preset: 'low_poly_prop' },
  pc: { label: 'Computer game', hint: 'More detail', triangles: 25_000, texture: 2048, lods: 3, collision: true, preset: 'stylized_character' },
  closeup: { label: 'Close-up', hint: 'The most detail', triangles: 100_000, texture: 4096, lods: 3, collision: true, preset: 'hero_mesh' },
};
export const WHERE_ORDER: UseWhere[] = ['phone', 'pc', 'closeup'];

// ── Who plans it ────────────────────────────────────────────────────────────

/** The planner chip's words: the agent, then its model by the catalog's name
 *  without the agent's own name in front ("Claude · Opus"), or "Default" for
 *  the agent's default model, then the effort in plain words when one was
 *  picked ("Claude Code · Opus 5.5 · High"; none picked: nothing). */
export function agentPickLabel(agent: string, model: string | null, modelName: string, effortLabel: string | null = null): string {
  const m = !model ? 'Default' : modelName.startsWith(`${agent} `) ? modelName.slice(agent.length + 1) : modelName;
  return `${agent} · ${m}${effortLabel ? ` · ${effortLabel}` : ''}`;
}

/** A Smart production's create body. A picked agent (and model, and effort)
 *  is sent; none picked sends neither, so the workspace default plans it. */
export function productionBody(
  workspace_id: string,
  description: string,
  pick: { provider: string; model: string | null; effort?: string | null } | null,
  project: string | null = null,
): CreateProductionReq & { effort?: string } {
  const agent = pick ? { provider: pick.provider, ...(pick.model ? { model: pick.model } : {}), ...(pick.effort ? { effort: pick.effort } : {}) } : {};
  return { workspace_id, description, use: 'pc', ...agent, ...(project ? { project_id: project } : {}) };
}

// ── Projects ───────────────────────────────────────────────────────────────

/** A project to start a new scene in: its id (its first creation's) and its name. */
export interface ProjectChoice {
  id: string;
  name: string;
}

/** The projects of the creations (newest first, as listed), each once, named
 *  by the project's name, else its first creation's title, else the newest one's. */
export function projectsOf(list: Pick<ProductionSummary, 'id' | 'project_id' | 'project_name' | 'title'>[]): ProjectChoice[] {
  const out: ProjectChoice[] = [];
  for (const p of list) {
    const id = p.project_id ?? p.id;
    if (out.some((x) => x.id === id)) continue;
    const first = list.find((x) => x.id === id);
    out.push({ id, name: p.project_name || first?.title || p.title || 'A project' });
  }
  return out;
}

/** A project's scenes on the shelf, under its name. */
export interface ProjectGroup<T> {
  key: string;
  label: string;
  items: T[];
}

/** The creations with the scenes of one project together, under its name,
 *  where its newest one stands; a project of one scene stays as it is. */
export function groupByProject<T extends { key: string }>(entries: T[], projectOf: (e: T) => ProjectChoice | null): (T | ProjectGroup<T>)[] {
  const count = new Map<string, number>();
  for (const e of entries) {
    const p = projectOf(e);
    if (p) count.set(p.id, (count.get(p.id) ?? 0) + 1);
  }
  const out: (T | ProjectGroup<T>)[] = [];
  const placed = new Set<string>();
  for (const e of entries) {
    const p = projectOf(e);
    if (!p || (count.get(p.id) ?? 0) < 2) out.push(e);
    else if (!placed.has(p.id)) {
      placed.add(p.id);
      out.push({ key: `project:${p.id}`, label: p.name, items: entries.filter((x) => projectOf(x)?.id === p.id) });
    }
  }
  return out;
}

/** The plan's honest word on what is reused: "9 of 12 pieces reused · about
 *  12 minutes instead of about an hour" (null when nothing is). */
export function reuseLine(items: ProductionItem[], secs: number, unreusedSecs: number | undefined): string | null {
  const models = items.filter((i) => i.enabled && (i.kind === 'character' || i.kind === 'object'));
  const reused = models.filter((i) => i.details?.reuse).length;
  if (!reused) return null;
  const saved = unreusedSecs && unreusedSecs > secs ? ` · ${timeText(secs).toLowerCase()} instead of ${timeText(unreusedSecs).toLowerCase()}` : '';
  return `${reused} of ${models.length} pieces reused${saved}`;
}


// ── Steps a child can follow ────────────────────────────────────────────────

export type StepId = 'draw' | 'shape' | 'ready' | 'skeleton' | 'moves';
export const STEP_LABEL: Record<StepId, string> = {
  draw: 'Drawing the idea',
  shape: 'Building the shape',
  ready: 'Finishing it',
  skeleton: 'Giving it joints',
  moves: 'Teaching it to move',
};
/** Seconds per step, measured with Otto Forge on an M5 Pro (docs/features/otto-forge.md §2.1). */
export const STEP_SECONDS: Record<StepId, number> = { draw: 55, shape: 195, ready: 40, skeleton: 60, moves: 15 };

/** The steps a 3D job walks through: a picture skips the drawing, a character adds a skeleton and moves. */
export function jobSteps(job: Pick<Media3dJob, 'prompt' | 'then_stages'>): StepId[] {
  const steps: StepId[] = job.prompt ? ['draw', 'shape', 'ready'] : ['shape', 'ready'];
  for (const s of job.then_stages ?? []) {
    if (s === 'rig') steps.push('skeleton');
    else if (s === 'animate') steps.push('moves');
  }
  return steps;
}

const STAGE_STEP: Partial<Record<Media3dPipelineStage, StepId>> = { gameready: 'ready', rig: 'skeleton', animate: 'moves' };

/**
 * Which step a job is on, from its current stage and the engine's own
 * progress words (Otto Forge: concept → cut-out → model → finish → game-ready).
 */
export function currentStep(job: Pick<Media3dJob, 'pipeline_stage' | 'stage' | 'status'>, steps: StepId[]): StepId {
  const byStage = STAGE_STEP[job.pipeline_stage];
  if (byStage) return byStage;
  const t = (job.stage || '').toLowerCase();
  if (job.status === 'queued' || /^generating 3d mesh with/.test(t)) return steps[0];
  if (/finish|game-ready|gameready|lod|saving/.test(t)) return 'ready';
  if (/concept|draw/.test(t)) return steps.includes('draw') ? 'draw' : 'shape';
  if (/cut|matte|background|build|model|mesh|pixal/.test(t)) return 'shape';
  return steps[0];
}

/** Seconds left: the steps not finished yet, never below what the steps still to come need. */
export function secondsLeft(steps: StepId[], current: StepId, elapsedSecs: number): number {
  const total = steps.reduce((n, s) => n + STEP_SECONDS[s], 0);
  const i = Math.max(0, steps.indexOf(current));
  const after = steps.slice(i + 1).reduce((n, s) => n + STEP_SECONDS[s], 0);
  return Math.max(after + 10, total - elapsedSecs);
}

/** "About 5 minutes", "About an hour", "Less than a minute". */
export function timeText(secs: number): string {
  if (secs < 60) return 'Less than a minute';
  const m = Math.round(secs / 60);
  if (m === 1) return 'About 1 minute';
  if (m < 50) return `About ${m} minutes`;
  if (m <= 80) return 'About an hour';
  const h = Math.round(m / 30) / 2;
  return `About ${h % 1 ? `${Math.floor(h)} and a half` : h} hours`;
}

/** Total time for a fresh creation (shown before it starts). */
export function estimateSecs(kind: 'character' | 'object', fromPicture: boolean): number {
  const steps = jobSteps({ prompt: fromPicture ? null : 'x', then_stages: kind === 'character' ? ['rig', 'animate'] : [] });
  return steps.reduce((n, s) => n + STEP_SECONDS[s], 0);
}

// ── Smart plan words ────────────────────────────────────────────────────────

export type GroupId = 'people' | 'things' | 'voices' | 'sounds';
export const GROUPS: { id: GroupId; label: string; kinds: ProductionItemKind[] }[] = [
  { id: 'people', label: 'People', kinds: ['character'] },
  { id: 'things', label: 'Things', kinds: ['object'] },
  { id: 'voices', label: 'Voices', kinds: ['voice_line'] },
  { id: 'sounds', label: 'Sounds and music', kinds: ['sound', 'music'] },
];

export function groupItems(items: ProductionItem[]): { id: GroupId; label: string; items: ProductionItem[] }[] {
  return GROUPS.map((g) => ({ id: g.id, label: g.label, items: items.filter((i) => g.kinds.includes(i.kind)) })).filter((g) => g.items.length > 0);
}

/** Seconds each kind of piece takes (measured on this Mac, one at a time: progress.ts). */
export const ITEM_SECONDS: Record<ProductionItemKind, number> = PIECE_SECONDS;

/**
 * THE time estimate, used the same way on the plan, while making and for a
 * change: what the pieces still to make take (pieces run one at a time), the
 * piece being made counted by how far along it is. It only ever counts down.
 */
export function remainingSecs(items: ProductionItem[]): number {
  return items
    .filter((i) => i.enabled && i.status !== 'done' && i.status !== 'failed' && i.status !== 'skipped' && !(i.status === 'waiting' && i.details?.reuse))
    .reduce((n, i) => n + ITEM_SECONDS[i.kind] * (i.status === 'working' ? 1 - Math.min(99, Math.max(0, i.progress_pct)) / 100 : 1), 0);
}

/** Kept for the unit tests and callers that pass a sound flag: the made pieces' remaining time. */
export function planSecs(items: ProductionItem[], soundReady = false): number {
  return remainingSecs(items.filter((i) => soundReady || (i.kind !== 'sound' && i.kind !== 'music')));
}

/** The steps a production item walks through while it is being made. */
export function itemSteps(kind: ProductionItemKind): StepId[] {
  if (kind === 'character') return ['draw', 'shape', 'ready', 'skeleton', 'moves'];
  if (kind === 'object') return ['draw', 'shape', 'ready'];
  return [];
}

/** The step a working 3D item is on, from its progress (equal shares of the bar). */
export function itemStep(item: Pick<ProductionItem, 'kind' | 'progress_pct' | 'message'>): StepId | null {
  const steps = itemSteps(item.kind);
  if (!steps.length) return null;
  const m = (item.message || '').toLowerCase();
  if (/skeleton|rig/.test(m) && steps.includes('skeleton')) return 'skeleton';
  if (/move|animat/.test(m) && steps.includes('moves')) return 'moves';
  const i = Math.min(steps.length - 1, Math.floor((Math.max(0, item.progress_pct) / 100) * steps.length));
  return steps[i];
}


/** What a sound or music item says while there is no sound maker. */
export const SOUND_WAITING = 'Not ready yet.';

const isSound = (i: ProductionItem) => i.kind === 'sound' || i.kind === 'music';

/** The plan as the person reads it: what will really be made, and one calm
 *  "Not yet" list of what can't be made today (sounds without a sound maker,
 *  moves and actions the engine doesn't have, the planner's own notes). */
export function readPlan(items: ProductionItem[], notes: string[], soundReady: boolean): { made: ProductionItem[]; notYet: string[] } {
  const made = items.filter((i) => i.enabled && (soundReady || !isSound(i)));
  const notYet: string[] = [];
  const sounds = items.filter((i) => i.enabled && isSound(i) && !soundReady);
  if (sounds.length) notYet.push(`Sounds and music aren’t ready yet (${sounds.map((s) => s.name.toLowerCase()).join(', ')}).`);
  if (made.some((i) => i.kind === 'character')) notYet.push('People can stand, walk and run. They can’t do other moves yet, like fighting.');
  for (const n of notes) if (!/sound|music|move|fight|walk|run/i.test(n)) notYet.push(n);
  return { made, notYet };
}

// ── Change something ────────────────────────────────────────────────────────

/** "the robot" from "Robot" or "The robot" (lowercased, one "the"). */
export function theName(name: string): string {
  const n = name.trim().toLowerCase();
  return n.startsWith('the ') ? n : `the ${n}`;
}

/** The change box's example, from THIS scene: its first person and first thing. */
export function steerExample(items: Pick<ProductionItem, 'kind' | 'name' | 'enabled'>[]): string {
  const who = items.find((i) => i.enabled && i.kind === 'character');
  const what = items.find((i) => i.enabled && i.kind === 'object');
  const parts = [who && `Make ${theName(who.name)} bigger.`, what && `Move ${theName(what.name)}…`].filter(Boolean);
  return parts.length ? parts.join(' ') : 'Tell me what to change…';
}

export interface ChangeNote {
  text: string;
  /** Plain words for where the note sits, when known ("on the chest"). */
  where?: string;
}

/** The new idea for a rebuild: the original idea plus every change note. */
export function changePrompt(idea: string, notes: ChangeNote[]): string {
  const base = idea.trim().replace(/[.\s]+$/, '');
  const list = notes
    .map((n) => n.text.trim())
    .map((t, i) => (t ? `${t}${notes[i].where ? ` (${notes[i].where})` : ''}` : ''))
    .filter(Boolean);
  return list.length ? `${base}. Changes: ${list.join('; ')}.` : base;
}

// ── Saving into the game folder ─────────────────────────────────────────────

export function slugName(title: string, fallback: string): string {
  const s = title
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
    .replace(/-+$/, '');
  return s || fallback;
}

/** The default place in the workspace: `assets/characters/knight.glb`. */
export function savePath(kind: CreateKind, title: string, ext: string, n = 1): string {
  const folder = kind === 'character' ? 'characters' : kind === 'object' ? 'objects' : 'sounds';
  const name = slugName(title, kind);
  return `assets/${folder}/${name}${n > 1 ? `-${n}` : ''}.${ext}`;
}

/** "assets → characters, named knight.glb". */
export function placeWords(path: string): string {
  const parts = path.split('/');
  const name = parts.pop() ?? path;
  return parts.length ? `${parts.join(' → ')}, named ${name}` : name;
}

/** A short name from an idea: its first phrase, capitalised. */
export function nameFromIdea(idea: string, fallback: string): string {
  const first = idea.trim().split(/[\n.!?,;]/)[0]?.trim() ?? '';
  const t = first.replace(/^(a|an|the)\s+/i, '');
  if (!t) return fallback;
  const c = t.charAt(0).toUpperCase() + t.slice(1);
  return c.length > 40 ? `${c.slice(0, 39).trimEnd()}…` : c;
}

/** What Otto Forge is asked to draw for a character: a full figure it can rig. */
export function characterPrompt(idea: string): string {
  return `${idea.trim().replace(/[.\s]+$/, '')}, one full-body character standing, arms a little away from the body`;
}

// ── Voices in plain words ───────────────────────────────────────────────────

export interface PlainVoice {
  id: 'deep' | 'cheerful' | 'soft' | 'rough';
  label: string;
  hint: string;
  /** Voice ids to try, in order (Otto Forge characters, Kokoro voices, macOS voices). */
  candidates: string[];
  speed: number;
}

export const PLAIN_VOICES: PlainVoice[] = [
  { id: 'deep', label: 'Deep and calm', hint: 'Like a wise guard', candidates: ['keeper', 'am_onyx', 'Reed', 'Daniel', 'Alex', 'Fred'], speed: 0.92 },
  { id: 'cheerful', label: 'Young and cheerful', hint: 'Quick and bright', candidates: ['scout', 'af_heart', 'Samantha', 'Karen'], speed: 1.06 },
  { id: 'soft', label: 'Soft and low', hint: 'Warm and quiet', candidates: ['ember', 'af_nicole', 'Moira', 'Tessa', 'Karen'], speed: 0.96 },
  { id: 'rough', label: 'Old storyteller', hint: 'Slow and scratchy', candidates: ['flint', 'bm_george', 'Grandpa', 'Ralph', 'Fred', 'Daniel'], speed: 0.9 },
];

/** The real voice id for a plain choice: the first candidate the voice list has, else its first voice. */
export function pickVoiceId(choice: PlainVoice, voices: Pick<TtsVoice, 'id' | 'name'>[]): string | null {
  for (const c of choice.candidates) {
    const hit = voices.find((v) => v.id === c || v.name === c || v.id.toLowerCase() === c.toLowerCase());
    if (hit) return hit.id;
  }
  return voices[0]?.id ?? null;
}

/** Example ideas under the Smart box: a short label, the full idea it fills in. */
export const SMART_EXAMPLES: { label: string; text: string }[] = [
  {
    label: 'A knight next to a chest of gold',
    text: 'A knight in blue armor standing next to a wooden chest full of gold coins. He says hello in a deep voice.',
  },
  { label: 'A girl with a bow and arrows', text: 'A woman with a bow and a bag of arrows who shoots at a target' },
  { label: 'A chest full of gold', text: 'A treasure chest full of gold coins' },
];

export const KIND_EXAMPLES: Record<'character' | 'object', string[]> = {
  character: ['A brave knight in blue armor', 'A friendly robot', 'A pirate with a big hat'],
  object: ['A treasure chest', 'A glowing sword', 'A wooden bridge'],
};

// ── Moves ─────────────────────────────────────────────────────────────────

const MOVE_WORDS: [RegExp, string][] = [
  [/idle|stand/, 'Stand'],
  [/walk|^100style-.*-fw$/, 'Walk'],
  [/run/, 'Run'],
  [/chop|attack|swing|slash/, 'Swing'],
  [/block|guard/, 'Block'],
  [/wave/, 'Wave'],
  [/jump/, 'Jump'],
];

/** A clip's plain name: `walk` → "Walk", `1h-melee-attack-chop` → "Swing", `100style-angry-fw` → "Angry walk". */
export function moveLabel(clip: string): string {
  const c = clip.toLowerCase();
  const style = /^100style-([a-z]+)-/.exec(c)?.[1];
  for (const [re, word] of MOVE_WORDS) {
    if (re.test(c)) return style ? `${style.charAt(0).toUpperCase()}${style.slice(1)} ${word.toLowerCase()}` : word;
  }
  const words = c.replace(/^100style-/, '').replace(/[_-]+/g, ' ').replace(/\b\d+h\b/g, '').trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : 'Move';
}

// ── Routes (`#/design/create/…`) ─────────────────────────────────────────

export type CreateRoute =
  | { view: 'home'; project?: string }
  | { view: 'production'; id: string; scene?: boolean }
  | { view: 'kinds' }
  | { view: 'describe'; kind: 'character' | 'object' }
  | { view: 'job'; id: string }
  | { view: 'result'; id: string }
  | { view: 'change'; id: string }
  | { view: 'compare'; id: string; before: number }
  | { view: 'voice'; who: string | null }
  | { view: 'sound' }
  | { view: 'mine' };

/** `parts` = the router parts after `design/create`. */
export function parseCreateRoute(parts: string[]): CreateRoute {
  const [a, b, c, d] = parts.map((p) => decodeURIComponent(p.split('?')[0]));
  switch (a) {
    case 'p':
      return b ? { view: 'production', id: b, ...(c === 'scene' ? { scene: true } : {}) } : { view: 'home' };
    case 'kinds':
      return { view: 'kinds' };
    case 'new':
      return b === 'character' || b === 'object' ? { view: 'describe', kind: b } : { view: 'kinds' };
    case 'job':
      return b ? { view: 'job', id: b } : { view: 'home' };
    case 'a':
      if (!b) return { view: 'mine' };
      if (c === 'change') return { view: 'change', id: b };
      if (c === 'compare' && Number(d) > 0) return { view: 'compare', id: b, before: Number(d) };
      return { view: 'result', id: b };
    case 'voice':
      return { view: 'voice', who: b || null };
    case 'sound':
      return { view: 'sound' };
    case 'mine':
      return { view: 'mine' };
    case 'in':
      return b ? { view: 'home', project: b } : { view: 'home' };
    default:
      return { view: 'home' };
  }
}

export const createHref = (...parts: string[]): string => ['design/create', ...parts.map(encodeURIComponent)].join('/');
