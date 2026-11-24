// The New session stage's Chat (spec A2, A4): a structured thread opened as a
// chat pane on the Agents canvas, beside the terminals; its first message is
// optional. Threads' one store starts it — no second start path.
import type { MetaResp } from '../../lib/api/types';
import { harnessLabel } from '../../lib/providers';
import { isRuntimeProvider } from '../../lib/providerRegistry';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';

/** Start the thread and open it as the focused pane. False (and a toast) when it didn't start. */
export async function startChatPane(o: {
  workspaceId: string;
  provider: string;
  /** `~` expands against `home` (the daemon's $HOME), as the terminal path does. */
  cwd: string;
  home?: string | null;
  title: string;
  model: string;
  prompt: string;
  meta?: MetaResp | null;
}): Promise<boolean> {
  if (!isRuntimeProvider(o.provider, o.meta)) {
    toasts.error(`${harnessLabel(o.provider)} can’t run as a chat`, 'Start it as a Terminal instead.');
    return false;
  }
  const dir = o.home && (o.cwd === '~' || o.cwd.startsWith('~/')) ? o.home + o.cwd.slice(1) : o.cwd;
  try {
    const { runtimeThreads } = await import('../threads/runtime/runtimeThreads.svelte'); // ui-guards: allow — Threads' one store starts the thread
    const created = await runtimeThreads.start(o.workspaceId, {
      provider: o.provider,
      cwd: dir || null,
      title: o.title || `${harnessLabel(o.provider)} thread`,
      titleAuto: !o.title,
      model: o.model || null,
      prompt: o.prompt,
    });
    ws.openSession(created.id, 'thread');
    ws.navigateToSession(created.id);
    return true;
  } catch (e) {
    toastError('Couldn’t start the chat', e);
    return false;
  }
}
