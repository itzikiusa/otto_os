<script lang="ts">
  // Settings › Threads (mounted by settings/Settings.svelte; it lives with
  // the Threads stores it edits): the per-device Threads prefs in one calm place. Each
  // lives in its own store (threadPrefs, the thread store's sidebar grouping,
  // threadsChrome's details card); the toolbar ⋯ › View offers the same
  // switches as shortcuts.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { threadPrefs, type SubagentOpenIn } from './runtime/threadPrefs.svelte';
  import type { MidTurn } from './runtime/composerSend';
  import type { ToolVisibility } from './runtime/workLog';
  import { threads } from './threadsStore.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { sectionLabel } from '../settings/sections'; // ui-guards: allow — the Settings registry names this page

  const MID: { id: MidTurn; label: string; hint: string }[] = [
    { id: 'queue', label: 'Queue', hint: 'A message sent while the agent works waits until its turn ends. ⌘↵ steers instead.' },
    { id: 'steer', label: 'Steer', hint: 'A message sent while the agent works goes into the running turn; the agent reads it at its next step. ⌘↵ queues instead. Agents that can’t take a message mid-turn still queue.' },
  ];
  const TOOLS: { id: ToolVisibility; label: string; hint: string }[] = [
    { id: 'all', label: 'Show all', hint: 'Every command, edit and search the agent ran.' },
    { id: 'hide-ok', label: 'Hide successful', hint: 'Only failed or gated calls. Replies, questions and approvals always show.' },
    { id: 'none', label: 'Hide all', hint: 'No tool calls except failures and approvals. Replies and questions always show.' },
  ];
  const SUB: { id: SubagentOpenIn; label: string }[] = [
    { id: 'panel', label: 'Side panel' },
    { id: 'main', label: 'Main view' },
  ];

  /** Arrow keys move between a radio group's options (WAI-ARIA radio group). */
  function radioKey<T>(e: KeyboardEvent, list: { id: T }[], cur: T, pick: (v: T) => void): void {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const i = (list.findIndex((o) => o.id === cur) + d + list.length) % list.length;
    pick(list[i].id);
    ((e.currentTarget as HTMLElement).parentElement?.children[i] as HTMLElement | undefined)?.focus();
  }
</script>

<div class="settings-section" data-testid="settings-threads">
  <PageHeader title={sectionLabel('threads')} subtitle="How Threads behaves on this device" />
  <PageBody width="readable">
    <p class="intro">Saved on this device. The toolbar ⋯ › View in Threads has the same switches.</p>

    <div class="section-title" id="th-mid">While the agent works, Enter</div>
    <div class="segmented" role="radiogroup" aria-labelledby="th-mid">
      {#each MID as o (o.id)}
        <button
          type="button"
          role="radio"
          aria-checked={threadPrefs.midTurn === o.id}
          tabindex={threadPrefs.midTurn === o.id ? 0 : -1}
          onkeydown={(e) => radioKey(e, MID, threadPrefs.midTurn, (v) => threadPrefs.setMidTurn(v))}
          onclick={() => threadPrefs.setMidTurn(o.id)}
        >{o.label}</button>
      {/each}
    </div>
    <p class="hint-line">{MID.find((o) => o.id === threadPrefs.midTurn)?.hint}</p>

    <div class="section-title" id="th-tools">Tool calls</div>
    <div class="segmented" role="radiogroup" aria-labelledby="th-tools">
      {#each TOOLS as o (o.id)}
        <button
          type="button"
          role="radio"
          aria-checked={threadPrefs.tools === o.id}
          tabindex={threadPrefs.tools === o.id ? 0 : -1}
          onkeydown={(e) => radioKey(e, TOOLS, threadPrefs.tools, (v) => threadPrefs.setTools(v))}
          onclick={() => threadPrefs.setTools(o.id)}
        >{o.label}</button>
      {/each}
    </div>
    <p class="hint-line">{TOOLS.find((o) => o.id === threadPrefs.tools)?.hint}</p>

    <div class="section-title" id="th-sub">Open a subagent in</div>
    <div class="segmented" role="radiogroup" aria-labelledby="th-sub">
      {#each SUB as o (o.id)}
        <button
          type="button"
          role="radio"
          aria-checked={threadPrefs.subagentIn === o.id}
          tabindex={threadPrefs.subagentIn === o.id ? 0 : -1}
          onkeydown={(e) => radioKey(e, SUB, threadPrefs.subagentIn, (v) => threadPrefs.setSubagentIn(v))}
          onclick={() => threadPrefs.setSubagentIn(o.id)}
        >{o.label}</button>
      {/each}
    </div>
    <p class="hint-line">Where a subagent’s thread opens when you pick it from its parent. A phone always opens it full screen.</p>

    <div class="section-title">Layout</div>
    <div class="toggle-block">
      {#snippet toggle(id: string, label: string, hint: string, on: boolean, set: (v: boolean) => void)}
        <label class="tg">
          <input type="checkbox" checked={on} onchange={(e) => set(e.currentTarget.checked)} data-testid={id} aria-describedby="{id}-hint" />
          <span class="tg-text"><span class="tg-label">{label}</span><span class="tg-hint" id="{id}-hint">{hint}</span></span>
        </label>
      {/snippet}
      {@render toggle('threads-pref-group', 'Group the sidebar by project', 'Folders per project instead of one inbox of every thread.', threads.groupByProject, (v) => threads.setGroupByProject(v))}
      {@render toggle('threads-pref-card', 'Show the thread details card', 'The card beside the conversation with the thread’s folder, branch, changes and agents, where there is room. It always heads the right panel.', threadsChrome.cardVisible, (v) => { if (v !== threadsChrome.cardVisible) threadsChrome.toggleCard(); })}
    </div>
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .intro {
    margin-block: 0 var(--sp-4);
    max-inline-size: var(--settings-col);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .hint-line {
    margin-block: 6px 0;
    max-inline-size: var(--settings-col);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .toggle-block {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-inline-size: var(--settings-col);
  }
  .tg {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
    cursor: pointer;
  }
  .tg input {
    margin-block-start: 2px;
  }
  .tg-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tg-label {
    font-size: var(--fs-m);
    color: var(--text);
  }
  .tg-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .tg {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
  .segmented > button:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  @media (pointer: coarse) {
    .segmented > button {
      min-block-size: calc(var(--hit-min) + 8px);
    }
  }
</style>
