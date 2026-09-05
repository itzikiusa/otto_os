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
  return 'sessionId' in src ? `s:${src.sessionId}` : `p:${src.transcriptPath}`;
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
  /** Bumped on every `transcript_appended` — the draft is hidden until the
   *  screen text moves past what the folded turn already shows. */
  lastAppendAt = $state(0);
  /** Lazy subagent bodies keyed by agent id (`?sub=`). */
  subagents: Record<string, { turns: Turn[]; loading: boolean; error: string | null; has_earlier: boolean; cursor: string }> =
    $state({});
  private inflight: AbortController | null = null;
  /** Index of the last record the client has folded (from the WS delta). */
  private tailCursor: string | null = null;

  constructor(src: TranscriptSource) {
    this.src = src;
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

  /** (Re)load the newest page, replacing what is shown. */
  async load(): Promise<void> {
    this.inflight?.abort();
    const ac = new AbortController();
    this.inflight = ac;
    this.loading = true;
    this.error = null;
    try {
      const t = await fetchTranscript(this.src, { limit: PAGE_TURNS }, ac.signal);
      if (ac.signal.aborted) return;
      this.transcript = t;
      this.turns = t.turns;
      this.tailCursor = null;
    } catch (e) {
      if (ac.signal.aborted) return;
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (this.inflight === ac) {
        this.inflight = null;
        this.loading = false;
      }
    }
  }

  /** Page one batch of earlier turns in front of the current list. */
  async loadEarlier(): Promise<void> {
    const t = this.transcript;
    if (!t || !t.has_earlier || this.loadingEarlier) return;
    this.loadingEarlier = true;
    try {
      const page = await fetchTranscript(this.src, { before: t.cursor, limit: PAGE_TURNS });
      const known = new Set(this.turns.map((x) => x.id));
      this.turns = [...page.turns.filter((x) => !known.has(x.id)), ...this.turns];
      this.transcript = { ...t, cursor: page.cursor, has_earlier: page.has_earlier };
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.loadingEarlier = false;
    }
  }

  /** Apply a `transcript_appended` delta. A turn whose id we already hold is
   *  REPLACED (the agent's response grew); new ids are appended. Frames the
   *  server could not fit in 64 KB (empty `turns` with a moved cursor), or a
   *  cursor that went backwards (file replaced), trigger a tail re-fetch. */
  applyDelta(cursor: string, turns: Turn[]): void {
    if (this.transcript == null || this.transcript.unavailable_reason) {
      // First signs of life for a session that had no transcript yet.
      void this.load();
      return;
    }
    const moved = this.tailCursor == null || Number(cursor) > Number(this.tailCursor);
    const backwards = this.tailCursor != null && Number(cursor) < Number(this.tailCursor);
    if (backwards || (turns.length === 0 && moved) || JSON.stringify(turns).length > DELTA_CAP_BYTES) {
      void this.refetchTail();
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

  /** Apply a `transcript_live` frame (the screen draft). */
  applyLive(text: string): void {
    if (text === this.liveDraft) return;
    this.liveDraft = text;
  }

  /** Keep the server-side tail armed while this conversation is on screen
   *  (it stops on its own a few minutes after the last touch). Cheap: no fold. */
  async touch(): Promise<void> {
    const sid = this.sessionId;
    if (!sid) return;
    try {
      await api.post<void>(`/sessions/${encodeURIComponent(sid)}/transcript/touch`, {});
    } catch {
      /* 409 = no transcript yet (the view is retrying the GET); anything else is transient */
    }
  }

  /** Re-read the newest page and merge it over what we hold (keeps earlier pages). */
  private async refetchTail(): Promise<void> {
    try {
      const t = await fetchTranscript(this.src, { limit: PAGE_TURNS });
      // Earlier pages we already hold (not in the fresh window) stay in front.
      const older = this.turns.filter((x) => !t.turns.some((n) => n.id === x.id));
      this.turns = [...older, ...t.turns];
      this.transcript = {
        ...t,
        cursor: this.transcript?.cursor ?? t.cursor,
        has_earlier: this.transcript?.has_earlier ?? t.has_earlier,
      };
      this.tailCursor = null;
      this.tailTick += 1;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  addArtifact(a: Artifact): void {
    if (this.liveArtifacts.some((x) => x.id === a.id)) return;
    this.liveArtifacts = [...this.liveArtifacts, a];
  }

  /** Fetch a subagent's body once (nested card expand). */
  async loadSubagent(agentId: string): Promise<void> {
    if (this.subagents[agentId]?.turns.length || this.subagents[agentId]?.loading) return;
    this.subagents[agentId] = { turns: [], loading: true, error: null, has_earlier: false, cursor: '' };
    try {
      const t = await fetchTranscript(this.src, { sub: agentId, limit: PAGE_TURNS });
      this.subagents[agentId] = { turns: t.turns, loading: false, error: null, has_earlier: t.has_earlier, cursor: t.cursor };
    } catch (e) {
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
    const cur = this.subagents[agentId];
    if (!cur || !cur.has_earlier || cur.loading) return;
    this.subagents[agentId] = { ...cur, loading: true };
    try {
      const t = await fetchTranscript(this.src, { sub: agentId, before: cur.cursor, limit: PAGE_TURNS });
      this.subagents[agentId] = {
        turns: [...t.turns, ...cur.turns],
        loading: false,
        error: null,
        has_earlier: t.has_earlier,
        cursor: t.cursor,
      };
    } catch (e) {
      this.subagents[agentId] = { ...cur, loading: false, error: e instanceof Error ? e.message : String(e) };
    }
  }

  dispose(): void {
    this.inflight?.abort();
  }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

export type SessionViewMode = 'terminal' | 'chat' | 'split';

const LS_SHOW_SYSTEM = 'otto_conv_show_system';
const LS_VIEW_PREFIX = 'otto_session_view:';
const LS_SPLIT_PREFIX = 'otto_session_split_frac:';

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
  /** Global "Show system" — reveals reminders / hooks / injected queue items. */
  showSystem = $state(lsGet(LS_SHOW_SYSTEM) === '1');
  /** Per-session view choice, mirrored from localStorage so panes react. */
  private views: Record<string, SessionViewMode> = $state({});

  /** Get-or-create the conversation for a source (never fetches by itself). */
  conversation(src: TranscriptSource): Conversation {
    const k = sourceKey(src);
    let c = this.convs.get(k);
    if (!c) {
      c = new Conversation(src);
      this.convs.set(k, c);
    }
    return c;
  }

  /** Get-or-create AND load once — the cheap "does this session have a
   *  transcript?" probe SessionView uses to pick the default view. */
  ensure(src: TranscriptSource): Conversation {
    const c = this.conversation(src);
    if (c.transcript == null && !c.loading && c.error == null) void c.load();
    return c;
  }

  peek(sessionId: string): Conversation | null {
    return this.convs.get(`s:${sessionId}`) ?? null;
  }

  forget(src: TranscriptSource): void {
    const k = sourceKey(src);
    this.convs.get(k)?.dispose();
    this.convs.delete(k);
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

  /** Route the three transcript WS events (called from events.svelte.ts). */
  applyEvent(ev: OttoEvent): boolean {
    switch (ev.type) {
      case 'transcript_appended': {
        this.convs.get(`s:${ev.session_id}`)?.applyDelta(ev.cursor, ev.turns);
        return true;
      }
      case 'transcript_live': {
        this.convs.get(`s:${ev.session_id}`)?.applyLive(ev.text);
        return true;
      }
      case 'artifact_added': {
        this.convs.get(`s:${ev.session_id}`)?.addArtifact(ev.artifact);
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

  /** Latest `history_index_progress` (null until the first event). */
  historyIndex: { scanned: number; total: number; done: boolean } | null = $state(null);
}

export const transcript = new TranscriptStore();
