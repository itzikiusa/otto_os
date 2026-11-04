// Side chats (`/btw`, T3 Code / PandaCode "By the way"): a question about the
// main thread answered in its own session, so the main turn is never
// interrupted. The side session is an ordinary agent session tagged
// `meta.side_of = <main id>` — hidden from the thread lists (isThreadSession)
// until promoted, which just clears the tag. Plan mode by default: it reads,
// it doesn't edit the files the main thread is working on.
import { api } from '../../lib/api/client';
import type { Session, Transcript, Turn } from '../../lib/api/types';
import { resolveSessionThreadConfig, supportsInteractionMode } from '../../lib/providerModels';
import { defaultAgentProvider } from '../../lib/providers';
import { git } from '../../lib/stores/git.svelte';
import { transcript } from '../../lib/stores/transcript.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { buildSideChatPrompt } from './sideChatPrompt';
import { isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
import { threadRepos } from './threadRepos.svelte';
import { harnessLabel, isPlainShellSession, sideChatOf } from './threadsHelpers';
import { threads } from './threadsStore.svelte';

/** The live side chat of `mainId`, newest first. */
export function sideChatFor(mainId: string | null): Session | null {
  if (!mainId) return null;
  const list = ws.sessions.filter((s) => !s.archived && sideChatOf(s) === mainId);
  return list.sort((a, b) => b.created_at.localeCompare(a.created_at))[0] ?? null;
}

/** A runtime thread's messages as transcript turns (the prompt reads prose only). */
function runtimeTurns(id: string): Turn[] {
  return (runtimeThreads.projections[id]?.items ?? []).flatMap((i) =>
    i.type === 'user_message' || i.type === 'assistant_message'
      ? [{ id: i.id, role: i.type === 'user_message' ? 'user' : 'assistant', ts: i.started_at, blocks: [{ kind: 'text', md: i.text }], duration_ms: null, model: null, system: [], reasoning_steps: 0 } satisfies Turn]
      : [],
  );
}

async function mainContext(main: Session): Promise<Parameters<typeof buildSideChatPrompt>[1]> {
  const repo = threadRepos.repoFor(main.cwd, main.workspace_id);
  const runtime = isRuntimeThread(main);
  const [tx, status] = await Promise.all([
    runtime ? null : api.get<Transcript>(`/sessions/${encodeURIComponent(main.id)}/transcript?limit=8`).catch(() => null),
    repo ? git.refreshStatus(repo.id).then(() => git.statusById[repo.id] ?? null, () => git.statusById[repo.id] ?? null) : null,
  ]);
  return {
    title: main.title,
    harness: harnessLabel(main.provider),
    cwd: main.cwd,
    branch: status?.branch ?? null,
    changedFiles: (status?.changes ?? []).map((c) => c.path),
    turns: runtime ? runtimeTurns(main.id) : (tx?.turns ?? []),
  };
}

/** Start `main`'s side chat with `question`; returns the new session. */
export async function startSideChat(main: Session, question: string): Promise<Session | null> {
  const q = question.trim();
  if (!q) return null;
  try {
    const provider = isPlainShellSession(main) ? defaultAgentProvider() : main.provider;
    const cfg = resolveSessionThreadConfig(main, null);
    const meta: Record<string, unknown> = {
      surface: 'threads',
      side_of: main.id,
      ...(supportsInteractionMode(provider) ? { interaction_mode: 'plan' } : {}),
      ...(provider === main.provider && cfg.pinnedModel ? { model: cfg.pinnedModel } : {}),
      ...(provider === main.provider && cfg.effort ? { effort: cfg.effort } : {}),
    };
    const prompt = buildSideChatPrompt(q, await mainContext(main));
    const created = await ws.openSessionWithPrompt(
      { provider, title: `Side: ${q.replace(/\s+/g, ' ').slice(0, 48)}`, cwd: main.cwd, prompt, meta },
      { scratch: main.workspace_id === SCRATCH_WORKSPACE_ID && ws.currentId !== SCRATCH_WORKSPACE_ID },
    );
    transcript.recordOptimisticPrompt(created.id, prompt);
    return created;
  } catch (e) {
    toastError('Couldn’t start the side chat', e);
    return null;
  }
}

/** Make the side chat a normal thread and open it beside its main thread. */
export async function promoteSideChat(side: Session): Promise<void> {
  try {
    await ws.updateSessionMeta(side.id, { side_of: null });
    threads.openBeside(side.id);
  } catch (e) {
    toastError('Couldn’t open the side chat as a thread', e);
  }
}

/** Archive the side chat (asks first while it is working). */
export async function dismissSideChat(side: Session): Promise<void> {
  await ws.requestArchive(side.id);
}
