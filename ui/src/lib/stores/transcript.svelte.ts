// Conversation-view state (docs/design/conversation-view.md §3/§5.2): one
// `Conversation` per source (session id, or an on-disk transcript path from the
// History page), holding the folded transcript, paging ("Load earlier" via the
// opaque `before` cursor) and the live tail fed by `transcript_appended`
// deltas. Subagent bodies are fetched lazily (`?sub=`) and cached per parent.
// Also owns the two persisted UI preferences: the global "Show system" toggle
// and the per-session Terminal · Chat view (winKey-namespaced, like the rest of
// the pane layout state).
import { api } from '../api/client';
import { winKey } from '../win';
import { TranscriptLifecycle } from './transcriptLifecycle';
import { parseSessionView, type SessionViewMode } from '../paneHeader';
import type { Transcript, TranscriptTouchQuery, Turn, Artifact, OttoEvent } from '../api/types';

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/** Where a conversation is read from: a live/known Otto session, or a raw
 *  transcript on disk that no session row claims (History `on_disk` rows). */
export type TranscriptSource =
  | { sessionId: string }
  | { workspaceId: string; transcriptPath: string };

export interface TranscriptPage {
  /** Opaque cursor (exclusive) — page earlier than this turn. */
  before?: string;
  /** Turn count per page. */
  limit?: number;
  /** Subagent id → reads `subagents/agent-<id>.jsonl` instead of the parent. */
  sub?: string;
}

/** Stable identity of a source — the store keys its per-conversation state by it. */
export function sourceKey(src: TranscriptSource): string {
  return 'sessionId' in src ? `s:${src.sessionId}` : `p:${src.workspaceId}:${src.transcriptPath}`;
}

function qs(page: TranscriptPage, extra: Record<string, string | undefined> = {}): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries({ ...extra, before: page.before, sub: page.sub })) {
    if (v !== undefined && v !== '') p.set(k, v);
  }
  if (page.limit !== undefined) p.set('limit', String(page.limit));
  const s = p.toString();
  return s ? `?${s}` : '';
}

export function fetchTranscript(
  src: TranscriptSource,
  page: TranscriptPage = {},
  signal?: AbortSignal,
): Promise<Transcript> {
  if ('sessionId' in src) {
    return api.get<Transcript>(
      `/sessions/${encodeURIComponent(src.sessionId)}/transcript${qs(page)}`,
      signal,
    );
  }
  return api.get<Transcript>(
    `/workspaces/${encodeURIComponent(src.workspaceId)}/history/transcript${qs(page, { path: src.transcriptPath })}`,
    signal,
  );
}

/** First page size — the app-style "last N turns, then Load earlier". */
export const PAGE_TURNS = 60;
/** A WS delta above this is not trusted to be complete — re-fetch the tail. */
const DELTA_CAP_BYTES = 64 * 1024;
/** A chat left open on a working agent keeps appending turns. Past TURN_CAP
 *  the oldest are dropped down to TURN_KEEP ("Load earlier" pages them back),
 *  so memory and per-delta work stay bounded however long the session runs. */
export const TURN_CAP = 600;
export const TURN_KEEP = 500;

/** Byte budget complements the turn count for tool-heavy conversations. */
const TURN_BYTES_CAP = 8 * 1024 * 1024;
const TURN_BYTES_KEEP = 6 * 1024 * 1024;
const turnCharges = new WeakMap<Turn, number>();
function turnCharge(turn: Turn): number {
  let charge = turnCharges.get(turn);
  if (charge === undefined) {
    charge = JSON.stringify(turn).length * 2;
    turnCharges.set(turn, charge);
  }
  return charge;
}

/** One lazily fetched subagent body (`?sub=<agent_id>`). */
export interface SubagentBody {
  turns: Turn[];
  loading: boolean;
  error: string | null;
  has_earlier: boolean;
  has_later: boolean;
  cursor: string;
  retryable: boolean;
}

/**
 * Drop the head of a live turn list that outgrew `cap`. The server pages by
 * RECORD index (`before` = exclusive first-record index), which turns don't
 * carry, so the list may only be cut in front of a turn with a known `before`
 * bound (`bounds`: turn id → a cursor B such that every turn ahead of it
 * started before B). Over-estimated bounds are safe — "Load earlier" dedupes
 * by id. Returns null when nothing can be dropped; otherwise the kept turns
 * and the cursor to page earlier from. `bounds` loses the dropped ids.
 */
export function trimTurnHead(
  turns: Turn[],
  bounds: Map<string, string>,
  cap = TURN_CAP,
  keep = TURN_KEEP,
): { turns: Turn[]; cursor: string; dropped: number } | null {
  const bytes = turns.reduce((total, turn) => total + turnCharge(turn), 0);
  const byteOverflow = bytes > TURN_BYTES_CAP;
  if (turns.length <= cap && !byteOverflow) return null;
  let start = Math.max(1, turns.length - keep);
  if (byteOverflow) {
    let retained = bytes;
    for (let i = 0; i < turns.length - 1; i++) {
      retained -= turnCharge(turns[i]);
      if (retained <= TURN_BYTES_KEEP) { start = Math.max(start, i + 1); break; }
    }
  }
  // A byte-heavy window may need fewer turns. Always retain its newest turn.
  const end = byteOverflow ? turns.length - 1 : turns.length - Math.ceil(keep / 2);
  for (let j = start; j <= end; j++) {
    const cursor = bounds.get(turns[j].id);
    if (cursor === undefined) continue;
    for (let i = 0; i < j; i++) bounds.delete(turns[i].id);
    return { turns: turns.slice(j), cursor, dropped: j };
  }
  return null;
}

// ---------------------------------------------------------------------------
// Per-source conversation
// ---------------------------------------------------------------------------

// Shared by every mounted parent in this window, not just one conversation.
const CHILD_BODY_COUNT = 8;
const CHILD_BODY_BYTES = 32 * 1024 * 1024;
const CHILD_PAGE_BYTES = 8 * 1024 * 1024;
const childBodyCharges = new Map<symbol, number>();
interface ChildReader {
  refs: number;
  key: symbol;
  controller: AbortController | null;
  pages: (string | undefined)[];
  index: number;
  retry: {before: string | undefined; index: number} | null;
}

export class Conversation {
  readonly src: TranscriptSource;
  /** Raw like `turns`: always replaced whole, never mutated. A deep proxy
   *  wrapped the page's turns and every `subagents` entry on first read (a
   *  big session's tree is 280+ entries, searched by each SubagentCard). */
  transcript: Transcript | null = $state.raw(null);
  /** Raw: turns are replaced (never mutated), and identity is what lets the
   *  view reuse render items and the search cache for unchanged turns. */
  turns: Turn[] = $state.raw([]);
  /** Turns dropped off the head by the TURN_CAP trim so far (the view shifts
   *  a pinned window by the difference). */
  headDropped = $state(0);
  loading = $state(false);
  loadingEarlier = $state(false);
  error: string | null = $state(null);
  /** Bumped when turns are appended by the live tail (auto-follow / "↓ new"). */
  tailTick = $state(0);
  /** Artifacts pushed by `artifact_added` since the load (chips at the tail). */
  liveArtifacts: Artifact[] = $state([]);
  /** The in-progress response read off the terminal screen (`transcript_live`);
   *  "" when nothing is streaming. Rendered as a draft under the last turn. */
  liveDraft = $state('');
  /** Unsent text in the terminal's input box (`transcript_live.input`). */
  liveInput = $state('');
  /** The CLI's status rows under the input box (`transcript_live.status`). */
  liveStatus = $state('');
  /** Git branch of the session cwd (`transcript_live.branch`). */
  liveBranch: string | null = $state(null);
  /** Bumped on every `transcript_appended` — the draft is hidden until the
   *  screen text moves past what the folded turn already shows. */
  lastAppendAt = $state(0);
  /** Lazy subagent bodies keyed by agent id (`?sub=`). Raw and replaced
   *  whole on every change (`setSub`), so a body's turns are never proxied. */
  subagents: Record<string, SubagentBody> = $state.raw({});
  private setSub(agentId: string, body: SubagentBody): void {
    this.subagents = { ...this.subagents, [agentId]: body };
  }
  private childReaders = new Map<string, ChildReader>();
  private inflight: AbortController | null = null;
  /** Index of the last record the client has folded (from the WS delta). */
  private tailCursor: string | null = null;
  /** Turn id → a `before` cursor that pages everything ahead of it (see
   *  `trimTurnHead`). Only ids still in `turns` are kept. */
  private headBounds = new Map<string, string>();
  /** The reader paged history in on purpose — don't trim it back out. */
  private holdCap = false;

  private parentBytes = 4096;
  get retainedBytes(): number {
    let total = this.parentBytes;
    for (const reader of this.childReaders.values()) total += childBodyCharges.get(reader.key) ?? 0;
    return total;
  }
  private retain(value: unknown): void {
    this.parentBytes = Math.min(32 * 1024 * 1024 + 1,
      this.parentBytes + TranscriptLifecycle.payloadCharge(value));
  }
  private readEpoch = 0;
  private reads = new AbortController();
  private isActive: () => boolean;
  private requestRead: () => void;

  constructor(src: TranscriptSource, isActive: () => boolean, requestRead: () => void) {
    this.src = src;
    this.isActive = isActive;
    this.requestRead = requestRead;
  }

  activate(): void {
    if (this.reads.signal.aborted) this.reads = new AbortController();
    // Visibility admission is restored after activate; defer until that same
    // synchronous store update completes, then refetch still-expanded cards.
    void Promise.resolve().then(() => {
      for (const [id, reader] of this.childReaders) if (reader.refs > 0) void this.loadSubagent(id);
    });
  }

  get key(): string {
    return sourceKey(this.src);
  }

  get sessionId(): string | null {
    return 'sessionId' in this.src ? this.src.sessionId : null;
  }

  /** True when the server resolved no transcript (chat shows the empty state). */
  get unavailable(): string | null {
    return this.transcript?.unavailable_reason ?? null;
  }

  requestRefresh(): void { this.requestRead(); }

  /** Explicit retry; ordinary mounted/reconnect reads use the lease scheduler. */
  async load(): Promise<void> {
    await this.readTail(undefined, true);
  }

  private async readTail(signal?: AbortSignal, replace = false): Promise<boolean> {
    if (!this.isActive() || signal?.aborted) return false;
    this.inflight?.abort();
    const ac = new AbortController();
    const abort = () => ac.abort();
    signal?.addEventListener('abort', abort, {once: true});
    this.inflight = ac;
    this.loading = this.transcript == null;
    this.error = null;
    try {
      const t = await fetchTranscript(this.src, {limit: PAGE_TURNS}, ac.signal);
      if (ac.signal.aborted || !this.isActive()) return false;
      this.retain(t);
      const overlaps = t.turns.some(turn => this.turns.some(old => old.id === turn.id));
      // A disconnected interval can exceed one page. Keep a single continuous
      // window so its cursor always reaches every preceding turn on disk.
      if (replace || this.transcript == null || this.transcript.unavailable_reason || !overlaps) {
        this.turns = t.turns;
        this.transcript = {...t, turns: []};
        this.headBounds.clear();
        this.holdCap = false;
      } else {
        const ids = new Set(t.turns.map(turn => turn.id));
        this.turns = [...this.turns.filter(turn => !ids.has(turn.id)), ...t.turns];
        this.transcript = {...t, turns: [], cursor: this.transcript.cursor, has_earlier: this.transcript.has_earlier};
      }
      // A page's cursor is the exact first record of its oldest turn.
      if (t.turns.length) this.headBounds.set(t.turns[0].id, t.cursor);
      if (!this.holdCap) this.followLive();
      this.tailCursor = null;
      this.tailTick++;
      return true;
    } catch (e) {
      if (!ac.signal.aborted && this.isActive()) this.error = e instanceof Error ? e.message : String(e);
      return false;
    } finally {
      signal?.removeEventListener('abort', abort);
      if (this.inflight === ac) {this.inflight = null; this.loading = false;}
    }
  }

  /** Page one batch of earlier turns in front of the current list. */
  async loadEarlier(): Promise<void> {
    const t = this.transcript;
    if (!t || !t.has_earlier || this.loadingEarlier) return;
    if (!this.isActive()) return;
    const epoch = this.readEpoch;
    this.loadingEarlier = true;
    try {
      const page = await fetchTranscript(this.src, { before: t.cursor, limit: PAGE_TURNS }, this.reads.signal);
      if (epoch !== this.readEpoch || !this.isActive() || this.transcript?.cursor !== t.cursor) return;
      this.retain(page);
      const known = new Set(this.turns.map((x) => x.id));
      this.turns = [...page.turns.filter((x) => !known.has(x.id)), ...this.turns];
      this.transcript = { ...this.transcript!, cursor: page.cursor, has_earlier: page.has_earlier };
      if (page.turns.length) this.headBounds.set(page.turns[0].id, page.cursor);
      this.holdCap = true;
    } catch (e) {
      if (epoch === this.readEpoch && this.isActive()) this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (epoch === this.readEpoch) this.loadingEarlier = false;
    }
  }

  /** Release historical pages when the reader explicitly returns to live. */
  followLive(): void {
    this.holdCap = false;
    const trimmed = trimTurnHead(this.turns, this.headBounds);
    if (trimmed && this.transcript) {
      this.turns = trimmed.turns;
      this.transcript = {...this.transcript, turns: [], cursor: trimmed.cursor, has_earlier: true};
      this.headDropped += trimmed.dropped;
    }
  }

  /** Apply a `transcript_appended` delta. A turn whose id we already hold is
   *  REPLACED (the agent's response grew); new ids are appended. Frames the
   *  server could not fit in 64 KB (empty `turns` with a moved cursor), or a
   *  cursor that went backwards (file replaced), trigger a tail re-fetch. */
  applyDelta(cursor: string, turns: Turn[]): void {
    if (this.transcript == null || this.transcript.unavailable_reason) {
      // First signs of life for a session that had no transcript yet.
      this.requestRead();
      return;
    }
    const moved = this.tailCursor == null || Number(cursor) > Number(this.tailCursor);
    const backwards = this.tailCursor != null && Number(cursor) < Number(this.tailCursor);
    if (backwards || (turns.length === 0 && moved) || JSON.stringify(turns).length > DELTA_CAP_BYTES) {
      this.requestRead();
      return;
    }
    if (!moved && turns.length === 0) return;
    const prevCursor = this.tailCursor;
    this.tailCursor = cursor;
    let next = [...this.turns];
    let firstNew = true;
    for (const t of turns) {
      const i = next.findIndex((x) => x.id === t.id);
      if (i >= 0) next[i] = t;
      else {
        // Every turn already held started at or before the previous delta's
        // last record, so `prev + 1` pages exactly the turns ahead of the
        // first NEW turn of this delta.
        if (firstNew && prevCursor != null) this.headBounds.set(t.id, String(Number(prevCursor) + 1));
        firstNew = false;
        next.push(t);
      }
    }
    // `stats.turns` stays the SERVER total (the loaded page is a window of it);
    // bump it only by the genuinely new turns.
    const added = next.length - this.turns.length;
    this.retain(turns);
    let transcript = this.transcript;
    const trimmed = this.holdCap ? null : trimTurnHead(next, this.headBounds);
    if (trimmed) {
      next = trimmed.turns;
      transcript = { ...transcript, cursor: trimmed.cursor, has_earlier: true };
      this.headDropped += trimmed.dropped;
    }
    this.turns = next;
    if (added > 0) {
      transcript = { ...transcript, stats: { ...transcript.stats, turns: transcript.stats.turns + added } };
    }
    if (transcript !== this.transcript) this.transcript = transcript;
    this.tailTick += 1;
    this.lastAppendAt = Date.now();
  }

  /** Apply a `transcript_live` frame (the screen draft + input + status). */
  applyLive(text: string, input = '', status = '', branch: string | null = null): void {
    // A hidden tab never paints; skip the churn (frames are state, not history).
    if (typeof document !== 'undefined' && document.hidden) return;
    if (text !== this.liveDraft) this.liveDraft = text;
    if (input !== this.liveInput) this.liveInput = input;
    if (status !== this.liveStatus) this.liveStatus = status;
    if (branch !== this.liveBranch) this.liveBranch = branch;
  }

  /** Keep the server-side tail armed while this conversation is on screen
   *  (it stops on its own a few minutes after the last touch). Passive by default;
   *  only an active-view mount opts into resume. Cheap: no fold. */
  async touch({view = true}: TranscriptTouchQuery = {}): Promise<void> {
    const sid = this.sessionId;
    if (!sid || !this.isActive()) return;
    try {
      await api.bg.post<void>(`/sessions/${encodeURIComponent(sid)}/transcript/touch?view=${view}`, {});
    } catch {
      /* 409 = no transcript yet (the view is retrying the GET); anything else is transient */
    }
  }

  /** Catch up after a gap (tab was hidden, socket reconnected): re-arm the
   *  tail and re-read the newest page so anything missed lands now. */
  async resync(signal?: AbortSignal): Promise<void> {
    if (await this.readTail(signal)) {
      if (!signal?.aborted && this.isActive()) await this.touch();
    }
  }

  addArtifact(a: Artifact): void {
    if (this.liveArtifacts.some((x) => x.id === a.id)) return;
    this.retain(a);
    this.liveArtifacts = [...this.liveArtifacts, a];
  }

  /** Cards own bodies explicitly. The final collapse drops payload immediately;
   * only small page cursors survive, so reopening preserves the reading page. */
  acquireSubagent(agentId: string): () => void {
    let reader = this.childReaders.get(agentId);
    if (!reader) {
      reader = {refs: 0, key: Symbol(agentId), controller: null, pages: [undefined], index: 0, retry: null};
      this.childReaders.set(agentId, reader);
    }
    reader.refs++;
    let released = false;
    const owned = reader;
    return () => {
      if (released) return;
      released = true;
      if (--owned.refs === 0) this.releaseSubagentBody(agentId, owned);
    };
  }

  private releaseSubagentBody(agentId: string, reader: ChildReader): void {
    reader.controller?.abort();
    reader.controller = null;
    childBodyCharges.delete(reader.key);
    const {[agentId]: _released, ...remaining} = this.subagents;
    this.subagents = remaining;
  }

  async loadSubagent(agentId: string): Promise<void> {
    const reader = this.childReaders.get(agentId);
    if (!reader?.refs || !this.isActive() || this.subagents[agentId]?.loading) return;
    const body = this.subagents[agentId];
    if (body?.turns.length && !body.error) return;
    if (body?.error && !body.retryable) return;
    const target = reader.retry ?? {before: reader.pages[reader.index], index: reader.index};
    await this.readSubagentPage(agentId, reader, target.before, target.index);
  }

  async loadSubagentEarlier(agentId: string): Promise<void> {
    const reader = this.childReaders.get(agentId), body = this.subagents[agentId];
    if (!reader?.refs || !body?.has_earlier || body.loading || !this.isActive()) return;
    await this.readSubagentPage(agentId, reader, body.cursor, reader.index + 1);
  }

  async loadSubagentLater(agentId: string): Promise<void> {
    const reader = this.childReaders.get(agentId), body = this.subagents[agentId];
    if (!reader?.refs || !body?.has_later || body.loading || !this.isActive()) return;
    const index = reader.index - 1;
    await this.readSubagentPage(agentId, reader, reader.pages[index], index);
  }

  private async readSubagentPage(agentId: string, reader: ChildReader, before: string | undefined, index: number): Promise<void> {
    const previous = this.subagents[agentId];
    const ac = new AbortController(), epoch = this.readEpoch;
    reader.controller?.abort();
    reader.controller = ac;
    const current = () => !ac.signal.aborted && reader.controller === ac && reader.refs > 0 && epoch === this.readEpoch && this.isActive();
    const empty: SubagentBody = {turns: [], loading: false, error: null, has_earlier: false, has_later: index > 0, cursor: '', retryable: true};
    this.setSub(agentId, {...(previous ?? empty), loading: true, error: null});
    let retryable = true;
    const hadBody = childBodyCharges.has(reader.key);
    let committed = false;
    try {
      // Reserve before fetching: expanding many cards must not start an
      // unbounded set of response downloads/JSON parses before admission.
      if (!hadBody) {
        if (childBodyCharges.size >= CHILD_BODY_COUNT) {
          throw new Error('Close another expanded subagent, then retry to display this transcript.');
        }
        childBodyCharges.set(reader.key, 0);
      }
      let limit = PAGE_TURNS;
      let t: Transcript;
      let charge: number;
      // A page with several large turns can be reduced without losing history:
      // the server's earlier cursor still reaches the omitted prefix.
      while (true) {
        t = await fetchTranscript(this.src, {sub: agentId, before, limit}, ac.signal);
        if (!current()) return;
        charge = TranscriptLifecycle.payloadCharge(t.turns);
        if (charge <= CHILD_PAGE_BYTES) break;
        if (t.turns.length <= 1 || limit === 1) {
          retryable = false;
          throw new Error('This recorded turn exceeds the subagent display limit. Read it in the provider transcript.');
        }
        limit = Math.max(1, Math.min(t.turns.length - 1, Math.floor(limit / 2)));
      }
      const oldCharge = childBodyCharges.get(reader.key) ?? 0;
      let total = 0;
      for (const bytes of childBodyCharges.values()) total += bytes;
      if (total - oldCharge + charge > CHILD_BODY_BYTES) {
        throw new Error('Close another expanded subagent, then retry to display this transcript.');
      }
      childBodyCharges.set(reader.key, charge);
      committed = true;
      reader.pages[index] = before;
      reader.index = index;
      reader.retry = null;
      this.setSub(agentId, {turns: t.turns, loading: false, error: null,
        has_earlier: t.has_earlier, has_later: index > 0, cursor: t.cursor, retryable: true});
    } catch (error) {
      if (!current()) return;
      reader.retry = {before, index};
      this.setSub(agentId, {...(previous ?? empty), loading: false, retryable,
        error: error instanceof Error ? error.message : String(error)});
    } finally {
      if (reader.controller === ac) {
        // A collapsed/reopened reader may already own a new reservation.
        if (!hadBody && !committed) childBodyCharges.delete(reader.key);
        reader.controller = null;
      }
    }
  }

  dispose(): void {
    this.readEpoch++;
    this.reads.abort();
    this.inflight?.abort();
    this.loadingEarlier = false;
    for (const [agentId, reader] of this.childReaders) this.releaseSubagentBody(agentId, reader);
  }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export type { SessionViewMode };

const LS_SHOW_SYSTEM = 'otto_conv_show_system';
const LS_VIEW_PREFIX = 'otto_session_view:';
/** The retired Split view's chat fraction — only ever deleted now. */
const LS_SPLIT_PREFIX = 'otto_session_split_frac:';
const LS_DRAFT_PREFIX = 'otto_chat_draft:';

function lsGet(k: string): string | null {
  try {
    return localStorage.getItem(k);
  } catch {
    return null;
  }
}
function lsSet(k: string, v: string): void {
  try {
    localStorage.setItem(k, v);
  } catch {
    /* private mode / quota — preference just doesn't persist */
  }
}
function lsDel(k: string): void {
  try {
    localStorage.removeItem(k);
  } catch {
    /* storage unavailable — nothing to clean up */
  }
}

class TranscriptStore {
  // Plain Map on purpose: `conversation()` is called from `$derived`s (which
  // may not write $state), and each Conversation carries its own $state fields
  // so reactivity lives on the instance, not on the registry.
  private convs = new Map<string, Conversation>();
  private lifecycle = new TranscriptLifecycle();
  private identity = '';
  private identityEpoch = $state(0);
  private inactive = new Map<string, number>();
  private retries = new Map<string, ReturnType<typeof setTimeout>>();
  private retryAttempts = new Map<string, number>();
  /** Global "Show system" — reveals reminders / hooks / injected queue items. */
  showSystem = $state(lsGet(LS_SHOW_SYSTEM) === '1');
  /** Per-session view choice, mirrored from localStorage so panes react. */
  private views: Record<string, SessionViewMode> = $state({});

  /** Get-or-create the conversation for a source (never fetches by itself). */
  conversation(src: TranscriptSource): Conversation {
    void this.identityEpoch;
    const k = sourceKey(src);
    let c = this.convs.get(k);
    if (!c) {
      c = new Conversation(src, () => this.lifecycle.active(k) && (typeof document === 'undefined' || !document.hidden), () => this.lifecycle.request(k));
      this.convs.set(k, c);
    }
    return c;
  }

  acquireView(src: TranscriptSource): () => void {
    const c = this.conversation(src);
    c.activate();
    this.inactive.delete(c.key);
    this.lifecycle.setVisible(typeof document === 'undefined' || !document.hidden);
    const release = this.lifecycle.acquire(c.key, async signal => {
      await c.resync(signal);
      const previous = this.retries.get(c.key);
      if (previous) clearTimeout(previous);
      this.retries.delete(c.key);
      if (!signal.aborted && this.lifecycle.active(c.key) && c.error?.includes('transcript busy')) {
        const attempt = this.retryAttempts.get(c.key) ?? 0;
        if (attempt < 3) {
          this.retryAttempts.set(c.key,attempt+1);
          this.retries.set(c.key,setTimeout(() => {
            this.retries.delete(c.key);
            if (this.lifecycle.active(c.key)) this.lifecycle.request(c.key);
          }, 1000 * (attempt+1)));
        }
      } else this.retryAttempts.delete(c.key);
    });
    return () => {
      release();
      if (!this.lifecycle.held(c.key)) {
        c.dispose();
        const retry = this.retries.get(c.key);
        if (retry) clearTimeout(retry);
        this.retries.delete(c.key); this.retryAttempts.delete(c.key);
        // Incoming bounded pages/deltas carry the charge; navigation scans no body.
        this.inactive.set(c.key, c.retainedBytes);
        this.evictInactive();
      }
    };
  }

  private evictInactive(): void {
    let bytes = [...this.inactive.values()].reduce((a, b) => a + b, 0);
    for (const [key, charge] of this.inactive) {
      if (this.inactive.size <= 24 && bytes <= 32 * 1024 * 1024) break;
      const c = this.convs.get(key);
      if (c?.sessionId && this.sending(c.sessionId)) continue;
      c?.dispose();
      this.convs.delete(key);
      this.inactive.delete(key);
      bytes -= charge;
    }
  }

  setVisible(visible: boolean): void {
    if (visible) for (const c of this.convs.values()) c.activate();
    else {
      for (const c of this.convs.values()) c.dispose();
      for (const retry of this.retries.values()) clearTimeout(retry);
      this.retries.clear(); this.retryAttempts.clear();
    }
    this.lifecycle.setVisible(visible);
  }

  setIdentity(identity: string): void {
    if (this.identity === identity) return;
    const previous = this.identity;
    this.identity = identity;
    if (previous) this.reset();
  }

  reset(): void {
    this.identityEpoch++;
    for (const retry of this.retries.values()) clearTimeout(retry);
    this.retries.clear(); this.retryAttempts.clear();
    this.lifecycle.clear();
    for (const c of this.convs.values()) c.dispose();
    this.convs.clear();
    this.inactive.clear();
  }

  peek(sessionId: string): Conversation | null {
    return this.convs.get(`s:${sessionId}`) ?? null;
  }

  forget(src: TranscriptSource): void {
    const k = sourceKey(src);
    this.convs.get(k)?.dispose();
    this.convs.delete(k);
    this.inactive.delete(k);
  }

  setShowSystem(on: boolean): void {
    this.showSystem = on;
    lsSet(LS_SHOW_SYSTEM, on ? '1' : '0');
  }

  /** The saved Terminal · Chat choice for a session, or null (= use the
   *  default). A choice saved by the retired Split view reads as Chat and is
   *  rewritten once, with its orphaned split fraction dropped. */
  view(sessionId: string): SessionViewMode | null {
    const cached = this.views[sessionId];
    if (cached) return cached;
    const key = winKey(LS_VIEW_PREFIX + sessionId);
    const raw = lsGet(key);
    const mode = parseSessionView(raw);
    if (raw === 'split' && mode) {
      lsSet(key, mode);
      lsDel(winKey(LS_SPLIT_PREFIX + sessionId));
    }
    return mode;
  }

  setView(sessionId: string, mode: SessionViewMode): void {
    this.views[sessionId] = mode;
    lsSet(winKey(LS_VIEW_PREFIX + sessionId), mode);
  }

  /** Recover only mounted, document-visible conversations. */
  resyncVisible(): void {
    this.setVisible(typeof document === 'undefined' || !document.hidden);
    this.lifecycle.request();
  }

  /** Route the three transcript WS events (called from events.svelte.ts). */
  applyEvent(ev: OttoEvent): boolean {
    switch (ev.type) {
      case 'transcript_appended': {
        (this.lifecycle.active(`s:${ev.session_id}`) ? this.convs.get(`s:${ev.session_id}`) : undefined)?.applyDelta(ev.cursor, ev.turns);
        return true;
      }
      case 'transcript_live': {
        (this.lifecycle.active(`s:${ev.session_id}`) ? this.convs.get(`s:${ev.session_id}`) : undefined)?.applyLive(ev.text, ev.input, ev.status, ev.branch);
        return true;
      }
      case 'artifact_added': {
        (this.lifecycle.active(`s:${ev.session_id}`) ? this.convs.get(`s:${ev.session_id}`) : undefined)?.addArtifact(ev.artifact);
        return true;
      }
      case 'history_index_progress':
        // The History page (Track C) reads `historyIndex` below.
        this.historyIndex = { scanned: ev.scanned, total: ev.total, done: ev.done };
        return true;
      default:
        return false;
    }
  }

  /** Unsent composer text per session — survives leaving the page and coming
   *  back (in-memory, mirrored to sessionStorage so a reload keeps it too). */
  private drafts: Record<string, string> = $state({});
  private pendingSends: Record<string, boolean> = $state({});
  sending(sessionId: string): boolean { return this.pendingSends[sessionId] === true; }
  tryBeginSend(sessionId: string): boolean {
    if (this.sending(sessionId)) return false;
    this.pendingSends[sessionId] = true;
    return true;
  }
  finishSend(sessionId: string): void { delete this.pendingSends[sessionId]; }
  /** Uploaded inbox files stay with their session across composer remounts.
   *  Object URLs are browser-local and are released on explicit remove/send. */
  private draftImages: Record<string, { path: string; name: string; url: string }[]> = $state({});
  attachments(sessionId: string): { path: string; name: string; url: string }[] {
    return this.draftImages[sessionId] ?? [];
  }
  setAttachments(sessionId: string, images: { path: string; name: string; url: string }[]): void {
    this.draftImages[sessionId] = images;
  }
  private draftKey(sessionId: string): string {
    return winKey(LS_DRAFT_PREFIX + JSON.stringify([this.identity, sessionId]));
  }
  draft(sessionId: string): string {
    const key = this.draftKey(sessionId);
    const mem = this.drafts[key];
    if (mem !== undefined) return mem;
    try {
      const saved = sessionStorage.getItem(key);
      if (saved !== null) return saved;
      // Adopt a pre-namespace draft once; do not erase a user's unsent text
      // during the upgrade, or expose it to later identity switches.
      const legacy = sessionStorage.getItem(LS_DRAFT_PREFIX + sessionId);
      if (legacy !== null) {
        sessionStorage.setItem(key,legacy);
        sessionStorage.removeItem(LS_DRAFT_PREFIX + sessionId);
        return legacy;
      }
      return '';
    } catch {return '';}

  }
  setDraft(sessionId: string, text: string): void {
    const key = this.draftKey(sessionId);
    this.drafts[key] = text;
    try {
      if (text) sessionStorage.setItem(key,text);
      else sessionStorage.removeItem(key);
    } catch { /* In-memory drafts remain available. */ }
  }

  /** Latest `history_index_progress` (null until the first event). */
  historyIndex: { scanned: number; total: number; done: boolean } | null = $state(null);
}

export const transcript = new TranscriptStore();
