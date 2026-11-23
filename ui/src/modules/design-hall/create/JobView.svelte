<script lang="ts">
  // One thing being made: the real job's progress read as plain steps
  // ("Building the shape", "Adding a skeleton"…), a first look as soon as
  // there is one, an honest time left and a clear Stop. The job runs in the
  // daemon, so leaving and coming back picks it up where it is.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { announce } from '../../../lib/announce.svelte';
  import { pollWhileVisible } from '../../../lib/poll';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { createStore } from './createStore.svelte';
  import CreateModel from './CreateModel.svelte';
  import { STEP_LABEL, createHref, currentStep, jobSteps, secondsLeft, timeText } from './plan';

  interface Props {
    id: string;
  }
  let { id }: Props = $props();

  let missing = $state(false);
  let stopping = $state(false);
  let now = $state(Date.now());
  const job = $derived(createStore.jobs[id]);
  let meta = $state<ReturnType<typeof createStore.meta>>(null);
  const steps = $derived(job ? jobSteps(job) : []);
  const step = $derived(job ? currentStep(job, steps) : null);
  const at = $derived(step ? steps.indexOf(step) : 0);
  const elapsed = $derived(job ? Math.max(0, (now - Date.parse(job.created_at)) / 1000) : 0);
  const left = $derived(step ? secondsLeft(steps, step, elapsed) : 0);
  const total = $derived(steps.length ? secondsLeft(steps, steps[0], 0) : 0);
  const slow = $derived(elapsed > total * 1.5);
  const what = $derived(job?.then_stages?.includes('rig') ? 'person' : 'thing');
  const again = $derived<'character' | 'object'>(job?.then_stages?.includes('rig') ? 'character' : 'object');

  $effect(() => {
    const jid = id;
    untrack(() => {
      meta = createStore.meta(jid);
      void createStore.refreshJob(jid).catch(() => (missing = true));
    });
    const clock = pollWhileVisible(() => void (now = Date.now()), { ms: 5000, immediate: false });
    return () => clock.stop();
  });

  // Done: straight to the result (or to before/after for a change).
  $effect(() => {
    if (job?.status !== 'succeeded' || !job.artifact_id) return;
    const aid = job.artifact_id;
    untrack(() => {
      if (meta?.before) router.replace(createHref('a', aid, 'compare', String(meta.before)));
      else router.replace(createHref('a', aid));
    });
  });

  // Each new step is said once for screen readers.
  $effect(() => {
    if (step && job?.status === 'running') announce(STEP_LABEL[step]);
  });

  async function stop(): Promise<void> {
    stopping = true;
    try {
      await createStore.stop(id);
    } catch {
      toasts.warn('I couldn’t stop it', 'Please try again.');
    } finally {
      stopping = false;
    }
  }
</script>

{#if missing && !job}
  <EmptyState variant="page" icon="search" title="I can’t find this one" body="It may have been removed." actionLabel="Go to my creations" onaction={() => router.go(createHref('mine'))} />
{:else if !job}
  <div class="jv-calm"><span class="spinner" role="img" aria-label="Loading"></span></div>
{:else if job.status === 'failed'}
  <div data-testid="create-job-failed">
    {#if job.artifact_id}
      <EmptyState variant="page" tone="error" icon="warning" title="Almost!" body={`Your ${what} is made, but ${STEP_LABEL[step ?? 'ready'].toLowerCase()} didn’t work.`} actionLabel="Look at it" onaction={() => router.go(createHref('a', job.artifact_id!))} />
    {:else}
      <EmptyState variant="page" tone="error" icon="warning" title="That didn’t work" body="Something went wrong while making it. Making it again often helps, or try different words." actionLabel="Make it again" onaction={() => router.go(createHref('new', again))} />
    {/if}
  </div>
{:else if job.status === 'cancelled'}
  <div data-testid="create-job-stopped">
    <EmptyState variant="page" icon="stop" title="Stopped" body={`Nothing was lost.${job.artifact_id ? ' What was finished is kept.' : ''}`} actionLabel={job.artifact_id ? 'Look at it' : 'Start again'} onaction={() => router.go(job.artifact_id ? createHref('a', job.artifact_id) : createHref('new', again))} />
  </div>
{:else}
  <h1 class="cr-h1">Making your {what}…</h1>
  <p class="cr-sub">
    {#if job.status === 'queued'}
      Waiting for its turn — something else is being made first.
    {:else if slow}
      Taking a bit longer than usual. Still working.
    {:else}
      {timeText(left)} left.
    {/if}
    You can leave — it keeps going and will be here when you come back.
  </p>
  <div class="cr-two">
    <div class="cr-stage jv-stage" class:empty={!job.artifact_id}>
      {#if job.artifact_id}
        {#key job.version_id}<CreateModel artifactId={job.artifact_id} label="First look" />{/key}
        <span class="jv-tag">First look — still being finished</span>
      {:else}
        <div class="jv-wait"><Icon name="box" size={24} /><span>A first look will show up here</span></div>
      {/if}
    </div>
    <div class="cr-card">
      <ol class="jv-steps" data-testid="create-steps">
        {#each steps as s, k (s)}
          <li class:done={k < at} class:now={k === at} aria-current={k === at ? 'step' : undefined}>
            <span class="jv-dot" aria-hidden="true">{#if k < at}<Icon name="check" size={14} />{:else}{k + 1}{/if}</span>
            {STEP_LABEL[s]}
          </li>
        {/each}
      </ol>
      <div class="jv-bar" role="progressbar" aria-label="How far along" aria-valuemin={0} aria-valuemax={100} aria-valuenow={job.progress_pct}>
        <i style:inline-size={`${Math.max(3, job.progress_pct)}%`}></i>
      </div>
      <button type="button" class="cr-secondary jv-stop" disabled={stopping} onclick={() => void stop()} data-testid="create-stop">
        <Icon name="stop" size={16} /> {stopping ? 'Stopping…' : 'Stop'}
      </button>
    </div>
  </div>
{/if}

<style>
  .jv-calm {
    display: grid;
    justify-items: center;
    gap: var(--sp-2);
    text-align: center;
    padding-block: var(--sp-7);
    margin-block-start: var(--sp-5);
  }
  .jv-stage {
    position: relative;
  }
  .jv-tag {
    position: absolute;
    inset-block-start: var(--sp-2);
    inset-inline-start: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
    box-shadow: var(--shadow-xs);
  }
  @media (max-width: 1024px) {
    /* Nothing to look at yet: the steps come first, the waiting box stays small. */
    .jv-stage.empty {
      order: 2;
      aspect-ratio: 3 / 1;
    }
  }
  .jv-wait {
    display: grid;
    place-items: center;
    align-content: center;
    gap: var(--sp-2);
    block-size: 100%;
    border-radius: var(--radius-l);
    border: 2px dashed var(--border);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .jv-steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--sp-2);
  }
  .jv-steps li {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-block-size: 52px;
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-l);
    color: var(--text-dim);
  }
  .jv-steps li.done {
    color: var(--text);
  }
  .jv-steps li.now {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 600;
  }
  .jv-dot {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: 30px;
    block-size: 30px;
    border-radius: 50%;
    background: var(--surface-3);
    font-size: var(--fs-m);
  }
  .done .jv-dot {
    background: var(--success);
    color: var(--accent-contrast);
  }
  .now .jv-dot {
    background: var(--accent-solid);
    color: var(--accent-contrast);
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .jv-bar {
    block-size: 12px;
    margin-block: var(--sp-4) var(--sp-2);
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .jv-bar i {
    display: block;
    block-size: 100%;
    background: var(--accent-solid);
    border-radius: 999px;
  }
  .jv-stop {
    inline-size: 100%;
    margin-block-start: var(--sp-3);
  }
  @media (prefers-reduced-motion: reduce) {
    .now .jv-dot {
      animation: none;
    }
  }
</style>
