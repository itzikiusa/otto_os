// Port for the Threads conversation view of one structured-runtime thread,
// for features that host it inside their own chrome (a personal agent's
// chat; next, the pull-request workspace's column and Connections). lib/ never
// imports modules/, and one feature never reaches into another, so the shell
// binds the loader at boot (shell/installShell.svelte.ts) and hosts load it
// through here.
//
// The embed contract (docs/features/threads-structured-runtime.md, "Embedding
// a thread in another module"):
//   - `sessionId` / `workspaceId`: the runtime thread and its workspace. The
//     embed reads only that row (GET /runtime-sessions/{id}) and its linked
//     group, never the workspace's whole thread list.
//   - `agentName`: a personal agent's chat ("Message Scout…", calm access).
//   - `focusOnMount` (default false): take keyboard focus when mounted. An
//     embed beside other work must not steal the caret.
//   - `compact` (default false): denser chrome for a narrow column (≥ 280 px,
//     usable at a 390 px phone width), plus a slim title bar with Open in
//     Threads.
//   - `onmaximize`: the host grows the embed (its own layout decision). The
//     maximise button renders only when given.
//   - `ondetach(id)`: the host moves the thread out (a window, a tab). The
//     detach button renders when given; a compact embed without one offers
//     "Open in Threads" instead (`openInThreads`).
//   - `onstatus(status)`: called on mount with the thread's state and on every
//     change — 'idle' | 'working' | 'needs-you' (an approval or question is
//     waiting) | 'closed' — so a folded host header can show a dot without
//     reading the Threads stores.
//   - `hostCommands`: the view-level slash commands the host can carry out.
//     A command with no handler is hidden from the slash popup, and typing it
//     sends it to the agent like any other `/command` (never a silent no-op).
//     `/clear` (fresh context) and `/review` (a prompt) belong to the thread
//     and always work. Threads is just another host (runtime/threadsHost.ts).
import type { Component } from 'svelte';
import { router } from './router.svelte';

/** The view-level commands a host may implement (all optional). */
export interface RuntimeHostCommands {
  /** `/diff` — show the thread's changes. */
  openDiff?: (sessionId: string) => void;
  /** `/files` — browse the thread's folder. */
  openFiles?: (sessionId: string) => void;
  /** `/terminal` — open (or toggle) a terminal in the thread's folder. */
  openTerminal?: (sessionId: string) => void;
  /** `/new` (beside = false) and `/fork` (beside = true) — start another thread. */
  newThread?: (sessionId: string, beside: boolean) => void;
  /** `/btw <question>` — a side question that does not interrupt the turn. */
  sideChat?: (sessionId: string, question: string) => void;
  /** Not a slash command: the background strip's tap — show every subagent
   *  and background task (Threads: Thread details' Agents & tasks). */
  showTasks?: (sessionId: string) => void;
}

/** An embedded thread's state as its host sees it (`onstatus`). */
export type RuntimeEmbedStatus = 'idle' | 'working' | 'needs-you' | 'closed';

export type RuntimeThreadViewProps = {
  sessionId: string;
  workspaceId: string;
  agentName?: string;
  focusOnMount?: boolean;
  compact?: boolean;
  onmaximize?: () => void;
  ondetach?: (sessionId: string) => void;
  onstatus?: (status: RuntimeEmbedStatus) => void;
  hostCommands?: RuntimeHostCommands;
};
/** The props an embed falls back to when the host leaves them out. */
export const EMBED_DEFAULTS = { focusOnMount: false, compact: false } as const;

type RuntimeThreadViewModule = { default: Component<RuntimeThreadViewProps> };
let loader: (() => Promise<RuntimeThreadViewModule>) | null = null;

export function registerRuntimeThreadView(load: () => Promise<RuntimeThreadViewModule>): void {
  loader = load;
}

export function loadRuntimeThreadView(): Promise<RuntimeThreadViewModule> {
  if (!loader) return Promise.reject(new Error('Threads aren’t available in this window'));
  return loader();
}

/** The default detach: the thread in the Threads page. */
export function openInThreads(sessionId: string): void {
  router.go(`threads/${sessionId}`);
}
