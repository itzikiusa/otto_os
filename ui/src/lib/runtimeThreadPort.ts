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
//     waiting) | 'background' (idle, background work still running: calm, not
//     "working"; a host that doesn't know it treats it as idle) | 'closed' —
//     so a folded host header can show a dot without reading the Threads stores.
//   - `hostCommands`: the view-level slash commands the host can carry out.
//     A command with no handler is hidden from the slash popup, and typing it
//     sends it to the agent like any other `/command` (never a silent no-op).
//     `/clear` (fresh context) and `/review` (a prompt) belong to the thread
//     and always work. Threads is just another host (runtime/threadsHost.ts).
//   - `draft` (with `sessionId: null`): no thread yet — the embed renders the
//     Threads start composer (provider · model · effort, access, attach, `@`,
//     `/`) under the host's lead line and suggestion chips, starts the thread
//     through the Threads store and hands it back (`draft.onstarted`).
//   - `onhandle(handle)`: the embed's send for host-initiated asks (a menu, a
//     file, a line pick): before a thread it starts one with the composer's
//     picks, after it reopens a closed thread and queues behind a running
//     turn — always through the Threads store, so the access chip, the closed
//     state and the working dot stay true.
//   - `contextOnFresh()`: text put before the first message after `/clear`,
//     so the agent knows again what it is looking at.
import type { Component, Snippet } from 'svelte';
import type {
  ApprovalDecision,
  AskChildReq,
  AskChildTarget,
  DiffResp,
  EmbeddedOrigin,
  EmbeddedThreadSource,
  MetaResp,
  PlanArtifact,
  PromptImage,
  ProviderSession,
  ProviderThread,
  ProviderTurn,
  QueuedPrompt,
  RuntimeAccessMode,
  RuntimePromptReq,
  RuntimePromptResp,
  RuntimeProviderOrSlug,
  RuntimeRequest,
  RuntimeSession,
  TurnItem,
} from './api/types';
import type { EffortLevel } from './providerModels';
import { router } from './router.svelte';
import type { ComposerSlashCommand } from './slashActions';
import type { ComposerFile } from './stores/runtimeDrafts.svelte';

/** Shared conversation Svelte context key + preview payload contract for hosts
 *  (IDE Agent dock, Threads inspector) that intercept file/code links. */
export const CONV_CTX = 'conv';

export type PreviewReq =
  | {
      kind: 'file';
      path: string;
      line?: number | null;
      content?: string | null;
      diff?: DiffResp | null;
    }
  | { kind: 'code'; title: string; text: string; lang: string | null; file?: string | null }
  | { kind: 'diff'; title: string; diff: DiffResp; focus?: string | null };

/** Structural projection view of a structured runtime session. */
export interface RuntimeProjectionView {
  lastSequence: number;
  session: ProviderSession | null;
  thread: ProviderThread | null;
  turns: ProviderTurn[];
  items: TurnItem[];
  requests: RuntimeRequest[];
  plans: PlanArtifact[];
  queued: QueuedPrompt[];
  output: Record<string, string>;
}

/** Optional `data-testid` overrides for embedded runtime thread composers. */
export interface RuntimeComposerTestIds {
  composer?: string;
  send?: string;
  stop?: string;
  model?: string;
  attach?: string;
}

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
export type RuntimeEmbedStatus = 'idle' | 'working' | 'needs-you' | 'background' | 'closed';

/** A suggestion chip above an embed's start composer: `prompt()` is the
 *  whole first message (the host's context included — no `transformFirstPrompt`). */
export interface RuntimeEmbedSuggestion {
  label: string;
  prompt: () => string | Promise<string>;
}

/** An embed with no thread yet (`sessionId: null`): what its start composer
 *  starts and how. */
export interface RuntimeEmbedDraft {
  workspaceId: string;
  /** Fixed: an embed has no folder picker (null: the workspace's root). */
  cwd: string | null;
  /** The thread's title (titleAuto stays off: the host's name is the point). */
  title: string;
  source?: EmbeddedThreadSource;
  origin?: EmbeddedOrigin;
  /** The access chip's first value (PR question / diagnosis: approval-required). */
  defaultAccess: RuntimeAccessMode;
  /** Asked when the person picks a mode other than `defaultAccess`; false keeps the old one. */
  confirmAccess?: (mode: RuntimeAccessMode) => Promise<boolean>;
  suggestions?: readonly RuntimeEmbedSuggestion[];
  placeholder?: string;
  /** One line (or more) above the composer: what the agent will do, and where. */
  lead?: Snippet;
  /** Wraps a TYPED first message (PR: its context block). */
  transformFirstPrompt?: (text: string) => string | Promise<string>;
  /** The thread exists (its first message is sent): the host keeps its id. */
  onstarted: (session: RuntimeSession) => void;
}

/** What `onhandle` gives the host: one send for its own asks. */
export interface RuntimeEmbedHandle {
  /** `text` is the whole message. `accessMode` applies from this turn on (the
   *  chip follows). Resolves once sent or queued; rejects with the reason. */
  send(text: string, opts?: { accessMode?: RuntimeAccessMode }): Promise<void>;
}

export type RuntimeThreadViewProps = {
  /** Null with `draft`: no thread yet (the start composer). */
  sessionId: string | null;
  workspaceId: string;
  draft?: RuntimeEmbedDraft;
  onhandle?: (handle: RuntimeEmbedHandle | null) => void;
  contextOnFresh?: () => string | null;
  agentName?: string;
  focusOnMount?: boolean;
  compact?: boolean;
  onmaximize?: () => void;
  ondetach?: (sessionId: string) => void;
  onstatus?: (status: RuntimeEmbedStatus) => void;
  hostCommands?: RuntimeHostCommands;
  /** Host-controlled access-mode override (e.g. IDE Plan mode `'approval-required'`). */
  accessModeOverride?: RuntimeAccessMode | null;
  /** Notifies the host when the user picks an access mode inside the embed chip. */
  onaccessmodechange?: (mode: RuntimeAccessMode) => void;
  hideEmbedBarWhenUnlinked?: boolean;
  fallbackCwd?: string | null;
  hasExternalContext?: boolean;
  /** `fresh`: the message starts a fresh context (`/clear`). */
  transformOutgoingText?: (text: string, fresh?: boolean) => string;
  transformRestoredQueuedText?: (text: string) => string;
  onExternalContextConsumed?: () => void;
  onopenpath?: (path: string, line?: number) => void;
  formatAssistantText?: (item: Extract<TurnItem, { type: 'assistant_message' }>) => string;
  assistantFooter?: Snippet<[Extract<TurnItem, { type: 'assistant_message' }>]>;
  composerTopSlot?: Snippet<[{ focusComposer: () => void }]>;
  composerFootLeadSlot?: Snippet<[{ focusComposer: () => void }]>;
  testIds?: RuntimeComposerTestIds;
};
/** The props an embed falls back to when the host leaves them out. */
export const EMBED_DEFAULTS = { focusOnMount: false, compact: false } as const;

export interface PtyConversationViewProps {
  sessionId: string;
  workspaceId: string;
  readonly?: boolean;
  resumeOnOpen?: boolean;
  surface?: 'agents' | 'threads';
  ontoggleterminal?: () => void;
  onopeninspector?: (tab: 'changes' | 'files') => void;
  onnewthread?: (opts: {
    beside: boolean;
    prompt?: string;
    mode?: 'full' | 'accept-edits' | 'plan';
  }) => void;
  onsidechat?: (question: string) => void;
  branchHint?: string | null;
}

export interface RuntimeMentionPickedFile {
  path: string;
  display: string;
  insertToken: string;
  isDir: boolean;
  nextText: string;
  nextCaret: number;
}

export interface RuntimeMentionPickedAgent {
  option: { provider: string; model?: string | null; label: string };
  nextText: string;
  nextCaret: number;
}

export interface RuntimeMentionMenuProps {
  text: string;
  cwd?: string | null;
  textareaEl?: HTMLTextAreaElement | null;
  anchorEl?: HTMLElement | null;
  allowAgents?: boolean;
  agentSectionLabel?: string;
  onpickfile: (picked: RuntimeMentionPickedFile) => void;
  onpickagent?: (picked: RuntimeMentionPickedAgent) => void;
  ondrilldir: (nextText: string, nextCaret: number) => void;
}

export type RuntimeMentionMenuComponent = Component<
  RuntimeMentionMenuProps,
  { onKey: (e: KeyboardEvent) => boolean }
>;

export interface IdeCouncilSeatInput {
  provider: string;
  model: string | null;
  effort: string | null;
}

export type IdeAskTarget = AskChildTarget;

export interface RuntimeComposerUiPort {
  council: {
    on: boolean;
    seats: IdeCouncilSeatInput[];
    readonly ready: boolean;
    watchModifiers(seed: IdeCouncilSeatInput): () => void;
    pick(seat: IdeCouncilSeatInput): void;
    start(
      workspaceId: string,
      req: { prompt: string; title: string; cwd: string | null; images?: PromptImage[] },
    ): Promise<RuntimeSession[]>;
  };
  councilAgentLabel(meta?: MetaResp | null): string;
  askTargets: {
    for(sessionId: string): IdeAskTarget[];
    clear(sessionId: string): void;
  };
  CouncilStagePicker: Component<{
    part: 'toggle' | 'seats';
    provider: string;
    model: string | null;
    effort: EffortLevel | null;
    disabled?: boolean;
  }>;
  RuntimeAccessChip: Component<{
    mode: RuntimeAccessMode;
    agent: string;
    cwd: string;
    disabled?: boolean;
    onpick: (mode: RuntimeAccessMode) => void;
  }>;
  RuntimeComposerImages: Component<{
    images: ComposerFile[];
    onremove: (index: number) => void;
  }>;
  RuntimeMentionMenu: RuntimeMentionMenuComponent;
  StageSlashPopup: Component<{
    id: string;
    items: readonly ComposerSlashCommand[];
    active: number;
    onaccept: (c: ComposerSlashCommand) => void;
    onhover: (i: number) => void;
  }>;
}

export interface RuntimeSessionPort {
  runtimeThreads: {
    readonly byId: Record<string, RuntimeSession>;
    get(id: string | null | undefined): RuntimeSession | null;
    live(closed?: boolean): RuntimeSession[];
    mount(): () => void;
    watch(id: string): () => void;
    load(workspaceId: string): Promise<void>;
    loadAll(): Promise<void>;
    loadOne(id: string): Promise<RuntimeSession>;
    loadLinked(id: string): Promise<void>;
    start(
      workspaceId: string,
      req: {
        provider: RuntimeProviderOrSlug;
        cwd: string | null;
        title: string;
        model: string | null;
        effort?: string;
        titleAuto?: boolean;
        plan?: boolean;
        prompt: string;
        attachments?: readonly { name: string; mime: string; data_base64: string }[];
        accessMode?: RuntimeAccessMode;
        source?: EmbeddedThreadSource;
        origin?: EmbeddedOrigin;
      },
    ): Promise<RuntimeSession>;
    /** The embedded thread for an origin: open before closed, then newest. */
    findByOrigin(
      workspaceId: string,
      source: string,
      originId: string,
      rememberedId?: string | null,
    ): Promise<RuntimeSession | null>;
    reopen(id: string): Promise<RuntimeSession>;
    close(id: string): Promise<void>;
    prompt: (
      id: string,
      req: RuntimePromptReq,
      localAttachments?: readonly ComposerFile[],
    ) => Promise<RuntimePromptResp>;
    /** Stop the turn (the store may answer with what it stopped; hosts only await it). */
    interrupt(id: string): Promise<unknown>;
    respond(id: string, requestId: string, decision: ApprovalDecision): Promise<void>;
    askChildren(parentId: string, req: AskChildReq): Promise<RuntimeSession[]>;
  };
  runtimeViews: {
    readonly projections: Record<string, RuntimeProjectionView | undefined>;
    open(id: string): () => void;
  };
  threads: {
    rememberProject(cwd: string): void;
    selectThread(id: string | null, opts?: { syncRoute?: boolean }): void;
    startNewThread(opts?: { workspaceId?: string | null; cwd?: string | null }): void;
  };
  readAttachments(files: Iterable<File>, takes: (file: File) => boolean): Promise<ComposerFile[]>;
  loadComposerUi?: () => Promise<RuntimeComposerUiPort>;
}

/** Backward-compatible alias for shell boot registration and callers migrating to `RuntimeSessionPort`. */
export type IdeRuntimeBridge = RuntimeSessionPort;

type RuntimeThreadViewModule = { default: Component<RuntimeThreadViewProps> };
type PtyConversationViewModule = { default: Component<PtyConversationViewProps> };
type RuntimeMentionMenuModule = { default: RuntimeMentionMenuComponent };

let loader: (() => Promise<RuntimeThreadViewModule>) | null = null;
let ptyLoader: (() => Promise<PtyConversationViewModule>) | null = null;
let mentionLoader: (() => Promise<RuntimeMentionMenuModule>) | null = null;
let sessionPortLoader: (() => Promise<RuntimeSessionPort>) | null = null;
let sessionPortInstance: RuntimeSessionPort | null = null;
let sessionPortPromise: Promise<RuntimeSessionPort> | null = null;
let composerUiLoader: (() => Promise<RuntimeComposerUiPort>) | null = null;
let composerUiInstance: RuntimeComposerUiPort | null = null;
let composerUiPromise: Promise<RuntimeComposerUiPort> | null = null;

export function registerRuntimeThreadView(load: () => Promise<RuntimeThreadViewModule>): void {
  loader = load;
}

export function loadRuntimeThreadView(): Promise<RuntimeThreadViewModule> {
  if (!loader) return Promise.reject(new Error('Threads aren’t available in this window'));
  return loader();
}

export function registerPtyConversationView(load: () => Promise<PtyConversationViewModule>): void {
  ptyLoader = load;
}

export function loadPtyConversationView(): Promise<PtyConversationViewModule> {
  if (!ptyLoader) return Promise.reject(new Error('Conversation view isn’t available in this window'));
  return ptyLoader();
}

export function registerRuntimeMentionMenu(load: () => Promise<RuntimeMentionMenuModule>): void {
  mentionLoader = load;
}

export function loadRuntimeMentionMenu(): Promise<RuntimeMentionMenuModule> {
  if (!mentionLoader) return Promise.reject(new Error('Mentions aren’t available in this window'));
  return mentionLoader();
}

export function registerIdeRuntimeSessionPort(load: () => Promise<RuntimeSessionPort>): void {
  sessionPortLoader = load;
  sessionPortInstance = null;
  sessionPortPromise = null;
}

export function getIdeRuntimeSessionPort(): RuntimeSessionPort | null {
  return sessionPortInstance;
}

export function loadIdeRuntimeSessionPort(): Promise<RuntimeSessionPort> {
  if (sessionPortInstance) return Promise.resolve(sessionPortInstance);
  if (sessionPortPromise) return sessionPortPromise;
  if (!sessionPortLoader) {
    return Promise.reject(new Error('Runtime threads aren’t available in this window'));
  }
  sessionPortPromise = sessionPortLoader()
    .then((mod) => {
      sessionPortInstance = mod;
      if (!composerUiLoader && mod.loadComposerUi) {
        composerUiLoader = mod.loadComposerUi;
      }
      return mod;
    })
    .finally(() => {
      sessionPortPromise = null;
    });
  return sessionPortPromise;
}

export function registerIdeRuntimeComposerUi(load: () => Promise<RuntimeComposerUiPort>): void {
  composerUiLoader = load;
  composerUiInstance = null;
  composerUiPromise = null;
}

export function getIdeRuntimeComposerUi(): RuntimeComposerUiPort | null {
  return composerUiInstance;
}

export function loadIdeRuntimeComposerUi(): Promise<RuntimeComposerUiPort> {
  if (composerUiInstance) return Promise.resolve(composerUiInstance);
  if (composerUiPromise) return composerUiPromise;
  const resolveLoader = async (): Promise<RuntimeComposerUiPort> => {
    if (composerUiLoader) return composerUiLoader();
    const sessionPort = await loadIdeRuntimeSessionPort();
    const viaSession = composerUiLoader ?? sessionPort.loadComposerUi;
    if (!viaSession) {
      throw new Error('Runtime composer UI isn’t available in this window');
    }
    return viaSession();
  };
  composerUiPromise = resolveLoader()
    .then((mod) => {
      composerUiInstance = mod;
      return mod;
    })
    .finally(() => {
      composerUiPromise = null;
    });
  return composerUiPromise;
}

export const registerIdeRuntimeBridge = registerIdeRuntimeSessionPort;
export const getIdeRuntimeBridge = getIdeRuntimeSessionPort;
export const loadIdeRuntimeBridge = loadIdeRuntimeSessionPort;

/** The default detach: the thread in the Threads page. */
export function openInThreads(sessionId: string): void {
  router.go(`threads/${sessionId}`);
}
