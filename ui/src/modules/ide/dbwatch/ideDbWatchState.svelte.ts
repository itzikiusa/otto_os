// The IDE DB Watch store: a thin client of the daemon's watch engine
// (/ide/db-watch/watches — the daemon keeps the checkpoints and takes one at
// every debugger stop). It also arms the watch on its own, so it works while
// the Debug tab is the one showing: `follow()` (called by the bottom dock,
// which is always mounted) re-attaches the project's watch, binds it to each
// new debug session / run, and at the first stop arms the remembered tables —
// or, the first time, the run's own Testcontainer with the tables the code
// touches. Never a saved connection nobody picked, never a prod one unasked.
// The agent-debug `db` command drives the same actions (prod confirm included).

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { appLive } from '../../../lib/live';
import { loadErrorText } from '../../../lib/loadError';
import { lsGet, lsSet } from '../../../lib/storage';
import { onIdeTargetReset } from '../store/ideStateSlices.svelte';
import type {
  IdeDbWatch,
  IdeDbWatchBind,
  IdeDbWatchCompare,
  IdeDbWatchDetectReq,
  IdeDbWatchDetectResp,
  IdeDbWatchItemSpec,
  IdeDbWatchListResp,
  IdeDbWatchObject,
  IdeDbWatchObjectsResp,
  IdeDbWatchReadResp,
  IdeDbWatchStartReq,
  IdeDbWatchTarget,
  IdeDbWatchTargetRef,
  IdeDetectedDbAccess,
  IdeEphemeralDbTarget,
  IdeSavedDbTarget,
} from '../../../lib/api/types/ide-db-watch';
import {
  changedRowsOf,
  DB_WATCH_OFF,
  isProdTarget,
  MAX_WATCH_ITEMS,
  memoryKey,
  memoryOf,
  parseMemory,
  pickAutoItems,
  pickAutoTarget,
  targetRefOf,
  type DbWatchMemory,
} from './ideDbWatchModel';

const W = '/ide/db-watch/watches';

/** What the dock hands over on every change. */
export interface DbWatchFollow {
  root: string;
  activeFile?: string | null;
  additionalRoots?: string[];
  debug?: { id: string; state: string; stop_seq?: number } | null;
  run?: { id: string; status: string } | null;
}

export interface DbWatchStartOpts {
  /** Armed by Otto itself: never confirms, so never touches a prod target. */
  auto?: boolean;
  /** Who asks, for the prod confirm ("Agent Messi"); default the person. */
  asker?: string;
}

function sameBind(a: IdeDbWatchBind | null | undefined, b: IdeDbWatchBind | null | undefined): boolean {
  if (!a || !b) return false;
  return 'debug_session_id' in a && 'debug_session_id' in b
    ? a.debug_session_id === b.debug_session_id
    : 'run_id' in a && 'run_id' in b && a.run_id === b.run_id;
}

function refOfWatch(w: IdeDbWatch): IdeDbWatchTargetRef {
  return w.target.kind === 'saved'
    ? { kind: 'saved', connection_id: w.target.connection_id ?? '' }
    : { kind: 'ephemeral', ephemeral_id: w.target.ephemeral_id ?? '' };
}

class IdeDbWatchController {
  root = $state('');
  watch = $state<IdeDbWatch | null>(null);
  read = $state<IdeDbWatchReadResp | null>(null);
  compare = $state<IdeDbWatchCompare>('previous_stop');
  /** A start / first read with nothing on screen yet. */
  loading = $state(false);
  /** A read / snapshot in flight while the last result stays on screen. */
  refreshing = $state(false);
  /** The watch or its target failed (start, read): shown inline with Retry. */
  error = $state<string | null>(null);

  detected = $state<IdeDetectedDbAccess[]>([]);
  savedConnections = $state<IdeSavedDbTarget[]>([]);
  ephemeralTargets = $state<IdeEphemeralDbTarget[]>([]);
  detectLoading = $state(false);
  detectError = $state<string | null>(null);
  /** The target's real objects ("Add table…"), for `objectsFor`. */
  objects = $state<IdeDbWatchObject[]>([]);
  objectsFor = $state('');
  /** Picked in Target ▾ before any table: the "Choose tables" state. */
  pendingTarget = $state<IdeDbWatchTarget | null>(null);
  /** The strip asked to show this table (the panel scrolls to it). */
  focusObject = $state('');
  liveWatch = $state(false);

  private gen = 0;
  private readSeq = 0;
  private restoring: Promise<void> = Promise.resolve();
  /** Starts run one at a time: the agent's and the first stop's auto-arm can race. */
  private starting: Promise<unknown> = Promise.resolve();
  private activeFile: string | null = null;
  private additionalRoots: string[] = [];
  /** The live debug session / run this root is following, and the ones armed. */
  private liveBind: IdeDbWatchBind | null = null;
  private seenDebug = '';
  private stoppedDebug = '';
  private seenRun = '';

  constructor() {
    onIdeTargetReset(() => this.reset());
    appLive.on(['ide_db_watch_checkpoint'], (ev) => {
      if (this.watch && ev.watch_id === this.watch.id) void this.refresh();
    });
    appLive.onResync(() => {
      if (this.watch) void this.refresh();
    });
  }

  reset(): void {
    this.gen += 1;
    this.readSeq += 1;
    this.root = '';
    this.watch = null;
    this.read = null;
    this.compare = 'previous_stop';
    this.loading = false;
    this.refreshing = false;
    this.error = null;
    this.detected = [];
    this.ephemeralTargets = [];
    this.detectLoading = false;
    this.detectError = null;
    this.objects = [];
    this.objectsFor = '';
    this.pendingTarget = null;
    this.focusObject = '';
    this.liveWatch = false;
    this.liveBind = null;
    this.seenDebug = '';
    this.stoppedDebug = '';
    this.seenRun = '';
    this.restoring = Promise.resolve();
  }

  /** Changed rows in what the panel shows (the dock tab badge). */
  get changeBadgeCount(): number | undefined {
    const n = (this.read?.items ?? []).reduce((s, i) => s + changedRowsOf(i), 0);
    return n > 0 ? n : undefined;
  }

  get isProd(): boolean {
    return isProdTarget(this.watch?.target);
  }

  get allTargets(): IdeDbWatchTarget[] {
    return [...this.ephemeralTargets, ...this.savedConnections];
  }

  // ── Following the IDE (store-level auto-arm) ─────────────────────────────

  follow(ctx: DbWatchFollow): void {
    this.activeFile = ctx.activeFile ?? null;
    this.additionalRoots = ctx.additionalRoots ?? [];
    if (ctx.root !== this.root) {
      this.reset();
      this.root = ctx.root;
      if (ctx.root) this.restoring = this.restore(ctx.root);
    }
    if (!ctx.root) return;
    const d = ctx.debug;
    const live = d && d.state !== 'terminated' && d.state !== 'error' ? d : null;
    const r = ctx.run?.status === 'running' ? ctx.run : null;
    this.liveBind = live ? { debug_session_id: live.id } : r ? { run_id: r.id } : null;
    if (live && live.id !== this.seenDebug) {
      this.seenDebug = live.id;
      void this.onSessionStart({ debug_session_id: live.id });
    }
    // The first stop: a Testcontainer started in TestMain exists by now.
    if (live && live.id !== this.stoppedDebug && (live.state === 'paused' || (live.stop_seq ?? 0) > 0)) {
      this.stoppedDebug = live.id;
      void this.armAtStop({ debug_session_id: live.id });
    }
    if (r && r.id !== this.seenRun) {
      this.seenRun = r.id;
      void this.onSessionStart({ run_id: r.id }).then(() => this.armAtStop({ run_id: r.id }));
    }
  }

  private memory(): DbWatchMemory | null {
    return this.root ? parseMemory(lsGet(memoryKey(this.root))) : null;
  }

  private remember(w: IdeDbWatch): void {
    const m = memoryOf(w);
    if (m) lsSet(memoryKey(w.root), JSON.stringify(m));
  }

  private async restore(root: string): Promise<void> {
    const gen = this.gen;
    try {
      const resp = await api.get<IdeDbWatchListResp>(`${W}?root=${encodeURIComponent(root)}`);
      if (gen !== this.gen) return;
      const w = (resp.watches ?? []).find((x) => x.root === root) ?? null;
      if (w && !this.watch) {
        this.watch = w;
        await this.refresh();
      }
    } catch {
      /* the panel's actions show their own errors; nothing to restore */
    }
  }

  /** A new debug session / run: bind a saved-target watch now; arm a remembered
   *  saved target (never a prod one) right away. */
  private async onSessionStart(bind: IdeDbWatchBind): Promise<void> {
    await this.restoring;
    const w = this.watch;
    if (w) {
      if (w.target.kind === 'saved' && !sameBind(w.bind, bind)) await this.bind(bind);
      return;
    }
    const mem = this.memory();
    if (mem?.target.kind !== 'saved') return;
    const id = mem.target.connection_id;
    await this.discover();
    const conn = this.savedConnections.find((c) => c.connection_id === id);
    if (!conn || isProdTarget(conn)) return;
    await this.start({ target: mem.target, database: mem.database, items: mem.items, bind }, { auto: true });
  }

  /** The first stop (or a run's start): arm on the run's own database. */
  private async armAtStop(bind: IdeDbWatchBind): Promise<void> {
    await this.restoring;
    const w = this.watch;
    if (w && (w.target.kind === 'saved' || sameBind(w.bind, bind))) return;
    const mem = this.memory();
    if (mem?.target.kind === 'saved' || lsGet(memoryKey(this.root)) === DB_WATCH_OFF) return;
    if (mem) {
      await this.start({ target: mem.target, database: mem.database, items: mem.items, bind }, { auto: true });
      return;
    }
    await this.discover({ debugSessionId: 'debug_session_id' in bind ? bind.debug_session_id : null });
    const t = pickAutoTarget(this.detected, this.ephemeralTargets);
    if (!t) return;
    await this.discover({ debugSessionId: 'debug_session_id' in bind ? bind.debug_session_id : null, target: t });
    const items = pickAutoItems(this.detected, t.engine, true);
    if (items.length === 0) {
      this.pendingTarget = t;
      return;
    }
    await this.start({ target: targetRefOf(t), database: t.database, items, bind }, { auto: true });
  }

  // ── Discovery ────────────────────────────────────────────────────────────

  /** Targets + ranked objects in code; with `target`, each object is verified
   *  against that target's real tables. */
  async discover(opts: { debugSessionId?: string | null; target?: IdeDbWatchTarget | IdeDbWatchTargetRef | null } = {}): Promise<void> {
    if (!this.root) return;
    const gen = this.gen;
    this.detectLoading = true;
    this.detectError = null;
    const t = opts.target;
    const target = t ? ('target_kind' in t ? targetRefOf(t) : t) : this.watch ? refOfWatch(this.watch) : null;
    try {
      const req: IdeDbWatchDetectReq = {
        root: this.root,
        active_file: this.activeFile,
        additional_roots: this.additionalRoots,
        target,
        database: this.watch?.database ?? null,
        debug_session_id: opts.debugSessionId ?? (this.liveBind && 'debug_session_id' in this.liveBind ? this.liveBind.debug_session_id : null),
      };
      const resp = await api.post<IdeDbWatchDetectResp>('/ide/db-watch/detect', req);
      if (gen !== this.gen) return;
      this.detected = resp.detected_objects ?? [];
      this.savedConnections = resp.saved_connections ?? [];
      this.ephemeralTargets = resp.ephemeral_targets ?? [];
    } catch (e) {
      if (gen === this.gen) this.detectError = loadErrorText(e);
    } finally {
      if (gen === this.gen) this.detectLoading = false;
    }
  }

  /** The target's real tables / collections / key prefixes ("Add table…"). */
  async loadObjects(): Promise<void> {
    const ref = this.watch ? refOfWatch(this.watch) : this.pendingTarget ? targetRefOf(this.pendingTarget) : null;
    if (!this.root || !ref) return;
    const key = JSON.stringify(ref);
    const gen = this.gen;
    try {
      const resp = await api.post<IdeDbWatchObjectsResp>('/ide/db-watch/objects', {
        root: this.root,
        target: ref,
        database: this.watch?.database ?? this.pendingTarget?.database ?? null,
      });
      if (gen !== this.gen) return;
      this.objects = resp.objects ?? [];
      this.objectsFor = key;
    } catch (e) {
      if (gen === this.gen) this.detectError = loadErrorText(e);
    }
  }

  // ── Watch actions (the panel's, the strip's and the agent bridge's) ──────

  private targetOf(ref: IdeDbWatchTargetRef): IdeDbWatchTarget | null {
    if (ref.kind === 'saved') return this.savedConnections.find((c) => c.connection_id === ref.connection_id) ?? null;
    if (ref.kind === 'ephemeral') return this.ephemeralTargets.find((c) => c.ephemeral_id === ref.ephemeral_id) ?? null;
    return null;
  }

  /** The typed prod confirm: what runs, where, and who asks. Never remembered. */
  private async confirmProd(name: string, items: readonly IdeDbWatchItemSpec[], asker: string): Promise<boolean> {
    const tables = items.map((i) => i.object).join(', ');
    const message = [
      'PRODUCTION — read-only, but it runs on live data.',
      `Asked by: ${asker}`,
      `Where: ${name}`,
      `What: up to ${items.length} SELECTs of at most 150 rows (${tables}) at the start, at every debugger stop and when you press Snapshot.`,
      `Type “${name}” to confirm.`,
    ].join('\n\n');
    const v = await confirmer.promptText(message, {
      title: 'Watch a production database?',
      confirmLabel: 'Watch',
      placeholder: name,
      mustMatch: name,
      danger: true,
    });
    return v === name;
  }

  start(req: Omit<IdeDbWatchStartReq, 'root'> & { root?: string }, opts: DbWatchStartOpts = {}): Promise<IdeDbWatch | null> {
    const turn = this.starting.then(() => this.startNow(req, opts));
    this.starting = turn.catch(() => null);
    return turn;
  }

  private async startNow(req: Omit<IdeDbWatchStartReq, 'root'> & { root?: string }, opts: DbWatchStartOpts): Promise<IdeDbWatch | null> {
    const root = req.root ?? this.root;
    if (!root || req.items.length === 0) return null;
    // The auto-arm never replaces a watch the agent (or the person) armed for this run.
    if (opts.auto && this.watch && sameBind(this.watch.bind, req.bind)) return this.watch;
    const gen = this.gen;
    const body: IdeDbWatchStartReq = {
      ...req,
      root,
      items: req.items.slice(0, MAX_WATCH_ITEMS),
      bind: req.bind === undefined ? this.liveBind : req.bind,
    };
    // An agent can name a saved connection the panel never listed: look it up, so prod still confirms.
    if (req.target.kind === 'saved' && !this.targetOf(req.target)) await this.discover();
    const conn = req.target.kind === 'saved' ? this.targetOf(req.target) : null;
    if (conn && isProdTarget(conn)) {
      if (opts.auto) return null;
      if (!(await this.confirmProd(conn.name, body.items, opts.asker ?? 'You'))) return null;
      body.confirm_production = true;
    }
    if (!this.watch) this.loading = true;
    this.error = null;
    try {
      const w = await api.post<IdeDbWatch>(W, body);
      if (gen !== this.gen) return null;
      this.watch = w;
      this.read = null;
      this.pendingTarget = null;
      this.remember(w);
      await this.refresh();
      return w;
    } catch (e) {
      if (gen === this.gen) this.error = loadErrorText(e);
      return null;
    } finally {
      if (gen === this.gen) this.loading = false;
    }
  }

  /** Watch the same tables on another target; with no watch yet, the target
   *  waits for its first table. */
  async setTarget(t: IdeDbWatchTargetRef | IdeDbWatchTarget, opts: DbWatchStartOpts = {}): Promise<void> {
    const ref = 'target_kind' in t ? targetRefOf(t) : t;
    const known = 'target_kind' in t ? t : this.targetOf(ref);
    const items = this.watch?.items.map((i) => ({ object: i.object, filter: i.filter ?? null })) ?? [];
    if (items.length === 0) {
      this.pendingTarget = known;
      this.error = null;
      await this.discover({ target: ref });
      return;
    }
    await this.start({ target: ref, database: known?.database ?? null, items }, opts);
  }

  private async patch(body: { items?: IdeDbWatchItemSpec[]; bind?: IdeDbWatchBind }, opts: DbWatchStartOpts = {}): Promise<boolean> {
    const w = this.watch;
    if (!w) return false;
    const gen = this.gen;
    const req: Record<string, unknown> = { ...body };
    if (body.items && isProdTarget(w.target)) {
      if (!(await this.confirmProd(w.target.label, body.items, opts.asker ?? 'You'))) return false;
      req.confirm_production = true;
    }
    this.refreshing = true;
    try {
      const next = await api.patch<IdeDbWatch>(`${W}/${encodeURIComponent(w.id)}`, req);
      if (gen !== this.gen) return false;
      this.watch = next;
      if (body.items) this.remember(next);
      await this.refresh();
      return true;
    } catch (e) {
      if (gen === this.gen) this.error = loadErrorText(e);
      return false;
    } finally {
      if (gen === this.gen) this.refreshing = false;
    }
  }

  private specs(): IdeDbWatchItemSpec[] {
    return (this.watch?.items ?? []).map((i) => (i.filter ? { object: i.object, filter: i.filter } : { object: i.object }));
  }

  async addItem(item: IdeDbWatchItemSpec, opts: DbWatchStartOpts = {}): Promise<void> {
    if (!this.watch) {
      const t = this.pendingTarget;
      if (t) await this.start({ target: targetRefOf(t), database: t.database ?? null, items: [item] }, opts);
      return;
    }
    const items = this.specs().filter((i) => i.object !== item.object);
    if (items.length >= MAX_WATCH_ITEMS) return;
    await this.patch({ items: [...items, item] }, opts);
  }

  /** Change one item in place (a filter, a Redis key pattern). */
  async replaceItem(object: string, next: IdeDbWatchItemSpec, opts: DbWatchStartOpts = {}): Promise<void> {
    await this.patch({ items: this.specs().map((i) => (i.object === object ? next : i)) }, opts);
  }

  async removeItem(object: string): Promise<void> {
    const w = this.watch;
    if (!w) return;
    const items = this.specs().filter((i) => i.object !== object);
    if (items.length > 0) {
      await this.patch({ items });
      return;
    }
    const target = this.targetOf(refOfWatch(w));
    await this.stop();
    this.pendingTarget = target;
  }

  async bind(b: IdeDbWatchBind): Promise<void> {
    await this.patch({ bind: b });
  }

  async snapshotNow(label?: string | null): Promise<IdeDbWatchReadResp | null> {
    const w = this.watch;
    if (!w) return null;
    const gen = this.gen;
    const seq = ++this.readSeq;
    this.refreshing = true;
    try {
      const resp = await api.post<IdeDbWatchReadResp>(`${W}/${encodeURIComponent(w.id)}/snapshot`, { label: label ?? null, compare: this.compare });
      if (gen !== this.gen || seq !== this.readSeq) return resp;
      this.read = resp;
      this.watch = resp.watch;
      this.error = null;
      return resp;
    } catch (e) {
      if (gen === this.gen && seq === this.readSeq) this.error = loadErrorText(e);
      return null;
    } finally {
      if (gen === this.gen && seq === this.readSeq) this.refreshing = false;
    }
  }

  /** The diff since the previous stop or the start. `forAgent` asks the daemon
   *  to mask (the agent-debug `db` command always passes it) and never touches
   *  what the panel shows. */
  async diffSince(compare: IdeDbWatchCompare, opts: { forAgent?: boolean; rows?: boolean } = {}): Promise<IdeDbWatchReadResp | null> {
    const w = this.watch;
    if (!w) return null;
    const q = new URLSearchParams({ compare });
    if (opts.rows) q.set('rows', '1');
    if (opts.forAgent) q.set('for_agent', '1');
    const resp = await api.get<IdeDbWatchReadResp>(`${W}/${encodeURIComponent(w.id)}?${q}`);
    if (!opts.forAgent && !opts.rows && compare === this.compare && this.watch?.id === w.id) {
      this.read = resp;
      this.watch = resp.watch;
    }
    return resp;
  }

  async setCompare(c: IdeDbWatchCompare): Promise<void> {
    if (this.compare === c) return;
    this.compare = c;
    await this.refresh();
  }

  /** Re-read the panel's view (a checkpoint event, a compare switch, Retry). */
  async refresh(): Promise<void> {
    const w = this.watch;
    if (!w) return;
    const gen = this.gen;
    const seq = ++this.readSeq;
    if (this.read) this.refreshing = true;
    else this.loading = true;
    try {
      const resp = await api.get<IdeDbWatchReadResp>(`${W}/${encodeURIComponent(w.id)}?compare=${this.compare}`);
      if (gen !== this.gen || seq !== this.readSeq) return;
      this.read = resp;
      this.watch = resp.watch;
      this.error = null;
    } catch (e) {
      if (gen !== this.gen || seq !== this.readSeq) return;
      // The daemon dropped it (ended session + 10 min, or replaced): forget it.
      if ((e as { status?: number })?.status === 404) {
        this.watch = null;
        this.read = null;
      } else {
        this.error = loadErrorText(e);
      }
    } finally {
      if (gen === this.gen && seq === this.readSeq) {
        this.refreshing = false;
        this.loading = false;
      }
    }
  }

  /** Stop watching: the daemon drops it and this project stops re-arming it. */
  async stop(): Promise<void> {
    const w = this.watch;
    if (!w) return;
    const gen = this.gen;
    try {
      await api.del(`${W}/${encodeURIComponent(w.id)}`);
    } catch (e) {
      if (gen === this.gen) this.error = loadErrorText(e);
      return;
    }
    if (gen !== this.gen) return;
    lsSet(memoryKey(w.root), DB_WATCH_OFF);
    this.watch = null;
    this.read = null;
    this.liveWatch = false;
  }
}

export const ideDbWatch = new IdeDbWatchController();
