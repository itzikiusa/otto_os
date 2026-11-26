// The Workflows module's addresses (ux.md §1.1): `#/workflows` (Runs),
// `#/workflows/all` (the definitions), `#/workflows/<wf>` (the editor),
// `#/workflows/<wf>/optimize` and `#/workflows/<wf>/runs/<run>[/<node>[/<child>]]`
// (the run page — the run header's "Copy link" and every notice hand it out). Also where a run came from,
// read from its input (a Slack/Telegram trigger stamps channel/chat/thread and
// the triggering message `raw`). Pure; unit-tested in unit/workflowRunRoute.test.ts.

export interface RunRoute {
  workflowId: string;
  runId: string;
  /** The step open on the run page (`…/runs/<run>/<node>`). */
  nodeId?: string;
  /** A reviewer of that step (`…/<node>/<child>`). */
  childId?: string;
}

/** What `#/workflows/…` shows. `all` cannot be a workflow id (ids are ULIDs). */
export type WorkflowsView =
  | { view: 'runs' }
  | { view: 'list' }
  | { view: 'editor'; workflowId: string }
  | { view: 'optimize'; workflowId: string }
  | ({ view: 'run' } & RunRoute);

/** router.parts → the screen. */
export function parseWorkflowsRoute(parts: readonly string[]): WorkflowsView {
  const run = parseRunRoute(parts);
  if (run) return { view: 'run', ...run };
  const [, id, sub] = parts;
  if (!id) return { view: 'runs' };
  if (id === 'all') return { view: 'list' };
  return sub === 'optimize' ? { view: 'optimize', workflowId: id } : { view: 'editor', workflowId: id };
}

/** `['workflows', wf, 'runs', run, node?, child?]` (router.parts) → the run it names. */
export function parseRunRoute(parts: readonly string[]): RunRoute | null {
  const [mod, workflowId, sub, runId, nodeId, childId] = parts;
  if (mod !== 'workflows' || !workflowId || sub !== 'runs' || !runId) return null;
  return { workflowId, runId, ...(nodeId ? { nodeId } : {}), ...(nodeId && childId ? { childId } : {}) };
}

/** The router path of a run, or of one of its steps (no leading `#/`). */
export function runRoutePath(workflowId: string, runId: string, nodeId?: string | null, childId?: string | null): string {
  const step = nodeId ? `/${encodeURIComponent(nodeId)}${childId ? `/${encodeURIComponent(childId)}` : ''}` : '';
  return `workflows/${encodeURIComponent(workflowId)}/runs/${encodeURIComponent(runId)}${step}`;
}

/** The absolute link to a run on this Otto (`origin` + `pathname` of the page
 *  it is copied from, so it opens wherever this UI is served). */
export function runDeepLink(loc: { origin: string; pathname: string }, workflowId: string, runId: string): string {
  return `${loc.origin}${loc.pathname}#/${runRoutePath(workflowId, runId)}`;
}

export interface RunOrigin {
  /** "Slack" | "Telegram". */
  source: string;
  /** The channel/chat id the message came from. */
  chat: string;
  thread: string | null;
  user: string | null;
  /** The triggering message as received. */
  raw: string | null;
}

const SOURCES: Record<string, string> = { slack: 'Slack', telegram: 'Telegram' };

/** Where a chat-started run came from, or null for any other run. */
export function runOrigin(input: unknown): RunOrigin | null {
  if (!input || typeof input !== 'object') return null;
  const o = input as Record<string, unknown>;
  const str = (k: string): string | null => (typeof o[k] === 'string' && (o[k] as string).trim() ? (o[k] as string) : null);
  const source = SOURCES[str('channel') ?? ''];
  const chat = str('chat');
  if (!source || !chat) return null;
  return { source, chat, thread: str('thread'), user: str('user'), raw: str('raw') };
}
