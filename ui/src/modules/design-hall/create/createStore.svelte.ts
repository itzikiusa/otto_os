// Create's shared state. Module-level on purpose: the work a person started
// keeps being watched while they browse elsewhere in the app, and every
// Create screen picks it up again (the jobs themselves run in the daemon).
//
// • engine  — the local making kit (Otto Forge): ready or not, and which 3D
//             provider Create uses. No provider list is ever shown.
// • jobs    — single-item 3D jobs, polled every 2 s while one runs (paused
//             while the window is hidden; the daemon keeps working).
// • productions — Smart plans, polled every second while planning/running
//             (several at once: each takes its turn in the daemon), with
//             one counting-down time left each and which finished ones were
//             opened (the rest show as "Ready to look" on the first page).
// • pending — what a job was for (kind, idea, where), kept in localStorage so
//             a reload still files the finished model with its idea.

import * as mapi from '../../../lib/api/media';
import * as papi from '../../../lib/api/productions';
import { updateArtifact } from '../../../lib/api/design';
import { loadErrorText } from '../../../lib/loadError';
import { pollWhileVisible, type Poller } from '../../../lib/poll';
import type { Media3dJob, Media3dProvider, Production, TtsProvider } from '../../../lib/api/types';
import type { CreateKind, UseWhere } from './plan';
import { nextEta, type Eta, type LiveProduction } from './progress';

export interface CreationMeta {
  kind: CreateKind;
  idea: string;
  where: UseWhere;
  picture?: boolean;
  /** A change: the version number shown before it, for "Keep new / Go back". */
  before?: number;
}

const PENDING_KEY = 'otto.create.pending';
const ACTIVE = new Set(['queued', 'running']);
const PROD_ACTIVE = new Set(['planning', 'running']);
const LOOKED_KEY = 'otto.create.looked';
/** A finished creation stays on the "In progress" strip as "Ready to look" for a day, until it is opened. */
const READY_FOR_MS = 86_400_000;

function readLooked(): Record<string, true> {
  try {
    const ids = JSON.parse(localStorage.getItem(LOOKED_KEY) ?? '[]') as string[];
    return Object.fromEntries(ids.map((id) => [id, true as const]));
  } catch {
    return {};
  }
}

function readPending(): Record<string, CreationMeta> {
  try {
    return JSON.parse(localStorage.getItem(PENDING_KEY) ?? '{}') as Record<string, CreationMeta>;
  } catch {
    return {};
  }
}
function writePending(p: Record<string, CreationMeta>): void {
  try {
    localStorage.setItem(PENDING_KEY, JSON.stringify(p));
  } catch {
    /* private mode */
  }
}

/** The 3D provider Create uses: the built-in Otto Forge when ready, else any ready provider that can do every stage. */
export function pickEngine(ps: Media3dProvider[]): Media3dProvider | null {
  const ok = (p: Media3dProvider) => p.enabled && p.ready && p.capabilities.supported_stages.includes('generate');
  const full = (p: Media3dProvider) => ['rig', 'animate'].every((s) => p.capabilities.supported_stages.includes(s as 'rig'));
  return ps.find((p) => p.id === 'otto-forge' && ok(p)) ?? ps.find((p) => ok(p) && full(p)) ?? null;
}

/** The voice maker: a ready local voice server (Otto Forge voices) first, else the built-in Mac voices. */
export function pickVoiceProvider(ps: TtsProvider[]): TtsProvider | null {
  const ok = (p: TtsProvider) => p.enabled && p.ready;
  return ps.find((p) => ok(p) && p.kind === 'kokoro') ?? ps.find((p) => ok(p) && p.kind !== 'macos_say') ?? ps.find(ok) ?? null;
}

/** A picture chosen on one screen, handed to the next (never persisted). */
export interface PickedPicture {
  b64: string;
  mime: string;
  name: string;
  url: string;
}

/** Read a picked file as base64 for a 3D job (≤ 12 MB, like the server). */
export async function readPicture(file: File): Promise<PickedPicture> {
  if (!/^image\/(png|jpeg|webp)$/.test(file.type)) throw new Error('Please use a photo or drawing (PNG, JPEG or WebP).');
  if (file.size > 12 * 1024 * 1024) throw new Error('That picture is too big. Please use one under 12 MB.');
  const buf = new Uint8Array(await file.arrayBuffer());
  let bin = '';
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return { b64: btoa(bin), mime: file.type, name: file.name, url: URL.createObjectURL(file) };
}

class CreateStore {
  picture = $state<PickedPicture | null>(null);
  engine = $state<{ loading: boolean; provider: Media3dProvider | null; error: string | null; checked: boolean }>({
    loading: false,
    provider: null,
    error: null,
    checked: false,
  });
  jobs = $state<Record<string, Media3dJob>>({});
  productions = $state<Record<string, LiveProduction>>({});
  /** Each production's one time left: the server's estimate, counted down between its changes. */
  eta = $state<Record<string, Eta>>({});
  /** Finished creations already opened (they leave the "In progress" strip). */
  looked = $state<Record<string, true>>(readLooked());
  /** Notes written on a piece while the rest was still being made, offered on the scene when it is done. */
  laterNotes = $state<Record<string, string[]>>({});
  /** The sound and music maker: ready, missing, or not asked yet (null). */
  soundReady = $state<boolean | null>(null);
  /** The notes pinned with the last change asked for, per production or thing (shown in "Here's what I'll change"). */
  lastNotes = $state<Record<string, string[]>>({});
  private jobPoll: Poller | null = null;
  private prodPoll: Poller | null = null;

  async loadSoundEngine(): Promise<void> {
    try {
      this.soundReady = (await mapi.getAudioEngine()).state === 'ready';
    } catch {
      this.soundReady = false;
    }
  }

  async loadEngine(force = false): Promise<void> {
    if (this.engine.loading || (this.engine.checked && !force)) return;
    this.engine.loading = true;
    this.engine.error = null;
    try {
      this.engine.provider = pickEngine(await mapi.list3dProviders());
    } catch (e) {
      this.engine.error = loadErrorText(e);
    } finally {
      this.engine.loading = false;
      this.engine.checked = true;
    }
  }

  /** Pick up the workspace's running jobs (a reload, another window). */
  async resume(workspaceId: string): Promise<void> {
    try {
      for (const j of await mapi.list3dJobs({ workspace_id: workspaceId, limit: 20 })) {
        if (ACTIVE.has(j.status)) this.track(j);
      }
    } catch {
      /* the screens show their own load errors */
    }
  }

  meta(jobId: string): CreationMeta | null {
    return readPending()[jobId] ?? null;
  }

  async start(body: Parameters<typeof mapi.start3dJob>[0], meta: CreationMeta | null): Promise<Media3dJob> {
    const job = await mapi.start3dJob(body);
    if (meta) writePending({ ...readPending(), [job.id]: meta });
    this.track(job);
    return job;
  }

  async stop(jobId: string): Promise<void> {
    this.jobs[jobId] = await mapi.cancel3dJob(jobId);
  }

  track(job: Media3dJob): void {
    this.jobs[job.id] = job;
    this.jobPoll ??= pollWhileVisible(() => this.pollJobs(), { ms: 2000, immediate: false });
  }

  async refreshJob(jobId: string): Promise<Media3dJob> {
    const j = await mapi.get3dJob(jobId);
    this.jobs[jobId] = j;
    if (ACTIVE.has(j.status)) this.track(j);
    else await this.settle(j);
    return j;
  }

  private async pollJobs(): Promise<void> {
    const live = Object.values(this.jobs).filter((j) => ACTIVE.has(j.status));
    if (!live.length) {
      this.jobPoll?.stop();
      this.jobPoll = null;
      return;
    }
    await Promise.all(live.map((j) => this.refreshJob(j.id).catch(() => undefined)));
  }

  /** A finished job files its model with the idea it came from (artifact meta `create`). */
  private async settle(j: Media3dJob): Promise<void> {
    const pending = readPending();
    const meta = pending[j.id];
    if (!meta) return;
    delete pending[j.id];
    writePending(pending);
    if (j.status === 'succeeded' && j.artifact_id && !j.target_artifact_id && meta.before === undefined) {
      await updateArtifact(j.artifact_id, { meta: { create: meta } }).catch(() => undefined);
    }
  }

  // ── Smart productions ─────────────────────────────────────────────────────

  watchProduction(p: Production): void {
    this.keep(p);
    this.prodPoll ??= pollWhileVisible(() => this.pollProductions(), { ms: 1000, immediate: false });
  }

  async refreshProduction(id: string): Promise<Production> {
    const p = await papi.getProduction(id);
    this.keep(p);
    if (PROD_ACTIVE.has(p.status)) this.watchProduction(p);
    return p;
  }

  private keep(p: LiveProduction): void {
    // It counts down only while it is being made (not planned, not waiting for its turn), from when that started.
    const making = (x: LiveProduction | undefined) => x?.status === 'running' && !(x.queue_position ?? 0);
    const was = making(this.productions[p.id]);
    this.productions[p.id] = p;
    if (typeof p.eta_secs === 'number') this.eta[p.id] = nextEta(was && making(p) ? (this.eta[p.id] ?? null) : null, p.eta_secs, Date.now());
  }

  /** The creations to show under "In progress" (every workspace): planning, waiting or making, and finished ones not opened yet. */
  inProgress(): LiveProduction[] {
    return Object.values(this.productions)
      .filter((p) => this.wanted(p))
      .sort((a, b) => a.created_at.localeCompare(b.created_at));
  }

  private wanted(p: Pick<Production, 'id' | 'status' | 'updated_at'>): boolean {
    return PROD_ACTIVE.has(p.status) || (p.status === 'done' && !this.looked[p.id] && Date.now() - Date.parse(p.updated_at) < READY_FOR_MS);
  }

  /** Pick up the creations in progress (a reload, another window, another workspace). */
  async loadInProgress(): Promise<void> {
    const shelf = await papi.listCreations().catch(() => null);
    await Promise.all((shelf?.productions ?? []).filter((s) => this.wanted(s)).map((s) => this.refreshProduction(s.id).catch(() => undefined)));
  }

  markLooked(id: string): void {
    if (this.looked[id]) return;
    this.looked[id] = true;
    try {
      localStorage.setItem(LOOKED_KEY, JSON.stringify(Object.keys(this.looked).slice(-200)));
    } catch {
      /* private mode */
    }
  }

  private async pollProductions(): Promise<void> {
    const live = Object.values(this.productions).filter((p) => PROD_ACTIVE.has(p.status));
    if (!live.length) {
      this.prodPoll?.stop();
      this.prodPoll = null;
      return;
    }
    await Promise.all(live.map((p) => this.refreshProduction(p.id).catch(() => undefined)));
  }
}

export const createStore = new CreateStore();
