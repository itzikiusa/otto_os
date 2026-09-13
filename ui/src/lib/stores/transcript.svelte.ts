// Conversation-view state (docs/design/conversation-view.md §3/§5.2): one
// `Conversation` per source (session id, or an on-disk transcript path from the
// History page), holding the folded transcript, paging ("Load earlier" via the
// opaque `before` cursor) and the live tail fed by `transcript_appended`
// deltas. Subagent bodies are fetched lazily (`?sub=`) and cached per parent.
// Also owns the two persisted UI preferences: the global "Show system" toggle
// and the per-session Terminal · Chat · Split view (winKey-namespaced, like
// the rest of the pane layout state).
import { api } from '../api/client';
import { winKey } from '../win';
import { TranscriptLifecycle } from './transcriptLifecycle';
import type { Transcript, Turn, Artifact, OttoEvent } from '../api/types';

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

// ---------------------------------------------------------------------------
// Per-source conversation
// ---------------------------------------------------------------------------

export class Conversation {
  readonly src: TranscriptSource;
  transcript: Transcript | null = $state(null);
  turns: Turn[] = $state([]);
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
  /** Lazy subagent bodies keyed by agent id (`?sub=`). */
  subagents: Record<string, { turns: Turn[]; loading: boolean; error: string | null; has_earlier: boolean; cursor: string }> =
    $state({});
  private inflight: AbortController | null = null;
  /** Index of the last record the client has folded (from the WS delta). */
  private tailCursor: string | null = null;

  retainedBytes = 4096;
  private retain(value: unknown): void {
    this.retainedBytes = Math.min(32 * 1024 * 1024 + 1,
      this.retainedBytes + TranscriptLifecycle.payloadCharge(value));
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
      if (replace || this.transcript == null || this.transcript.unavailable_reason) {
        this.turns = t.turns;
        this.transcript = t;
      } else {
        const ids = new Set(t.turns.map(turn => turn.id));
        this.turns = [...this.turns.filter(turn => !ids.has(turn.id)), ...t.turns];
        this.transcript = {...t, cursor: this.transcript.cursor, has_earlier: this.transcript.has_earlier};
      }
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
      if (epoch !== this.readEpoch || !this.isActive()) return;
      this.retain(page);
      const known = new Set(this.turns.map((x) => x.id));
      this.turns = [...page.turns.filter((x) => !known.has(x.id)), ...this.turns];
      this.transcript = { ...t, cursor: page.cursor, has_earlier: page.has_earlier };
    } catch (e) {
      if (epoch === this.readEpoch && this.isActive()) this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (epoch === this.readEpoch) this.loadingEarlier = false;
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
    this.tailCursor = cursor;
    const next = [...this.turns];
    for (const t of turns) {
      const i = next.findIndex((x) => x.id === t.id);
      if (i >= 0) next[i] = t;
      else next.push(t);
    }
    // `stats.turns` stays the SERVER total (the loaded page is a window of it);
    // bump it only by the genuinely new turns.
    const added = next.length - this.turns.length;
    this.retain(turns);
    this.turns = next;
    if (added > 0) {
      this.transcript = {
        ...this.transcript,
        stats: { ...this.transcript.stats, turns: this.transcript.stats.turns + added },
      };
    }
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
   *  (it stops on its own a few minutes after the last touch). Cheap: no fold. */
  async touch(): Promise<void> {
    const sid = this.sessionId;
    if (!sid || !this.isActive()) return;
    try {
      await api.post<void>(`/sessions/${encodeURIComponent(sid)}/transcript/touch`, {});
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

  /** Fetch a subagent's body once (nested card expand). */
  async loadSubagent(agentId: string): Promise<void> {
    if (!this.isActive()) return;
    const epoch = this.readEpoch;
    if (this.subagents[agentId]?.turns.length || this.subagents[agentId]?.loading) return;
    this.subagents[agentId] = { turns: [], loading: true, error: null, has_earlier: false, cursor: '' };
    try {
      const t = await fetchTranscript(this.src, { sub: agentId, limit: PAGE_TURNS }, this.reads.signal);
      if (epoch !== this.readEpoch || !this.isActive()) return;
      this.retain(t);
      this.subagents[agentId] = { turns: t.turns, loading: false, error: null, has_earlier: t.has_earlier, cursor: t.cursor };
    } catch (e) {
      if (epoch !== this.readEpoch || !this.isActive()) return;
      this.subagents[agentId] = {
        turns: [],
        loading: false,
        error: e instanceof Error ? e.message : String(e),
        has_earlier: false,
        cursor: '',
      };
    }
  }

  async loadSubagentEarlier(agentId: string): Promise<void> {
    if (!this.isActive()) return;
    const epoch = this.readEpoch;
    const cur = this.subagents[agentId];
    if (!cur || !cur.has_earlier || cur.loading) return;
    this.subagents[agentId] = { ...cur, loading: true };
    try {
      const t = await fetchTranscript(this.src, { sub: agentId, before: cur.cursor, limit: PAGE_TURNS }, this.reads.signal);
      if (epoch !== this.readEpoch || !this.isActive()) return;
      this.retain(t);
      this.subagents[agentId] = {
        turns: [...t.turns, ...cur.turns],
        loading: false,
        error: null,
        has_earlier: t.has_earlier,
        cursor: t.cursor,
      };
    } catch (e) {
      if (epoch !== this.readEpoch || !this.isActive()) return;
      this.subagents[agentId] = { ...cur, loading: false, error: e instanceof Error ? e.message : String(e) };
    }
  }

  dispose(): void {
    this.readEpoch++;
    this.reads.abort();
    this.inflight?.abort();
    this.loadingEarlier = false;
    for (const [id, state] of Object.entries(this.subagents)) {
      if (state.loading) this.subagents[id] = { ...state, loading: false };
    }
  }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export type SessionViewMode = 'terminal' | 'chat' | 'split';

const LS_SHOW_SYSTEM = 'otto_conv_show_system';
const LS_VIEW_PREFIX = 'otto_session_view:';
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

  /** The saved Terminal · Chat · Split choice for a session, or null (= use the
   *  transcript-driven default). */
  view(sessionId: string): SessionViewMode | null {
    const cached = this.views[sessionId];
    if (cached) return cached;
    const raw = lsGet(winKey(LS_VIEW_PREFIX + sessionId));
    return raw === 'terminal' || raw === 'chat' || raw === 'split' ? raw : null;
  }

  setView(sessionId: string, mode: SessionViewMode): void {
    this.views[sessionId] = mode;
    lsSet(winKey(LS_VIEW_PREFIX + sessionId), mode);
  }

  /** Chat-pane fraction of the Split view (0.3–0.8), per session. */
  splitFrac(sessionId: string): number {
    const n = Number(lsGet(winKey(LS_SPLIT_PREFIX + sessionId)));
    return Number.isFinite(n) && n >= 0.3 && n <= 0.8 ? n : 0.55;
  }

  setSplitFrac(sessionId: string, frac: number): void {
    lsSet(winKey(LS_SPLIT_PREFIX + sessionId), String(Math.min(0.8, Math.max(0.3, frac))));
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
