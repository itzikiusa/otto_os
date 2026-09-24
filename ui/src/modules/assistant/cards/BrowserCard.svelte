<script lang="ts">
  // A browser chore the assistant is doing in its own browser profile: a live
  // thumbnail, the step list (done / now / next), and the hand-off controls —
  // Watch (opens the live tab), Take over (pauses the agent so you can type a
  // password, 2FA or CAPTCHA) and Hand back.
  import ActionCard from './ActionCard.svelte';
  import StatePill from './StatePill.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
  import { browser } from '../../../lib/stores/browser.svelte';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { taskStateLabel, taskTone, type BrowserProgress } from '../model';
  import type { AssistantTask } from '../../../lib/api/types';

  interface Props {
    task: AssistantTask;
    progress: BrowserProgress;
  }
  let { task, progress }: Props = $props();

  // `takeover` moves the task to needs_you with a `takeover` item until handback.
  const youHaveIt = $derived(task.state === 'needs_you' && (task.kind === 'takeover' || task.needs_you?.kind === 'takeover'));
  const finished = $derived(task.state === 'done' || task.state === 'failed' || task.state === 'cancelled');

  // Thumbnail: a daemon path is fetched with the bearer token; absolute URLs
  // and data: URIs are used as they are. No thumbnail → a quiet wireframe.
  let thumb = $state<string | null>(null);
  let thumbFailed = $state(false);
  $effect(() => {
    const src = progress.thumbnail_url;
    thumbFailed = false;
    if (!src) {
      thumb = null;
      return;
    }
    if (!src.startsWith('/')) {
      thumb = src;
      return;
    }
    let url: string | null = null;
    let live = true;
    authedBlobUrl(src)
      .then((u) => {
        if (live) thumb = url = u;
        else URL.revokeObjectURL(u);
      })
      .catch(() => {
        if (live) thumbFailed = true;
      });
    return () => {
      live = false;
      if (url) URL.revokeObjectURL(url);
    };
  });

  let busy = $state(false);
  async function run(action: 'takeover' | 'handback'): Promise<void> {
    busy = true;
    try {
      await assistant.act(task.id, action);
    } catch (e) {
      toasts.error(action === 'takeover' ? 'Couldn’t take over the browser' : 'Couldn’t hand the browser back', describeError(e));
    } finally {
      busy = false;
    }
  }
  function watch(): void {
    if (!progress.tab_id) return;
    browser.select(progress.tab_id);
    router.go('browser');
  }
</script>

<ActionCard icon="compass" kind="Browser" summary={task.title} attention={youHaveIt} testid="card-browser">
  {#snippet pill()}
    {#if youHaveIt}
      <StatePill tone="warn" label="You have control" />
    {:else}
      <StatePill tone={taskTone(task)} label={taskStateLabel(task)} live={task.state === 'running'} />
    {/if}
  {/snippet}
  <div class="wrap"><div class="grid">
    <figure class="shot" aria-label="Live browser thumbnail">
      {#if progress.url}<figcaption class="url mono" dir="ltr" title={progress.url}>{progress.url}</figcaption>{/if}
      {#if thumb && !thumbFailed}
        <img src={thumb} alt={`Browser: ${task.title}`} />
      {:else}
        <div class="wire" aria-hidden="true">
          <span class="b" style="inset-block-start:10px;inset-inline-start:8px;width:40%;height:8px"></span>
          <span class="b" style="inset-block-start:26px;inset-inline-start:8px;width:28%;height:40px"></span>
          <span class="b" style="inset-block-start:26px;inset-inline-start:36%;width:56%;height:8px"></span>
          <span class="b" style="inset-block-start:40px;inset-inline-start:36%;width:40%;height:8px"></span>
        </div>
      {/if}
    </figure>
    <div class="side">
      <ol class="steps">
        {#each progress.steps as s, i (i)}
          <li class={s.state}>
            <span class="mark" aria-hidden="true">
              {#if s.state === 'done'}<Icon name="check" size={12} />{:else if s.state === 'current'}<span class="now"></span>{:else}<span class="todo"></span>{/if}
            </span>
            <span class="sr-only">{s.state === 'done' ? 'Done:' : s.state === 'current' ? 'Now:' : 'Next:'}</span>
            <span>{s.label}</span>
          </li>
        {/each}
      </ol>
      {#if youHaveIt}
        <p class="note warn">You have the browser. Otto is paused until you hand it back.</p>
      {:else if progress.note}
        <p class="note">{progress.note}</p>
      {/if}
    </div>
  </div></div>
  {#snippet footer()}
    <button class="btn small" onclick={watch} disabled={!progress.tab_id} title={progress.tab_id ? 'Open the live tab in Browser' : 'The live tab isn’t available yet'}>
      <Icon name="eye" size={12} /> Watch live
    </button>
    {#if youHaveIt}
      <button class="btn small primary" onclick={() => void run('handback')} disabled={busy}>{busy ? 'Handing back…' : 'Hand back'}</button>
    {:else}
      <button class="btn small" onclick={() => void run('takeover')} disabled={busy || finished} title={finished ? 'This task has finished' : 'Pause Otto and use the browser yourself'}>
        <Icon name="cursor" size={12} /> {busy ? 'Taking over…' : 'Take over'}
      </button>
    {/if}
  {/snippet}
</ActionCard>

<style>
  .wrap {
    container-type: inline-size;
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 240px) minmax(0, 1fr);
    gap: 12px;
  }
  .shot {
    margin: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface-2);
    aspect-ratio: 16 / 10;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .url {
    flex-shrink: 0;
    padding: 2px 8px;
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--surface-3);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .shot img {
    flex: 1;
    min-height: 0;
    width: 100%;
    object-fit: cover;
    object-position: top;
  }
  .wire {
    position: relative;
    flex: 1;
  }
  .wire .b {
    position: absolute;
    border-radius: 3px;
    background: var(--surface-3);
  }
  .side {
    min-width: 0;
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .steps li {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    line-height: 1.4;
  }
  .steps li.todo {
    color: var(--text-dim);
  }
  .steps li.current {
    font-weight: 500;
  }
  .mark {
    flex-shrink: 0;
    width: 12px;
    height: 17px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--success);
  }
  .now {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--info);
    animation: step-pulse 1.4s ease-in-out infinite;
  }
  .todo {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1px solid var(--text-dim);
  }
  @keyframes step-pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .now {
      animation: none;
    }
  }
  .note {
    margin: 8px 0 0;
    color: var(--text-dim);
  }
  .note.warn {
    color: var(--warning);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  @container (max-width: 460px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
