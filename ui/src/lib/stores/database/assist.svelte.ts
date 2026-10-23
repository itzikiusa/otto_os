// Database Explorer store — the DB Assistant (embedded, file-backed agent
// panel). The DatabaseStore façade (../database.svelte.ts) owns an instance and
// delegates its `assist*` fields and methods here; the host supplies the
// selected connection, the active database and the query-tab hooks (insert the
// proposed SQL, run it read-only).
//
// The DB Assistant runs an agent as a managed Otto session beside the query
// editor/results. The panel binds a live `<Terminal>` to its session (the user
// talks to the agent IN the shell), shows the agent's proposed SQL, and can
// summarize → download or close → discard. The session is hidden from Agents
// (meta.source = 'db_assist'). State is ephemeral (no persistence) — Close
// (DELETE) kills the session + discards the working dir.

import { dbAssistClose, dbAssistStart, dbAssistSummary } from '../../api/client';
import type { DbAssistMode, Id } from '../../api/types';
import { auth } from '../auth.svelte';
import { confirmer } from '../../confirm.svelte';
import { toasts } from '../../toast.svelte';
import { toastError } from '../../toastError';
import { downloadText } from '../../components/exporters';

/** What the assist store reads from (and asks of) its DatabaseStore host. */
export interface AssistHost {
  selectedConnId(): Id | null;
  /** The active database/node, sent with each turn. */
  activeDb(): string | null;
  /** The active query tab's text — inserting asks before replacing it. */
  currentStatement(): string;
  /** Put `sql` in the active query tab and show the Query view. */
  insertIntoQuery(sql: string): void;
  /** Run the active tab READ-ONLY with an attributed write confirm. */
  runReadOnly(agentLabel: string, confirmWrite: () => Promise<boolean>): Promise<void>;
}

/** How long a closed, id-less turn waits to surface before its orphan slot
 *  lapses (a turn that never started a session must not claim a later one). */
const ORPHAN_TTL_MS = 120_000;

export class DbAssistStore {
  private readonly host: AssistHost;
  constructor(host: AssistHost) {
    this.host = host;
  }

  /** The DB Assistant panel is open (split beside the editor/results). */
  assistOpen = $state(false);
  /** Connection the current assist targets (captured at open). */
  assistConnId: Id | null = $state(null);
  /** Server assist id (minted by the first turn; the resume + DELETE key). */
  assistId: string | null = $state(null);
  /** Live agent session id — the panel mounts `<Terminal>` on it. */
  assistSessionId: string | null = $state(null);
  /** Entry mode (nl/ask/investigate) — drives the prompt + the panel hint. */
  assistMode: DbAssistMode = $state('nl');
  /** Chosen agent CLI (claude/codex/…) — picked before the first turn. */
  assistProvider = $state('');
  /** The agent's proposed SQL (start response + live `db_assist_updated`). */
  assistProposedSql = $state('');
  /** A one-line explanation/note from the agent. */
  assistNote = $state('');
  /** A turn (the first POST) is in flight. */
  assistBusy = $state(false);
  /** investigate-mode seed (statement + a small result sample); non-reactive. */
  private assistResultContext: string | null = null;
  /** Aborts the in-flight assist POST (Stop). Non-reactive plain field. */
  private assistAbort: AbortController | null = null;
  /** Turn generation, bumped by open and close: a turn that outlives the panel
   *  it started in never writes the panel (id, session, SQL, busy). */
  private gen = 0;
  /** Stop dropped the POST before the server's id arrived: the panel still
   *  adopts the turn's `db_assist_session_started`, so Close can discard it. */
  private awaitingId = false;
  /** Per connection, the deadlines of turns that were closed before their
   *  assist id was known. The next unclaimed `db_assist_session_started` for
   *  that connection is one of them and is discarded on sight (DELETE) — the
   *  session is hidden from Agents, so nothing else would ever close it. */
  private orphans = new Map<Id, number[]>();
  /** Assist ids already discarded (an orphan's event and its late POST
   *  response both name it; one DELETE). */
  private discarded = new Set<string>();

  /**
   * Open the DB Assistant panel in `mode`. For `investigate`, pass a compact
   * `resultContext` (statement + result sample). Targets the active connection.
   * Does NOT start a session — the user picks the agent and types the first
   * question (which POSTs the first turn). Reopening resets the local view; the
   * previous session (if any) was already discarded on Close.
   */
  openAssist(mode: DbAssistMode, resultContext?: string | null): void {
    if (!auth.can('agents','edit')) return;
    const connId = this.host.selectedConnId();
    if (!connId) {
      toasts.error('No connection selected');
      return;
    }
    this.retireTurn();
    this.assistOpen = true;
    this.assistMode = mode;
    this.assistConnId = connId;
    this.assistResultContext = resultContext ?? null;
    this.assistId = null;
    this.assistSessionId = null;
    this.assistProposedSql = '';
    this.assistNote = '';
    this.assistBusy = false;
  }

  /** Set the chosen agent CLI for the assist (before the first turn). */
  setAssistProvider(provider: string): void {
    this.assistProvider = provider;
  }

  /**
   * Run ONE agent turn — the first question (or a resume). POSTs to
   * `…/db/assist`; the live session appears via the `db_assist_session_started`
   * event (mirrored in the response as a fallback) and the proposed SQL streams
   * in via `db_assist_updated`. After the first turn the user continues the
   * conversation by typing directly in the embedded terminal.
   */
  async startAssist(question: string): Promise<void> {
    const id = this.assistConnId ?? this.host.selectedConnId();
    if (!id) {
      toasts.error('No connection selected');
      return;
    }
    const q = question.trim();
    if (!q || this.assistBusy) return;
    this.assistBusy = true;
    this.awaitingId = false;
    const gen = this.gen;
    // Abortable: the POST is long (agent turn) and can wedge — Stop drops the
    // HTTP wait and releases the input (the session, if started, stays live).
    const ac = new AbortController();
    this.assistAbort = ac;
    const activeDb = this.host.activeDb();
    try {
      const resp = await dbAssistStart(
        id,
        {
          question: q,
          mode: this.assistMode,
          ...(activeDb ? { node: activeDb } : {}),
          ...(this.assistProvider ? { provider: this.assistProvider } : {}),
          ...(this.assistId ? { assist_id: this.assistId } : {}),
          ...(this.assistResultContext && this.assistMode === 'investigate'
            ? { result_context: this.assistResultContext }
            : {}),
        },
        ac.signal,
      );
      // The panel was closed or reopened while the turn ran: its session is
      // nobody's now — discard it, and leave the (new) panel untouched.
      if (gen !== this.gen) {
        this.discardOrphan(id, resp.assist_id);
        return;
      }
      this.assistId = resp.assist_id;
      // The session usually arrives first (turn START) via the event; set it from
      // the response too in case it didn't (idempotent — same id).
      if (resp.session_id) this.assistSessionId = resp.session_id;
      if (resp.sql) this.assistProposedSql = resp.sql;
      if (resp.note) this.assistNote = resp.note;
    } catch (e) {
      if (gen !== this.gen) {
        // A stale turn that failed minted nothing to discard.
        this.takeOrphan(id);
        return;
      }
      if (!ac.signal.aborted) toastError('Couldn’t run the DB assistant', e);
    } finally {
      if (this.assistAbort === ac) this.assistAbort = null;
      if (gen === this.gen) this.assistBusy = false;
    }
  }

  /** Stop the in-flight assist turn: abort the POST and release the input. A
   *  turn whose id never arrived stays adoptable by its session event. */
  stopAssist(): void {
    if (this.assistBusy && this.assistId === null) this.awaitingId = true;
    this.assistAbort?.abort();
    this.assistAbort = null;
    this.assistBusy = false;
  }

  /** End the panel's current turn (open / close): a turn still waiting for its
   *  id becomes an orphan of its connection, discarded when it surfaces. The
   *  POST itself is NOT aborted: its response carries the minted assist id,
   *  the one reliable handle to DELETE it by; dropping it would leave only the
   *  WS event to find the session with. */
  private retireTurn(): void {
    const conn = this.assistConnId;
    if (conn && this.assistId === null && (this.assistBusy || this.awaitingId)) {
      const list = this.orphans.get(conn) ?? [];
      list.push(Date.now() + ORPHAN_TTL_MS);
      this.orphans.set(conn, list);
    }
    this.gen++;
    this.awaitingId = false;
    this.assistBusy = false;
    this.assistAbort = null;
  }

  /** Consume one orphan slot of `connId`; false when it has none (expired
   *  slots — a turn that never started a session — are dropped first). */
  private takeOrphan(connId: Id): boolean {
    const now = Date.now();
    const live = (this.orphans.get(connId) ?? []).filter((t) => t > now);
    const took = live.length > 0;
    if (took) live.shift();
    if (live.length) this.orphans.set(connId, live);
    else this.orphans.delete(connId);
    return took;
  }

  /** DELETE an orphaned assist once (its event and its late POST response
   *  both lead here), releasing its connection's orphan slot. */
  private discardOrphan(connId: Id, assistId: string): void {
    if (this.discarded.has(assistId)) return;
    this.discarded.add(assistId);
    this.takeOrphan(connId);
    void dbAssistClose(connId, assistId).catch(() => {
      /* best-effort discard — the session is reaped on daemon restart anyway */
    });
  }

  /** `db_assist_session_started` → attach the live shell, or discard the
   *  session of a turn that was closed before its id was known. */
  setAssistSession(assistId: string, connId: Id, sessionId: string): void {
    if (this.discarded.has(assistId)) return;
    if (assistId !== this.assistId && this.orphans.has(connId) && this.takeOrphanFor(connId, assistId)) return;
    if (this.adoptAssist(assistId, connId)) this.assistSessionId = sessionId;
  }

  /** An unclaimed session of a connection with a live orphan slot is that
   *  orphan: DELETE it (true). */
  private takeOrphanFor(connId: Id, assistId: string): boolean {
    const now = Date.now();
    if (!(this.orphans.get(connId) ?? []).some((t) => t > now)) {
      this.takeOrphan(connId); // prune the expired slots
      return false;
    }
    this.discardOrphan(connId, assistId);
    return true;
  }

  /** `db_assist_updated` → live proposed SQL + note. */
  applyAssistUpdate(assistId: string, connId: Id, sql: string, note: string): void {
    if (!this.adoptAssist(assistId, connId)) return;
    if (sql) this.assistProposedSql = sql;
    if (note) this.assistNote = note;
  }

  /** True when an event's `assist_id` belongs to this open panel — either it
   *  already IS our assist, or a turn is IN FLIGHT for this connection with no id
   *  yet (the server minted the id mid-POST) and we adopt it. */
  private adoptAssist(assistId: string, connId: Id): boolean {
    if (!this.assistOpen) return false;
    if (assistId === this.assistId) return true;
    if ((this.assistBusy || this.awaitingId) && this.assistId === null && connId === this.assistConnId) {
      this.assistId = assistId;
      this.awaitingId = false;
      return true;
    }
    return false;
  }

  /** Insert the agent's proposed SQL into the active query tab. Replaces the
   *  editor's text, so a non-empty, different buffer asks first — the person's
   *  own SQL is never overwritten silently. Resolves false when kept. */
  async insertAssistSql(): Promise<boolean> {
    const sql = this.assistProposedSql;
    if (!sql.trim()) return false;
    const current = this.host.currentStatement();
    if (current.trim() && current.trim() !== sql.trim()) {
      const ok = await confirmer.ask(
        'The agent’s query will replace what is in the editor now. Your current text is not saved anywhere else.',
        { title: 'Replace the current query?', confirmLabel: 'Replace', cancelLabel: 'Keep mine', danger: false },
      );
      if (!ok) return false;
    }
    this.host.insertIntoQuery(sql);
    return true;
  }

  /** Insert the agent's proposed SQL and run it via the normal run path. The
   *  run is READ-ONLY: a write the agent proposed is refused by the server and
   *  only goes ahead after an attributed confirm (guarded connections take the
   *  typed confirm) — agent SQL can never silently write. */
  async runAssistSql(): Promise<void> {
    if (!(await this.insertAssistSql())) return;
    const who = this.assistProvider || 'Agent';
    await this.host.runReadOnly(`${who} · DB assistant`, () =>
      confirmer.ask(`${who} proposed a query that writes to this database. Run it anyway?`, {
        title: 'Allow this write?',
        confirmLabel: 'Run write',
        danger: true,
      }),
    );
  }

  /** Summarize the investigation → download the returned markdown. */
  async summarizeAssist(): Promise<void> {
    const id = this.assistConnId ?? this.host.selectedConnId();
    if (!id || !this.assistId) {
      toasts.error('Nothing to summarize yet');
      return;
    }
    this.assistBusy = true;
    const gen = this.gen;
    const aid = this.assistId;
    try {
      const resp = await dbAssistSummary(id, aid);
      downloadText(resp.markdown, `db-assist-${aid}.md`, 'text/markdown');
      toasts.success('Summary downloaded');
    } catch (e) {
      toastError('Couldn’t summarize', e);
    } finally {
      // A panel reopened meanwhile owns `assistBusy` now.
      if (gen === this.gen) this.assistBusy = false;
    }
  }

  /** Close the panel — DELETE the assist (kills the session, discards the dir). */
  async closeAssist(): Promise<void> {
    const id = this.assistConnId ?? this.host.selectedConnId();
    const aid = this.assistId;
    this.retireTurn();
    this.assistOpen = false;
    this.assistSessionId = null;
    this.assistProposedSql = '';
    this.assistNote = '';
    this.assistId = null;
    this.assistResultContext = null;
    if (id && aid) {
      try {
        await dbAssistClose(id, aid);
      } catch {
        /* best-effort discard — the session is reaped on daemon restart anyway */
      }
    }
  }
}
