<script lang="ts">
  // The right rail on wide windows: what needs you, what is running, and how
  // this week's load splits across your subscriptions (with any limit hit).
  // Each block owns its own loading / empty / error state so one failing
  // source never blanks the rest.
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { assistant } from '../../lib/stores/assistant.svelte';
  import { formatCount } from '../../lib/metric-format';
  import { TASK_KIND, loadShare, providerName } from './model';
  import { whenLabel } from './format';

  interface Props {
    onopenthread: (id: string) => void;
    onopentasks: () => void;
  }
  let { onopenthread, onopentasks }: Props = $props();

  $effect(() => {
    if (assistant.limits.state === 'idle') void assistant.loadLimits();
    if (assistant.tasks.state === 'idle') void assistant.loadTasks();
    if (assistant.usageWeek.state === 'idle') void assistant.loadUsageWeek();
  });

  const needs = $derived(assistant.needs.items);
  const running = $derived(assistant.tasks.data.filter((t) => t.state === 'running'));
  const share = $derived(loadShare(assistant.usageWeek.data.filter((p) => p.provider === 'claude' || p.provider === 'codex')));
  const limited = $derived(assistant.limits.data.filter((l) => l.limited));
  const threadTitle = (id: string | null): string => (id ? (assistant.thread(id)?.title ?? 'A thread') : 'Not in a thread');
</script>

<aside class="rail" aria-label="Needs you, running and subscriptions" data-testid="assistant-rail">
  <section>
    <h2 class="section-title">Needs you</h2>
    {#if assistant.needsState === 'loading' && !needs.length}
      <p class="dim">Loading…</p>
    {:else if assistant.needsState === 'error'}
      <p class="dim">Couldn’t load. <button class="link" onclick={() => void assistant.loadNeedsYou()}>Retry</button></p>
    {:else if !needs.length}
      <p class="dim">Nothing is waiting on you.</p>
    {:else}
      <ul>
        {#each needs as n (n.id)}
          <li>
            <button class="item" title={n.title} onclick={() => (n.thread_id ? onopenthread(n.thread_id) : onopentasks())}>
              <StatusDot status="idle" needsYou size={8} />
              <span class="t"><span class="name">{n.title}</span><span class="sub">{threadTitle(n.thread_id)}</span></span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section>
    <h2 class="section-title">Running</h2>
    {#if assistant.tasks.state === 'loading' && !assistant.tasks.data.length}
      <p class="dim">Loading…</p>
    {:else if assistant.tasks.state === 'error'}
      <p class="dim">Couldn’t load. <button class="link" onclick={() => void assistant.loadTasks()}>Retry</button></p>
    {:else if !running.length}
      <p class="dim">Nothing running.</p>
    {:else}
      <ul>
        {#each running as t (t.id)}
          <li>
            <button class="item" title={t.title} onclick={onopentasks}>
              <StatusDot status="working" size={8} />
              <span class="t"><span class="name">{t.title}</span><span class="sub">{TASK_KIND[t.kind] ?? t.kind} · {threadTitle(t.thread_id)}</span></span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section>
    <h2 class="section-title">Subscriptions this week</h2>
    {#each limited as l (l.provider + (l.account_id ?? ''))}
      <p class="limit" role="status">
        <ProviderIcon provider={l.provider} size={12} />
        <span>{providerName(l.provider)} limit reached{l.until ? ` · back ${whenLabel(l.until)}` : ''}</span>
      </p>
    {/each}
    {#if assistant.usageWeek.state === 'loading' && !share.length}
      <p class="dim">Loading…</p>
    {:else if share.length}
      <div class="share" role="img" aria-label={share.map((s) => `${providerName(s.provider)} ${s.pct}%`).join(', ')}>
        {#each share as s (s.provider)}
          <span class="seg {s.provider}" style:width="{s.pct}%"></span>
        {/each}
      </div>
      <ul class="legend">
        {#each share as s (s.provider)}
          <li>
            <ProviderIcon provider={s.provider} size={12} />
            <span class="grow">{providerName(s.provider)}</span>
            <span class="num">{s.pct}%</span>
            <span class="dim num">{formatCount(s.tokens)} tokens</span>
          </li>
        {/each}
      </ul>
    {:else if !limited.length}
      <p class="dim">
        {assistant.usageWeek.state === 'unsupported' ? 'Usage tracking is off or needs admin access. Limits still show here when a provider hits one.' : 'No usage recorded this week.'}
      </p>
    {/if}
  </section>
</aside>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    gap: 18px;
    padding: 12px 12px 20px;
    overflow-y: auto;
    min-height: 0;
    background: var(--surface-2);
    border-inline-start: 1px solid var(--border);
  }
  .section-title {
    margin: 0 0 6px;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .item {
    width: 100%;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 5px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .item :global(.dot) {
    margin-top: 5px;
  }
  .item:hover {
    background: var(--hover);
  }
  .t {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
  }
  .limit {
    margin: 0 0 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--warning);
  }
  .share {
    display: flex;
    height: 6px;
    border-radius: 999px;
    overflow: hidden;
    background: var(--surface-3);
    gap: 2px;
  }
  .seg {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .seg.codex {
    background: var(--text-dim);
  }
  .legend {
    margin-top: 8px;
    gap: 4px;
  }
  .legend li {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .legend .dim {
    font-size: var(--fs-xs);
  }
</style>
