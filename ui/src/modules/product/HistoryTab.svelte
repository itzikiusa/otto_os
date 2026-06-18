<script lang="ts">
  // History tab — sectioned event timeline with section filter for the selected story.
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { ProductEvent } from './types';

  type SectionFilter =
    | 'all'
    | 'source'
    | 'analysis'
    | 'questions'
    | 'notes'
    | 'rewrite'
    | 'tests'
    | 'publish'
    | 'inject'
    | 'watch';

  const SECTIONS: { id: SectionFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'source', label: 'Source' },
    { id: 'analysis', label: 'Analysis' },
    { id: 'questions', label: 'Questions' },
    { id: 'notes', label: 'Notes' },
    { id: 'rewrite', label: 'Rewrite' },
    { id: 'tests', label: 'Tests' },
    { id: 'publish', label: 'Publish' },
    { id: 'inject', label: 'Inject' },
    { id: 'watch', label: 'Watch' },
  ];

  let selectedSection = $state<SectionFilter>('all');
  let loaded = $state(false);

  const story = $derived(product.detail?.story ?? null);

  // Reset and reload when story changes.
  $effect(() => {
    product.selectedId;
    loaded = false;
    void doLoad('all');
    selectedSection = 'all';
  });

  async function doLoad(section: SectionFilter): Promise<void> {
    try {
      await product.loadEvents(section === 'all' ? undefined : section);
      loaded = true;
    } catch (e) {
      toasts.error('Could not load events', product.errMsg(e));
    }
  }

  async function onSectionChange(e: Event): Promise<void> {
    const val = (e.target as HTMLSelectElement).value as SectionFilter;
    selectedSection = val;
    await doLoad(val);
  }

  // Newest first (already from API but sort defensively)
  const events = $derived(
    [...product.events].sort(
      (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
    ),
  );

  function sectionColor(section: string): string {
    switch (section) {
      case 'source': return 'sec-source';
      case 'analysis': return 'sec-analysis';
      case 'questions': return 'sec-questions';
      case 'notes': return 'sec-notes';
      case 'rewrite': return 'sec-rewrite';
      case 'tests': return 'sec-tests';
      case 'publish': return 'sec-publish';
      case 'inject': return 'sec-inject';
      case 'watch': return 'sec-watch';
      default: return 'sec-other';
    }
  }

  function sectionIcon(section: string): string {
    switch (section) {
      case 'source': return 'file';
      case 'analysis': return 'gauge';
      case 'questions': return 'comment';
      case 'notes': return 'note';
      case 'rewrite': return 'edit';
      case 'tests': return 'check';
      case 'publish': return 'send';
      case 'inject': return 'zap';
      case 'watch': return 'eye';
      default: return 'dot';
    }
  }

  function relativeTime(iso: string): string {
    const now = Date.now();
    const then = new Date(iso).getTime();
    const diffMs = now - then;
    const diffMins = Math.floor(diffMs / 60000);
    if (diffMins < 1) return 'just now';
    if (diffMins < 60) return `${diffMins}m ago`;
    const diffHours = Math.floor(diffMins / 60);
    if (diffHours < 24) return `${diffHours}h ago`;
    const diffDays = Math.floor(diffHours / 24);
    if (diffDays < 30) return `${diffDays}d ago`;
    return new Date(iso).toLocaleDateString();
  }

  function absoluteTime(iso: string): string {
    try {
      return new Date(iso).toLocaleString();
    } catch {
      return iso;
    }
  }
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="history-tab">
    <!-- Toolbar -->
    <div class="toolbar">
      <Icon name="clock" size={13} />
      <span class="label">Section</span>
      <select class="sec-select" value={selectedSection} onchange={onSectionChange} disabled={product.loadingEvents}>
        {#each SECTIONS as s (s.id)}
          <option value={s.id}>{s.label}</option>
        {/each}
      </select>
      {#if product.loadingEvents}
        <span class="dim">Loading…</span>
      {:else}
        <span class="dim">{events.length} event{events.length !== 1 ? 's' : ''}</span>
      {/if}
    </div>

    <!-- Timeline -->
    {#if product.loadingEvents && !loaded}
      <div class="muted">Loading events…</div>
    {:else if events.length === 0}
      <div class="muted">No events yet{selectedSection !== 'all' ? ` in "${selectedSection}"` : ''}.</div>
    {:else}
      <div class="timeline">
        {#each events as ev (ev.id)}
          <div class="event-row">
            <!-- Connector line -->
            <div class="ev-line-col">
              <div class="ev-dot {sectionColor(ev.section)}"></div>
              <div class="ev-connector"></div>
            </div>

            <!-- Content -->
            <div class="ev-content">
              <div class="ev-header">
                <span class="sec-chip {sectionColor(ev.section)}">
                  <Icon name={sectionIcon(ev.section)} size={10} />
                  {ev.section}
                </span>
                <span class="ev-kind">{ev.kind}</span>
                {#if ev.actor_id}
                  <span class="ev-actor dim">· {ev.actor_id}</span>
                {/if}
                <span class="spacer"></span>
                <span class="ev-time" title={absoluteTime(ev.created_at)}>
                  {relativeTime(ev.created_at)}
                </span>
              </div>
              <p class="ev-summary">{ev.summary}</p>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }
  .history-tab {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 780px;
    width: 100%;
  }

  /* Toolbar */
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-raised, var(--surface));
  }
  .label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .sec-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 3px 7px;
  }
  .dim {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .spacer {
    flex: 1;
  }

  /* Timeline */
  .timeline {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .event-row {
    display: flex;
    gap: 12px;
    min-width: 0;
  }
  .ev-line-col {
    display: flex;
    flex-direction: column;
    align-items: center;
    flex-shrink: 0;
    padding-top: 4px;
    width: 16px;
  }
  .ev-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
    border: 2px solid var(--surface);
  }
  .ev-connector {
    flex: 1;
    width: 1px;
    background: var(--border);
    margin: 3px 0;
    min-height: 18px;
  }
  .event-row:last-child .ev-connector {
    display: none;
  }
  .ev-content {
    flex: 1;
    min-width: 0;
    padding-bottom: 14px;
  }
  .ev-header {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    margin-bottom: 3px;
  }
  .sec-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: 999px;
  }
  .ev-kind {
    font-size: 12px;
    font-weight: 500;
    color: var(--text);
  }
  .ev-actor {
    font-size: 11px;
  }
  .ev-time {
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    cursor: default;
  }
  .ev-summary {
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text-dim);
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Section color theme — dot + chip */
  .sec-source   { background: color-mix(in srgb, #60a5fa 18%, transparent); color: #3b82f6; }
  .sec-analysis { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .sec-questions { background: color-mix(in srgb, #a78bfa 18%, transparent); color: #7c3aed; }
  .sec-notes    { background: color-mix(in srgb, #34d399 18%, transparent); color: #059669; }
  .sec-rewrite  { background: color-mix(in srgb, #f59e0b 18%, transparent); color: #b45309; }
  .sec-tests    { background: color-mix(in srgb, #10b981 18%, transparent); color: #047857; }
  .sec-publish  { background: color-mix(in srgb, #f97316 18%, transparent); color: #c2410c; }
  .sec-inject   { background: color-mix(in srgb, #e879f9 18%, transparent); color: #a21caf; }
  .sec-watch    { background: color-mix(in srgb, #64748b 18%, transparent); color: #475569; }
  .sec-other    { background: color-mix(in srgb, var(--text-dim) 15%, transparent); color: var(--text-dim); }
</style>
