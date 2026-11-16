<!--
  An embed's draft state (lib/runtimeThreadPort.ts `draft`, `sessionId: null`):
  no thread yet, so the host's lead line and suggestion chips sit above the
  Threads start composer (RuntimeStartComposer, compact) — the person picks
  the agent, model, effort and access exactly as on the Threads page. A
  start goes through the Threads store (`runtimeThreads.start`, with the
  host's source / origin and the access mode) and hands the thread back.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import type { RuntimeAccessMode } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { agentProviders, orderThreadHarnesses, providerReadiness } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import type { RuntimeEmbedDraft } from '../../../lib/runtimeThreadPort';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { toastError } from '../../../lib/toastError';
  import RuntimeStartComposer from '../RuntimeStartComposer.svelte';
  import { ACCESS_HINT, ACCESS_RANK, startHarnesses, type StartLaunch } from '../startComposer';
  import { isRuntimeProvider, runtimeThreads } from './runtimeThreads.svelte';

  let { draft }: { draft: RuntimeEmbedDraft } = $props();

  // The default comes from the host once; the person's pick then sticks
  // (a host re-render must not snap it back).
  let access = $state<RuntimeAccessMode>(untrack(() => draft.defaultAccess));
  let composer = $state<{ launchText: (text: string, access?: RuntimeAccessMode) => Promise<boolean> } | null>(null);
  let busy = $state(false);
  const runnable = $derived(
    startHarnesses(orderThreadHarnesses(agentProviders()), true, (p) => isRuntimeProvider(p, auth.meta), (p) => providerReadiness(p).available),
  );
  const draftKey = $derived(`embed-start:${draft.source ?? 'thread'}:${draft.origin?.id ?? draft.cwd}`);

  async function pickAccess(m: RuntimeAccessMode): Promise<void> {
    if (m === access) return;
    if (ACCESS_RANK[m] > ACCESS_RANK[draft.defaultAccess] && draft.confirmAccess && !(await draft.confirmAccess(m))) return;
    access = m;
  }

  async function launch(req: StartLaunch): Promise<boolean> {
    try {
      const text = req.raw || !draft.transformFirstPrompt ? req.message : await draft.transformFirstPrompt(req.message);
      const created = await runtimeThreads.start(draft.workspaceId, {
        provider: req.provider,
        cwd: draft.cwd,
        title: draft.title,
        model: req.model,
        effort: req.effort ?? undefined,
        plan: req.plan,
        prompt: text,
        attachments: req.attachments,
        accessMode: req.accessMode ?? access,
        source: draft.source,
        origin: draft.origin,
      });
      draft.onstarted(created);
      return true;
    } catch (e) {
      toastError('Couldn’t start the thread', e);
      return false;
    }
  }

  /** A host ask before the thread exists: started with the composer's picks. */
  export async function send(text: string, accessMode?: RuntimeAccessMode): Promise<boolean> {
    if (!composer) return false;
    return composer.launchText(text, accessMode);
  }

  async function suggest(message: () => string | Promise<string>): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      await send(await message());
    } catch (e) {
      toastError('Couldn’t start the thread', e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="ed" data-testid="embedded-runtime-draft">
  {#if runnable.length === 0}
    <EmptyState icon="sparkle" title="No agent can run here yet"
      body="This needs an agent that runs a Threads conversation (Claude Code, Codex, Gemini, Grok, opencode, pi, Muse or a custom provider built on one). Install one, or set it up in Settings › Providers."
      actionLabel="Open Providers" actionKind="secondary" onaction={() => router.go('settings/providers')} />
  {:else}
    <!-- Laid out like a conversation: the lead scrolls at the top, the
         suggestions sit in one row (scrolling sideways, never cut) right
         above the composer, which stays at the bottom — so the agent and
         access pickers open upward from it, over the lead. -->
    <div class="ed-top">
      {#if draft.lead}
        <div class="ed-lead">{@render draft.lead()}</div>
      {/if}
    </div>
    {#if draft.suggestions?.length}
      <div class="ed-suggest" role="group" aria-label="Suggestions">
        {#each draft.suggestions as s (s.label)}
          <button type="button" class="btn small ed-chip" disabled={busy} onclick={() => void suggest(s.prompt)}>{s.label}</button>
        {/each}
      </div>
    {/if}
    <RuntimeStartComposer
      bind:this={composer}
      compact
      runtimeOnly
      allowCouncil={false}
      cwd={draft.cwd ?? ''}
      {draftKey}
      rememberScope="embed:{draft.source ?? 'thread'}"
      accessMode={access}
      onaccess={(m) => void pickAccess(m)}
      placeholder={draft.placeholder}
      inputLabel={draft.placeholder ?? 'First message'}
      onlaunch={launch}
    >
      {#snippet below()}
        <p class="ed-hint" data-testid="embedded-draft-access-hint">{ACCESS_HINT[access]}</p>
      {/snippet}
    </RuntimeStartComposer>
  {/if}
</div>

<style>
  .ed {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px 10px;
  }
  .ed-top {
    flex: 1 1 auto;
    min-block-size: 0;
    overflow-y: auto;
  }
  .ed > :global(.nt-composer) {
    flex: none;
  }
  .ed-lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .ed-suggest {
    flex: none;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    overscroll-behavior-x: contain;
    scrollbar-width: thin;
  }
  .ed-chip {
    flex: none;
    white-space: nowrap;
  }
  .ed-hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 1024px) {
    .ed-chip {
      min-block-size: var(--hit-min);
    }
  }
</style>
