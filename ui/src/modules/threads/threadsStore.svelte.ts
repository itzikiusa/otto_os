import { api } from '../../lib/api/client';
import { onLive } from '../../lib/live';
import { plural } from '../../lib/plural';
import { router } from '../../lib/router.svelte';
import { openFile } from '../../lib/stores/openfile.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import type { BroadcastResp, TrailEvent } from '../../lib/api/types';

export type ThreadsLayoutMode = 'single' | 'split' | 'grid';
export type ThreadsInspectorTab = 'files' | 'changes' | 'connections' | 'peers';

export interface OpenFileTab {
  path: string;
  line?: number | null;
  pinned?: boolean;
}

export interface WorkspaceFileTabs {
  tabs: OpenFileTab[];
  activeFilePath: string | null;
}

export interface PeerTimelineItem {
  id: string;
  ts: string;
  kind: 'message' | 'broadcast';
  fromSessionId: string | null;
  fromTitle: string;
  toSessionId: string | null;
  toTitle: string;
  summary: string;
}

export interface McpDbActivity {
  activity_id: string;
  connection_id?: string | null;
  session_id?: string | null;
  session_title?: string | null;
  tool: string;
  sql: string;
  status: string;
  row_count?: number | null;
  duration_ms?: number | null;
  columns?: string[];
  rows_preview?: unknown[][];
  error?: string | null;
  ts: string;
}

interface PersistedThreadsPrefs {
  pinnedIds?: string[];
  settledIds?: string[];
  paneIds?: string[];
  focusedSessionId?: string | null;
  layoutMode?: ThreadsLayoutMode;
  sidebarOpen?: boolean;
  inspectorOpen?: boolean;
  inspectorTab?: ThreadsInspectorTab;
  openFilesByWorkspace?: Record<string, WorkspaceFileTabs>;
}

const STORAGE_KEY = 'otto_threads_v1';
const MAX_PANES = 4;
const MAX_MCP_ACTIVITIES = 60;
const MAX_PEER_EVENTS = 80;

function readPrefs(): PersistedThreadsPrefs {
  if (typeof localStorage === 'undefined') return {};
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as PersistedThreadsPrefs) : {};
  } catch {
    return {};
  }
}

class ThreadsStore {
  pinnedIds = $state<string[]>([]);
  settledIds = $state<string[]>([]);
  paneIds = $state<string[]>([]);
  focusedSessionId = $state<string | null>(null);
  layoutMode = $state<ThreadsLayoutMode>('single');
  newDraftMode = $state(false);
  draftWorkspaceId = $state<string | null>(null);
  draftCwd = $state<string | null>(null);
  sidebarOpen = $state(true);
  inspectorOpen = $state(true);
  inspectorTab = $state<ThreadsInspectorTab>('files');
  openFilesByWorkspace = $state<Record<string, WorkspaceFileTabs>>({});
  projectFilter = $state('all');

  peerEvents = $state<PeerTimelineItem[]>([]);
  mcpActivities = $state<McpDbActivity[]>([]);
  peerRelayTarget = $state<string>('broadcast');

  private liveRefs = 0;
  private stopLive: (() => void) | null = null;
  private seededTrailWs: string | null = null;

  constructor() {
    const p = readPrefs();
    if (Array.isArray(p.pinnedIds)) this.pinnedIds = p.pinnedIds.filter((x): x is string => typeof x === 'string');
    if (Array.isArray(p.settledIds)) this.settledIds = p.settledIds.filter((x): x is string => typeof x === 'string');
    if (Array.isArray(p.paneIds)) {
      this.paneIds = p.paneIds.filter((x): x is string => typeof x === 'string').slice(0, MAX_PANES);
    }
    if (typeof p.focusedSessionId === 'string') this.focusedSessionId = p.focusedSessionId;
    if (p.layoutMode === 'single' || p.layoutMode === 'split' || p.layoutMode === 'grid') {
      this.layoutMode = p.layoutMode;
    }
    if (typeof p.sidebarOpen === 'boolean') this.sidebarOpen = p.sidebarOpen;
    if (typeof p.inspectorOpen === 'boolean') this.inspectorOpen = p.inspectorOpen;
    if (
      p.inspectorTab === 'files' ||
      p.inspectorTab === 'changes' ||
      p.inspectorTab === 'connections' ||
      p.inspectorTab === 'peers'
    ) {
      this.inspectorTab = p.inspectorTab;
    }
    if (p.openFilesByWorkspace && typeof p.openFilesByWorkspace === 'object') {
      this.openFilesByWorkspace = p.openFilesByWorkspace;
    }
  }

  private save(): void {
    if (typeof localStorage === 'undefined') return;
    try {
      const payload: PersistedThreadsPrefs = {
        pinnedIds: this.pinnedIds,
        settledIds: this.settledIds,
        paneIds: this.paneIds,
        focusedSessionId: this.focusedSessionId,
        layoutMode: this.layoutMode,
        sidebarOpen: this.sidebarOpen,
        inspectorOpen: this.inspectorOpen,
        inspectorTab: this.inspectorTab,
        openFilesByWorkspace: this.openFilesByWorkspace,
      };
      localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
    } catch {
      /* ignore quota / private mode */
    }
  }

  /** Subscribe to live cross-session messages and MCP DB query telemetry while Threads is mounted. */
  mountLive(): () => void {
    this.liveRefs++;
    if (!this.stopLive) {
      this.stopLive = onLive(['trail_appended', 'db_mcp_activity'], (ev) => {
        if (ev.type === 'trail_appended') {
          this.ingestTrailEntry((ev as unknown as { event?: TrailEvent }).event);
        } else if (ev.type === 'db_mcp_activity') {
          this.ingestMcpActivity(ev as unknown as Record<string, unknown>);
        }
      });
    }
    return () => {
      this.liveRefs = Math.max(0, this.liveRefs - 1);
      if (this.liveRefs === 0 && this.stopLive) {
        this.stopLive();
        this.stopLive = null;
      }
    };
  }

  /** Seed recent cross-session messages from workspace trail once per workspace. */
  async seedWorkspaceTrail(workspaceId: string | null): Promise<void> {
    if (!workspaceId || this.seededTrailWs === workspaceId) return;
    this.seededTrailWs = workspaceId;
    try {
      const list = await api.get<TrailEvent[]>(`/workspaces/${workspaceId}/trail?limit=60`);
      for (const entry of [...list].reverse()) {
        this.ingestTrailEntry(entry);
      }
    } catch {
      /* trail endpoint optional on restricted roles */
    }
  }

  private ingestTrailEntry(entry: TrailEvent | undefined): void {
    if (!entry) return;
    const rawKind = String(entry.kind);
    const det = (entry.detail && typeof entry.detail === 'object' ? entry.detail : {}) as Record<string, unknown>;
    const isBroadcast = rawKind === 'broadcast' || det.broadcast === true || /^\[Broadcast/i.test(entry.summary);
    const isPeerMsg =
      isBroadcast ||
      rawKind === 'message' ||
      typeof det.sender === 'string' ||
      typeof det.from_session_id === 'string' ||
      /^\[(Message from|From )/i.test(entry.summary);
    if (!isPeerMsg) return;

    const itemId = `trail-${entry.id}`;
    if (this.peerEvents.some((x) => x.id === itemId)) return;
    const fromId = typeof det.from_session_id === 'string' ? det.from_session_id : null;
    const senderLabel =
      typeof det.sender === 'string' && det.sender.trim() !== ''
        ? det.sender
        : fromId
          ? (ws.getSession(fromId)?.title ?? 'Peer thread')
          : 'Operator';
    const toTitle = isBroadcast
      ? 'All active threads'
      : entry.session_id
        ? (ws.getSession(entry.session_id)?.title ?? 'Thread')
        : 'Thread';
    const item: PeerTimelineItem = {
      id: itemId,
      ts: entry.ts,
      kind: isBroadcast ? 'broadcast' : 'message',
      fromSessionId: fromId,
      fromTitle: senderLabel,
      toSessionId: entry.session_id || null,
      toTitle,
      summary: entry.summary,
    };
    const base = this.peerEvents.filter(
      (x) =>
        !(
          x.id.startsWith('local-') &&
          x.kind === item.kind &&
          x.toSessionId === item.toSessionId &&
          Math.abs(Date.now() - Date.parse(x.ts)) < 5000
        ),
    );
    if (fromId === null) {
      const localMatch = this.peerEvents.find(
        (x) => x.id.startsWith('local-') && x.kind === item.kind && x.toSessionId === item.toSessionId,
      );
      if (localMatch?.fromSessionId) item.fromSessionId = localMatch.fromSessionId;
    }
    this.peerEvents = [item, ...base].slice(0, MAX_PEER_EVENTS);
  }

  private ingestMcpActivity(raw: Record<string, unknown>): void {
    const id = typeof raw.activity_id === 'string' ? raw.activity_id : `mcp-${Date.now()}-${Math.random()}`;
    const item: McpDbActivity = {
      activity_id: id,
      connection_id: typeof raw.connection_id === 'string' ? raw.connection_id : null,
      session_id: typeof raw.session_id === 'string' ? raw.session_id : null,
      session_title: typeof raw.session_title === 'string' ? raw.session_title : null,
      tool: typeof raw.tool === 'string' ? raw.tool : 'db.query',
      sql: typeof raw.sql === 'string' ? raw.sql : typeof raw.query === 'string' ? raw.query : '',
      status: typeof raw.status === 'string' ? raw.status : 'ok',
      row_count: typeof raw.row_count === 'number' ? raw.row_count : null,
      duration_ms: typeof raw.duration_ms === 'number' ? raw.duration_ms : null,
      columns: Array.isArray(raw.columns) ? raw.columns.map(String) : undefined,
      rows_preview: Array.isArray(raw.rows_preview) ? (raw.rows_preview as unknown[][]) : undefined,
      error: typeof raw.error === 'string' ? raw.error : null,
      ts: typeof raw.ts === 'string' ? raw.ts : new Date().toISOString(),
    };
    const idx = this.mcpActivities.findIndex((x) => x.activity_id === id);
    if (idx >= 0) {
      const next = [...this.mcpActivities];
      next[idx] = { ...next[idx], ...item };
      this.mcpActivities = next;
    } else {
      this.mcpActivities = [item, ...this.mcpActivities].slice(0, MAX_MCP_ACTIVITIES);
    }
  }

  isPinned(id: string): boolean {
    return this.pinnedIds.includes(id);
  }

  togglePin(id: string): void {
    if (this.pinnedIds.includes(id)) {
      this.pinnedIds = this.pinnedIds.filter((x) => x !== id);
    } else {
      this.pinnedIds = [id, ...this.pinnedIds];
    }
    this.save();
  }

  isSettled(id: string): boolean {
    return this.settledIds.includes(id);
  }

  toggleSettle(id: string): void {
    if (this.settledIds.includes(id)) {
      this.settledIds = this.settledIds.filter((x) => x !== id);
    } else {
      this.settledIds = [id, ...this.settledIds];
    }
    this.save();
  }

  unsettle(id: string): void {
    if (!this.settledIds.includes(id)) return;
    this.settledIds = this.settledIds.filter((x) => x !== id);
    this.save();
  }

  focusPane(id: string, syncRoute = true): void {
    this.focusedSessionId = id;
    this.newDraftMode = false;
    ws.clearNeedsYou(id);
    this.save();
    if (syncRoute && router.module === 'threads' && router.parts[1] !== id) {
      router.replace(`threads/${id}`);
    }
  }

  selectThread(id: string, opts: { split?: boolean; syncRoute?: boolean } = {}): void {
    if (opts.split) {
      this.openBeside(id, opts.syncRoute ?? true);
      return;
    }
    this.newDraftMode = false;
    if (this.paneIds.includes(id)) {
      this.focusPane(id, opts.syncRoute ?? true);
      return;
    }
    if (this.layoutMode === 'single' || this.paneIds.length <= 1) {
      this.paneIds = [id];
      this.layoutMode = 'single';
    } else {
      const targetIdx = this.focusedSessionId ? this.paneIds.indexOf(this.focusedSessionId) : -1;
      const next = [...this.paneIds];
      if (targetIdx >= 0) next[targetIdx] = id;
      else next[0] = id;
      this.paneIds = next;
    }
    this.focusPane(id, opts.syncRoute ?? true);
  }

  openBeside(id: string, syncRoute = true): void {
    this.newDraftMode = false;
    if (this.paneIds.includes(id)) {
      if (this.paneIds.length === 2) this.layoutMode = 'split';
      else if (this.paneIds.length >= 3) this.layoutMode = 'grid';
      this.focusPane(id, syncRoute);
      return;
    }
    let next = [...this.paneIds, id];
    if (next.length > MAX_PANES) {
      // Keep focused pane and replace the oldest unfocused slot.
      const dropIdx = next.findIndex((p) => p !== this.focusedSessionId && p !== id);
      if (dropIdx >= 0) next.splice(dropIdx, 1);
      else next = next.slice(-MAX_PANES);
    }
    this.paneIds = next;
    this.layoutMode = next.length <= 1 ? 'single' : next.length === 2 ? 'split' : 'grid';
    this.focusPane(id, syncRoute);
  }

  openPairInSplit(idA: string, idB: string): void {
    this.newDraftMode = false;
    const unique = idA === idB ? [idA] : [idA, idB];
    this.paneIds = unique;
    this.layoutMode = unique.length === 2 ? 'split' : 'single';
    this.focusPane(idA, true);
  }

  closePane(id: string): void {
    const next = this.paneIds.filter((x) => x !== id);
    this.paneIds = next;
    if (next.length === 0) {
      this.focusedSessionId = null;
      this.layoutMode = 'single';
      this.newDraftMode = true;
      if (router.module === 'threads') router.replace('threads');
    } else {
      this.layoutMode = next.length === 1 ? 'single' : next.length === 2 ? 'split' : 'grid';
      const nextFocus = this.focusedSessionId === id ? next[next.length - 1] : (this.focusedSessionId ?? next[0]);
      this.focusPane(nextFocus, true);
    }
    this.save();
  }

  setLayout(mode: ThreadsLayoutMode, candidateIds: readonly string[] = []): void {
    this.layoutMode = mode;
    this.newDraftMode = false;
    if (mode === 'single' && this.paneIds.length > 1) {
      const keep = this.focusedSessionId && this.paneIds.includes(this.focusedSessionId) ? this.focusedSessionId : this.paneIds[0];
      this.paneIds = [keep];
      this.focusedSessionId = keep;
    } else if (mode === 'split') {
      if (this.paneIds.length > 2) {
        const keepFirst = this.focusedSessionId && this.paneIds.includes(this.focusedSessionId) ? this.focusedSessionId : this.paneIds[0];
        const second = this.paneIds.find((x) => x !== keepFirst) ?? keepFirst;
        this.paneIds = keepFirst === second ? [keepFirst] : [keepFirst, second];
      } else if (this.paneIds.length === 1) {
        const extra = candidateIds.find((id) => !this.paneIds.includes(id));
        if (extra) this.paneIds = [this.paneIds[0], extra];
      }
    } else if (mode === 'grid' && this.paneIds.length < 3) {
      const next = [...this.paneIds];
      for (const id of candidateIds) {
        if (next.length >= 3) break;
        if (!next.includes(id)) next.push(id);
      }
      this.paneIds = next;
    }
    this.save();
  }

  startNewThread(opts: { workspaceId?: string | null; cwd?: string | null } = {}): void {
    this.draftWorkspaceId = opts.workspaceId ?? null;
    this.draftCwd = opts.cwd ?? null;
    this.newDraftMode = true;
  }

  toggleSidebar(): void {
    this.sidebarOpen = !this.sidebarOpen;
    this.save();
  }

  toggleInspector(tab?: ThreadsInspectorTab): void {
    if (tab) {
      if (this.inspectorOpen && this.inspectorTab === tab) {
        this.inspectorOpen = false;
      } else {
        this.inspectorTab = tab;
        this.inspectorOpen = true;
      }
    } else {
      this.inspectorOpen = !this.inspectorOpen;
    }
    this.save();
  }

  setInspectorTab(tab: ThreadsInspectorTab): void {
    this.inspectorTab = tab;
    this.inspectorOpen = true;
    this.save();
  }

  openPeerComposerTo(targetSessionId: string): void {
    this.peerRelayTarget = targetSessionId;
    this.inspectorTab = 'peers';
    this.inspectorOpen = true;
    this.save();
  }

  // ── PandaCode-style Workspace File Tabs ─────────────────────────────────────

  fileState(wsKey: string): WorkspaceFileTabs {
    return this.openFilesByWorkspace[wsKey] ?? { tabs: [], activeFilePath: null };
  }

  openFileTab(wsKey: string, path: string, opts: { line?: number | null; pin?: boolean } = {}): void {
    const cur = this.fileState(wsKey);
    const existingIdx = cur.tabs.findIndex((t) => t.path === path);
    const tabs = [...cur.tabs];
    if (existingIdx >= 0) {
      tabs[existingIdx] = {
        ...tabs[existingIdx],
        line: opts.line ?? tabs[existingIdx].line ?? null,
        pinned: opts.pin ? true : tabs[existingIdx].pinned,
      };
    } else if (opts.pin) {
      tabs.push({ path, line: opts.line ?? null, pinned: true });
    } else {
      const previewIdx = tabs.findIndex((t) => !t.pinned);
      const nextTab: OpenFileTab = { path, line: opts.line ?? null, pinned: false };
      if (previewIdx >= 0) tabs[previewIdx] = nextTab;
      else tabs.push(nextTab);
    }
    this.openFilesByWorkspace = {
      ...this.openFilesByWorkspace,
      [wsKey]: { tabs, activeFilePath: path },
    };
    this.inspectorTab = 'files';
    this.inspectorOpen = true;
    this.save();
  }

  pinFileTab(wsKey: string, path: string): void {
    this.openFileTab(wsKey, path, { pin: true });
  }

  setActiveFile(wsKey: string, path: string): void {
    const cur = this.fileState(wsKey);
    if (!cur.tabs.some((t) => t.path === path)) return;
    this.openFilesByWorkspace = {
      ...this.openFilesByWorkspace,
      [wsKey]: { ...cur, activeFilePath: path },
    };
    this.save();
  }

  closeFileTab(wsKey: string, path: string): void {
    const cur = this.fileState(wsKey);
    const idx = cur.tabs.findIndex((t) => t.path === path);
    if (idx < 0) return;
    const tabs = cur.tabs.filter((t) => t.path !== path);
    let active = cur.activeFilePath;
    if (active === path) {
      active = tabs[Math.min(idx, tabs.length - 1)]?.path ?? null;
    }
    this.openFilesByWorkspace = {
      ...this.openFilesByWorkspace,
      [wsKey]: { tabs, activeFilePath: active },
    };
    this.save();
  }

  /** Consume global `openFile.request` while inside `#/threads`. */
  consumeOpenFileRequest(wsKey: string): void {
    const req = openFile.request;
    if (!req) return;
    openFile.request = null;
    this.openFileTab(wsKey, req.path, { line: req.line, pin: true });
  }

  // ── Session-to-Session Relay ────────────────────────────────────────────────

  async sendPeerMessage(opts: {
    fromSessionId: string | null;
    target: string;
    text: string;
    workspaceId: string | null;
  }): Promise<boolean> {
    const msg = opts.text.trim();
    if (!msg) return false;
    const fromSession = opts.fromSessionId ? ws.getSession(opts.fromSessionId) : null;
    const fromTitle = fromSession?.title ?? 'Operator';
    try {
      if (opts.target === 'broadcast') {
        const wid = opts.workspaceId ?? ws.currentId;
        if (!wid) {
          toasts.warn('Pick a workspace first', 'Broadcast sends to active threads in a workspace.');
          return false;
        }
        const resp = await api.post<BroadcastResp>(`/workspaces/${wid}/broadcast`, {
          text: fromSession ? `[From ${fromTitle}] ${msg}` : msg,
        });
        const count = resp.session_ids.length;
        if (count === 0) {
          toasts.warn('No live threads received it', 'Start or resume a thread first.');
          return false;
        }
        toasts.success('Broadcast delivered', `Sent to ${plural(count, 'thread')}`);
        this.recordLocalPeerEvent({
          kind: 'broadcast',
          fromSessionId: opts.fromSessionId,
          fromTitle,
          toSessionId: null,
          toTitle: 'All active threads',
          summary: msg,
        });
        return true;
      }
      const targetSession = ws.getSession(opts.target);
      await api.post(`/sessions/${encodeURIComponent(opts.target)}/message`, {
        text: fromSession ? `[From ${fromTitle}] ${msg}` : msg,
        ...(opts.fromSessionId ? { from_session_id: opts.fromSessionId } : {}),
      });
      toasts.success('Sent to thread', `Delivered to ${targetSession?.title ?? 'thread'}`);
      this.recordLocalPeerEvent({
        kind: 'message',
        fromSessionId: opts.fromSessionId,
        fromTitle,
        toSessionId: opts.target,
        toTitle: targetSession?.title ?? 'Thread',
        summary: msg,
      });
      return true;
    } catch (e) {
      toastError('Couldn’t send message to thread', e);
      return false;
    }
  }

  private recordLocalPeerEvent(p: Omit<PeerTimelineItem, 'id' | 'ts'>): void {
    const existingIdx = this.peerEvents.findIndex(
      (x) =>
        x.id.startsWith('trail-') &&
        x.kind === p.kind &&
        x.toSessionId === p.toSessionId &&
        Math.abs(Date.now() - Date.parse(x.ts)) < 5000,
    );
    if (existingIdx >= 0) {
      if (p.fromSessionId && !this.peerEvents[existingIdx].fromSessionId) {
        const next = [...this.peerEvents];
        next[existingIdx] = { ...next[existingIdx], fromSessionId: p.fromSessionId, fromTitle: p.fromTitle };
        this.peerEvents = next;
      }
      return;
    }
    const item: PeerTimelineItem = {
      ...p,
      id: `local-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      ts: new Date().toISOString(),
    };
    this.peerEvents = [item, ...this.peerEvents].slice(0, MAX_PEER_EVENTS);
  }
}

export const threads = new ThreadsStore();
