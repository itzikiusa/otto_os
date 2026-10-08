// Otto Assistant store: threads + each open thread's turn index, the
// needs-you queue (drives the sidebar badge), tasks, memory, routing and
// limits, with live WS application. Like the other page stores it does NOT
// import events.svelte.ts — the dispatcher calls `assistant.applyEvent(...)`.
//
// Stale-response guards: every loader takes a per-key ticket and drops its
// result if a newer load for the same key started meanwhile; live rows only
// replace what they are newer than (`upsertNewer`, `reduceNeedsYou`); and the
// needs-you `open_count` on each frame re-syncs the queue when we drifted.
//
// Loaders are called from components' $effects, so every synchronous READ of
// store state inside them is untracked — otherwise the effect would depend on
// the very state the loader writes and loop.
import { untrack } from 'svelte';
import { api, ApiError } from '../api/client';
import { mapLimit } from '../poll';
import type { ProviderUsage, UsageSummary } from '../api/usage.svelte';
import { assistantApi } from '../api/assistant';
import { needsYouByThread, reduceNeedsYou, upsertNewer, type NeedsYouAction, type NeedsYouState } from '../../modules/assistant/model';
import type {
  AssistantAttachment,
  AssistantDecisionReq,
  AssistantHermesPreview,
  AssistantLimitState,
  AssistantMemoryView,
  AssistantRoutingSettings,
  AssistantTask,
  AssistantTaskAction,
  AssistantThread,
  AssistantTurn,
  CreateAssistantThreadReq,
  OttoEvent,
  ProviderAccount,
} from '../api/types';

/** `unsupported` = the daemon has no such route yet (404/405/501): the UI says so honestly. */
export type LoadState = 'idle' | 'loading' | 'ready' | 'error' | 'unsupported';

export interface Loadable<T> {
  state: LoadState;
  data: T;
  error: string;
}

type AssistantEvent = Extract<OttoEvent, { type: `assistant_${string}` }>;

function initial<T>(data: T): Loadable<T> {
  return { state: 'idle', data, error: '' };
}

function isMissingRoute(e: unknown): boolean {
  return e instanceof ApiError && (e.status === 404 || e.status === 405 || e.status === 501);
}

/** A human cause for a failed call (content.md §4) — never the raw exception as the headline. */
export function describeError(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 403) return 'You don’t have access to the assistant.';
    if (e.status === 404) return 'It no longer exists.';
    if (e.status === 409) return e.message || 'It changed meanwhile. Refresh and try again.';
    return e.message || `The daemon answered ${e.status}.`;
  }
  if (e instanceof TypeError) return 'Otto can’t reach the daemon.';
  return e instanceof Error ? e.message : String(e);
}

/** An optimistic bubble for a turn this window sent that the daemon hasn't echoed yet. */
export interface PendingTurn {
  id: string;
  text: string;
  attachments: AssistantAttachment[];
  created_at: string;
}

/** How long a provider-account sign-in check result is reused (SI-03). */
const SIGNIN_TTL_MS = 5 * 60_000;
/** Concurrent sign-in checks: leaves ≥ 4 of the webview's 6 sockets free. */
const SIGNIN_CONCURRENCY = 2;

class AssistantStore {
  threads: Loadable<AssistantThread[]> = $state(initial([]));
  threadsHasMore = $state(false);
  threadsMoreLoading = $state(false);
  threadsMoreError = $state('');
  private threadsOffset = 0;
  private threadsGeneration = 0;
  selectedThread: Loadable<AssistantThread | null> = $state(initial(null));
  selectedThreadId = $state<string | null>(null);
  /** thread_id → its turn index (oldest first). `$state.raw` (A6): turns are
   *  replaced, never mutated, so deep proxies over every turn's text and
   *  blocks bought nothing but proxy cost on each read. Write through
   *  `setTurns`, which reassigns the map (a raw object's properties are not
   *  reactive). */
  turns: Record<string, Loadable<AssistantTurn[]>> = $state.raw({});
  pending: Record<string, PendingTurn[]> = $state({});
  needs: NeedsYouState = $state({ items: [], seen: {} });
  needsState: LoadState = $state('idle');
  tasks: Loadable<AssistantTask[]> = $state(initial([]));
  memory: Loadable<AssistantMemoryView | null> = $state(initial(null));
  hermes: Loadable<AssistantHermesPreview | null> = $state(initial(null));
  routing: Loadable<AssistantRoutingSettings | null> = $state(initial(null));
  limits: Loadable<AssistantLimitState[]> = $state(initial([]));
  /** This week's tokens per provider (`/usage/summary`, root-only → 'unsupported' for members). */
  usageWeek: Loadable<ProviderUsage[]> = $state(initial([]));
  /** Named subscription accounts (`/auth/provider-accounts`) + their sign-in state. */
  accounts: Loadable<(ProviderAccount & { signed_in: boolean | null })[]> = $state(initial([]));

  private tickets: Record<string, number> = {};
  private turnReaders = new Map<string, number>();
  private turnRequests = new Map<string, AbortController>();
  private serial = 0;
  private ticket(key: string): () => boolean {
    const mine = (this.tickets[key] = ++this.serial);
    return () => this.tickets[key] === mine;
  }

  get needsYouCount(): number {
    return this.needs.items.length;
  }
  get needsByThread(): Record<string, number> {
    return needsYouByThread(this.needs.items);
  }
  thread(id: string | null): AssistantThread | undefined {
    return id ? this.threads.data.find((t) => t.id === id) ?? (this.selectedThread.data?.id === id ? this.selectedThread.data : undefined) : undefined;
  }
  /** The freshest row we have for a task (queue, board, or a one-off fetch). */
  task(id: string): AssistantTask | undefined {
    const a = this.needs.items.find((t) => t.id === id);
    const b = this.tasks.data.find((t) => t.id === id);
    if (a && b) return a.updated_at >= b.updated_at ? a : b;
    return a ?? b;
  }
  limitFor(provider: string): AssistantLimitState | undefined {
    return this.limits.data.find((l) => l.provider === provider && l.limited);
  }

  // ── loaders ────────────────────────────────────────────────────────────────

  async loadThreads(): Promise<void> {
    const generation = ++this.threadsGeneration;
    this.threadsMoreLoading = false;
    this.threadsMoreError = '';
    await this.load('threads', async () => {
      const rows = await assistantApi.threads(0, 100);
      if (generation === this.threadsGeneration) {
        this.threadsOffset = rows.length;
        this.threadsHasMore = rows.length === 100;
      }
      return rows;
    }, (v) => (this.threads = v), () => this.threads);
  }

  async loadMoreThreads(): Promise<void> {
    if (this.threadsMoreLoading || !this.threadsHasMore) return;
    const generation = this.threadsGeneration;
    this.threadsMoreLoading = true;
    this.threadsMoreError = '';
    try {
      const rows = await assistantApi.threads(this.threadsOffset, 100);
      if (generation !== this.threadsGeneration) return;
      const ids = new Set(this.threads.data.map((t) => t.id));
      this.threads = { ...this.threads, data: [...this.threads.data, ...rows.filter((t) => !ids.has(t.id))] };
      this.threadsOffset += rows.length;
      this.threadsHasMore = rows.length === 100;
    } catch (e) {
      if (generation === this.threadsGeneration) this.threadsMoreError = describeError(e);
    } finally {
      if (generation === this.threadsGeneration) this.threadsMoreLoading = false;
    }
  }

  /** Keep one selected row independently of history pages, including deep links. */
  async loadSelectedThread(id: string): Promise<void> {
    const current = this.ticket('selectedThread');
    this.selectedThreadId = id;
    const cached = untrack(() => this.thread(id));
    this.selectedThread = { state: cached ? 'ready' : 'loading', data: cached ?? null, error: '' };
    try {
      const row = await assistantApi.thread(id);
      if (current()) this.selectedThread = { state: 'ready', data: row, error: '' };
    } catch (e) {
      if (current()) this.selectedThread = { state: 'error', data: cached ?? null, error: describeError(e) };
    }
  }

  async loadTurns(threadId: string): Promise<void> {
    if (!this.turnReaders.has(threadId)) return;
    this.turnRequests.get(threadId)?.abort();
    const request = new AbortController();
    this.turnRequests.set(threadId, request);
    const current = this.ticket(`turns:${threadId}`);
    const have = untrack(() => this.turns[threadId]);
    this.setTurns(threadId, { state: have?.state === 'ready' ? 'ready' : 'loading', data: have?.data ?? [], error: '' });
    try {
      const data = await assistantApi.turns(threadId, undefined, 200, request.signal);
      if (!current()) return;
      // Keep live-appended rows the GET raced past.
      const ids = new Set(data.map((t) => t.id));
      const extra = (this.turns[threadId]?.data ?? []).filter((t) => !ids.has(t.id) && t.created_at > (data.at(-1)?.created_at ?? ''));
      this.setTurns(threadId, { state: 'ready', data: [...data, ...extra], error: '' });
      this.settlePending(threadId, data);
    } catch (e) {
      if (!current()) return;
      this.setTurns(threadId, { state: 'error', data: have?.data ?? [], error: describeError(e) });
    } finally {
      if (this.turnRequests.get(threadId) === request) this.turnRequests.delete(threadId);
    }
  }

  /** Turn payload belongs to mounted views; pending sends live separately. */
  acquireTurns(threadId: string): () => void {
    const readers = this.turnReaders.get(threadId) ?? 0;
    this.turnReaders.set(threadId, readers + 1);
    if (!readers) void this.loadTurns(threadId);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const remaining = (this.turnReaders.get(threadId) ?? 1) - 1;
      if (remaining) { this.turnReaders.set(threadId, remaining); return; }
      this.turnReaders.delete(threadId);
      this.turnRequests.get(threadId)?.abort();
      this.turnRequests.delete(threadId);
      delete this.tickets[`turns:${threadId}`];
      const next = { ...this.turns };
      delete next[threadId];
      this.turns = next;
    };
  }

  async loadNeedsYou(): Promise<void> {
    const current = this.ticket('needs');
    const startedAt = new Date().toISOString();
    if (untrack(() => this.needsState) !== 'ready') this.needsState = 'loading';
    try {
      const items = await assistantApi.needsYou();
      if (!current()) return;
      this.dispatch({ type: 'load', items, startedAt });
      this.needsState = 'ready';
    } catch (e) {
      if (current()) this.needsState = isMissingRoute(e) ? 'unsupported' : 'error';
    }
  }

  async loadTasks(): Promise<void> {
    await this.load('tasks', () => assistantApi.tasks(), (v) => (this.tasks = v), () => this.tasks);
  }
  async loadMemory(): Promise<void> {
    await this.load('memory', () => assistantApi.memory(), (v) => (this.memory = v), () => this.memory);
  }
  async loadHermes(): Promise<void> {
    await this.load('hermes', () => assistantApi.hermesPreview(), (v) => (this.hermes = v), () => this.hermes);
  }
  async loadRouting(): Promise<void> {
    await this.load('routing', () => assistantApi.routing(), (v) => (this.routing = v), () => this.routing);
  }
  async loadUsageWeek(): Promise<void> {
    const current = this.ticket('usage');
    const was = untrack(() => this.usageWeek);
    this.usageWeek = { ...was, state: was.state === 'ready' ? 'ready' : 'loading' };
    try {
      const s = await api.get<UsageSummary>('/usage/summary?days=7&otto_only=false');
      if (current()) this.usageWeek = { state: 'ready', data: s.providers ?? [], error: '' };
    } catch (e) {
      // Usage is root-only and optional (ClickHouse may be off): not an error for this page.
      if (current()) this.usageWeek = { state: e instanceof ApiError && (e.status === 403 || e.status === 404 || e.status === 409 || e.status === 503) ? 'unsupported' : 'error', data: [], error: describeError(e) };
    }
  }

  /** Per-account CLI sign-in results, reused for {@link SIGNIN_TTL_MS}: each
   *  check boots the provider CLI (1–3 s, ≤ 10 s) and holds a webview socket
   *  the whole time, so re-opening Settings must not re-run them all. */
  private signedInCache = new Map<string, { value: boolean | null; at: number }>();

  /** `force` (an explicit Refresh) re-checks every account; otherwise cached
   *  sign-in results younger than the TTL are reused. */
  async loadAccounts(force = false): Promise<void> {
    const current = this.ticket('accounts');
    const was = untrack(() => this.accounts);
    this.accounts = { ...was, state: was.state === 'ready' ? 'ready' : 'loading' };
    try {
      const list = await api.get<ProviderAccount[]>('/auth/provider-accounts');
      if (!current()) return;
      const now = Date.now();
      const cached = (id: string): boolean | null | undefined => {
        const hit = this.signedInCache.get(id);
        return !force && hit && now - hit.at < SIGNIN_TTL_MS ? hit.value : undefined;
      };
      this.accounts = {
        state: 'ready',
        data: list.map((a) => ({ ...a, signed_in: cached(a.id) ?? null })),
        error: '',
      };
      // Sign-in checks run the CLI's own status command (≤ 10 s each) — fill
      // in as they land, at most SIGNIN_CONCURRENCY at a time: N parallel
      // checks used to take every one of the webview's 6 sockets (and spawn N
      // CLIs at once) while they booted.
      const stale = list.filter((a) => cached(a.id) === undefined);
      await mapLimit(stale, SIGNIN_CONCURRENCY, async (a) => {
        if (!current()) return;
        const signed = await api
          .get<{ signed_in: boolean }>(`/auth/provider-accounts/${encodeURIComponent(a.id)}/status`)
          .then((r) => r.signed_in)
          .catch(() => null);
        if (signed !== null) this.signedInCache.set(a.id, { value: signed, at: Date.now() });
        if (current()) this.accounts = { ...this.accounts, data: this.accounts.data.map((x) => (x.id === a.id ? { ...x, signed_in: signed } : x)) };
      });
    } catch (e) {
      if (current()) this.accounts = { state: isMissingRoute(e) ? 'unsupported' : 'error', data: [], error: describeError(e) };
    }
  }

  /** Drop a cached sign-in result (after a login/logout flow for `id`). */
  forgetSignIn(id?: string): void {
    if (id) this.signedInCache.delete(id);
    else this.signedInCache.clear();
  }

  async loadLimits(): Promise<void> {
    await this.load('limits', () => assistantApi.limits(), (v) => (this.limits = v), () => this.limits);
  }

  /** A task a card references but no list has yet (e.g. an old, finished one). */
  async ensureTask(id: string): Promise<void> {
    if (untrack(() => this.task(id)) || this.tickets[`task:${id}`]) return;
    this.ticket(`task:${id}`);
    try {
      this.mergeTask(await assistantApi.task(id));
    } catch {
      /* the card shows "not available" */
    }
  }

  private async load<T>(key: string, fetcher: () => Promise<T>, set: (v: Loadable<T>) => void, get: () => Loadable<T>): Promise<void> {
    const current = this.ticket(key);
    const was = untrack(get);
    // Keep showing the last data while refreshing (no skeleton flash).
    set({ ...was, state: was.state === 'ready' ? 'ready' : 'loading', error: '' });
    try {
      const data = await fetcher();
      if (current()) set({ state: 'ready', data, error: '' });
    } catch (e) {
      if (current()) set({ state: isMissingRoute(e) ? 'unsupported' : 'error', data: get().data, error: describeError(e) });
    }
  }

  /** WS (re)connect: the badge and the live views may have missed frames. */
  resync(): void {
    void this.loadNeedsYou();
    if (this.tasks.state !== 'idle') void this.loadTasks();
    if (this.threads.state !== 'idle') void this.loadThreads();
    if (this.limits.state !== 'idle') void this.loadLimits();
    void mapLimit([...this.turnReaders.keys()], 2, (id) => this.loadTurns(id));
  }

  // ── actions ────────────────────────────────────────────────────────────────

  async createThread(body: CreateAssistantThreadReq): Promise<AssistantThread> {
    const t = await assistantApi.createThread(body);
    this.mergeThread(t);
    return t;
  }

  async send(threadId: string, text: string, attachments: AssistantAttachment[]): Promise<void> {
    const local: PendingTurn = { id: `local-${Date.now().toString(36)}`, text, attachments, created_at: new Date().toISOString() };
    this.pending[threadId] = [...(this.pending[threadId] ?? []), local];
    const drop = (): void => {
      this.pending[threadId] = (this.pending[threadId] ?? []).filter((t) => t.id !== local.id);
    };
    try {
      const res = await assistantApi.send(threadId, { text, attachment_ids: attachments.map((a) => a.id), origin: 'app' });
      drop();
      this.mergeThread(res.thread);
      this.appendTurn(threadId, res.turn);
    } catch (e) {
      drop();
      throw e;
    }
  }

  /** Pin provider/model (the model chip); `provider: null` → back to the routing rules. */
  async route(threadId: string, provider: string | null, model: string | null = null): Promise<void> {
    this.mergeThread(await assistantApi.route(threadId, { provider, model }));
  }

  async act(taskId: string, action: AssistantTaskAction, body: AssistantDecisionReq = {}): Promise<AssistantTask> {
    const t = await assistantApi.act(taskId, action, body);
    this.mergeTask(t);
    return t;
  }

  async saveProfile(content: string): Promise<void> {
    const view = this.memory.data;
    const doc = await assistantApi.saveProfile(content, view?.profile.version ?? '');
    const now = this.memory.data;
    if (now) this.memory = { ...this.memory, data: { ...now, profile: doc } };
  }

  /** Forget one memory; returns the undo token. Chips referencing it read "Forgot". */
  async forgetMemory(id: string): Promise<string> {
    const res = await assistantApi.forgetMemory(id);
    const m = this.memory.data;
    if (m) this.memory = { ...this.memory, data: { ...m, memories: m.memories.filter((x) => x.id !== id), pending: m.pending.filter((x) => x.id !== id) } };
    this.forgotten = { ...this.forgotten, [id]: res.undo_token };
    return res.undo_token;
  }
  /** memory id → undo token, for chips undone in this window. */
  forgotten: Record<string, string> = $state({});

  async restoreMemory(undoTokens: string[]): Promise<void> {
    for (const t of undoTokens) await assistantApi.undoForget(t);
    this.forgotten = Object.fromEntries(Object.entries(this.forgotten).filter(([, tok]) => !undoTokens.includes(tok)));
    if (this.memory.state !== 'idle') void this.loadMemory();
  }

  async acceptMemory(id: string): Promise<void> {
    const mem = await assistantApi.acceptMemory(id);
    const m = this.memory.data;
    if (m) this.memory = { ...this.memory, data: { ...m, pending: m.pending.filter((x) => x.id !== id), memories: [mem, ...m.memories.filter((x) => x.id !== id)] } };
  }

  async saveRouting(body: Partial<Omit<AssistantRoutingSettings, 'updated_at'>>): Promise<void> {
    const data = await assistantApi.saveRouting(body);
    this.routing = { state: 'ready', data, error: '' };
  }

  // ── live events ────────────────────────────────────────────────────────────

  applyEvent(ev: OttoEvent): void {
    const e = ev as AssistantEvent;
    switch (e.type) {
      case 'assistant_turn':
        if (e.thread) this.mergeThread(e.thread);
        this.appendTurn(e.thread_id, e.turn);
        break;
      case 'assistant_task_update':
        this.mergeTask(e.task);
        break;
      case 'assistant_needs_you':
        this.mergeTask(e.task);
        // Drifted (a frame was lost): the count on the frame is the truth.
        if (e.open_count !== this.needs.items.length) void this.loadNeedsYou();
        break;
      case 'assistant_limit': {
        const l = e.limit;
        const rest = this.limits.data.filter((x) => !(x.provider === l.provider && x.account_id === l.account_id));
        this.limits = { ...this.limits, data: [l, ...rest] };
        if (e.task_id) void this.ensureTask(e.task_id);
        break;
      }
    }
  }

  private dispatch(a: NeedsYouAction): void {
    this.needs = reduceNeedsYou(this.needs, a);
  }

  private mergeThread(t: AssistantThread): void {
    const list = this.threads.data;
    const next = upsertNewer(list, t);
    if (next !== list) this.threads = { ...this.threads, data: next };
  }

  private mergeTask(t: AssistantTask): void {
    this.tasks = { ...this.tasks, data: upsertNewer(this.tasks.data, t) };
    this.dispatch({ type: 'upsert', task: t });
  }

  private appendTurn(threadId: string, turn: AssistantTurn): void {
    this.settlePending(threadId, [turn]);
    const have = this.turns[threadId];
    if (!have) return; // not open — it loads fresh when opened
    const i = have.data.findIndex((t) => t.id === turn.id);
    const data = i < 0 ? [...have.data, turn] : have.data.map((t, j) => (j === i ? turn : t));
    this.setTurns(threadId, { ...have, data });
    this.settlePending(threadId, [turn]);
  }

  private setTurns(threadId: string, v: Loadable<AssistantTurn[]>): void {
    if (!this.turnReaders.has(threadId)) return;
    // Bound a live view as well as its initial page. Release discards payload;
    // reacquiring reads a fresh page. Pending sends remain independent.
    const data = v.data.slice(-200);
    let bytes = 0;
    let start = data.length;
    while (start > 0) {
      const cost = JSON.stringify(data[start - 1]).length * 2;
      if (start < data.length && bytes + cost > 2 * 1024 * 1024) break;
      bytes += cost;
      start--;
    }
    this.turns = { ...untrack(() => this.turns), [threadId]: { ...v, data: data.slice(start) } };
  }

  private settlePending(threadId: string, turns: AssistantTurn[]): void {
    const p = this.pending[threadId];
    if (!p?.length) return;
    const echoed = turns.filter((t) => t.role === 'user').map((t) => t.text.trim());
    this.pending[threadId] = p.filter((t) => !echoed.some((x) => x === t.text.trim() || t.text.trim().endsWith(x)));
  }
}

export const assistant = new AssistantStore();
