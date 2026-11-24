<script lang="ts">
  // One Agents inbox row on the shared InboxRow (patterns.md §10): the
  // workspace's badge and name … the status WORD (lib/statusWords.ts); the
  // title; branch or folder · provider · terminal / chat. Select mode puts a
  // checkbox before it; the Manual sort makes it a drag source. Everything a
  // row can do is in its menu (right-click / long-press), the same menu the
  // tab and pane carry.
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { Session } from '../../lib/api/types';
  import type { StatusWord } from '../../lib/statusWords';

  interface Props {
    session: Session;
    project: string;
    status: StatusWord;
    strong: boolean;
    /** `chat` — a structured thread; `archived` — in History. */
    kind: 'terminal' | 'chat' | 'archived';
    branch?: string | null;
    selected: boolean;
    inPane: boolean;
    tip: string;
    stateKey: string;
    select?: { on: boolean; checked: boolean; ontoggle: () => void } | null;
    drag?: { over: boolean; onstart: (e: DragEvent) => void; onover: (e: DragEvent) => void; ondrop: (e: DragEvent) => void; onend: () => void; onleave: () => void } | null;
    onclick: (e: MouseEvent) => void;
    onmenu: (e: MouseEvent) => void;
    onrename?: () => void;
    /** The row's × (hover / focus): close the session as its tab's × does. */
    close?: { title: string; onclose: () => void } | null;
  }

  let { session: s, project, status, strong, kind, branch = null, selected, inPane, tip, stateKey, select = null, drag = null, onclick, onmenu, onrename, close = null }: Props = $props();
  const provider = $derived(s.provider ? s.provider.charAt(0).toUpperCase() + s.provider.slice(1) : '');
  const where = $derived(branch ?? (s.cwd ? s.cwd.replace(/\/+$/, '').split('/').pop() || '~' : '~'));
</script>

<!-- A drag source/target for the Manual order (presentation); the row's
     button and menu carry every action for the keyboard. -->
<div
  role="presentation"
  class="ai-row"
  data-workspace={s.workspace_id}
  class:drag-over={drag?.over}
  draggable={drag ? 'true' : undefined}
  ondragstart={drag?.onstart}
  ondragover={drag?.onover}
  ondragleave={drag?.onleave}
  ondrop={drag?.ondrop}
  ondragend={drag?.onend}
  ondblclick={onrename}
>
  {#if select?.on}
    <input type="checkbox" class="ai-check" checked={select.checked} onchange={select.ontoggle} aria-label="Select {s.title}" />
  {/if}
  <InboxRow
    title={s.title}
    {project}
    word={status.word}
    tone={status.tone}
    time={rel(s.last_active_at)}
    {tip}
    {strong}
    {selected}
    {inPane}
    testid="agents-row-{s.id}"
    statusTestid="agents-status-{s.id}"
    state={stateKey}
    {onclick}
    oncontextmenu={onmenu}
  >
    {#snippet actions()}
      {#if close}
        <button type="button" class="icon-btn" aria-label="Close session" title={close.title} onclick={close.onclose}>
          <Icon name="x" size={12} />
        </button>
      {/if}
    {/snippet}
    {#snippet line3()}
      <Icon name={branch ? 'branch' : 'folder'} size={12} />
      <span class="ellipsis" dir="auto">{where}</span>
      <span class="ai-dim" aria-hidden="true">·</span>
      {#if hasProviderIcon(s.provider)}<ProviderIcon provider={s.provider} size={12} />{/if}
      <span class="ai-prov">{provider}</span>
      <span class="ai-dim" aria-hidden="true">·</span>
      <span class="ai-kind">{kind === 'chat' ? 'chat' : kind === 'archived' ? 'closed' : 'terminal'}</span>
    {/snippet}
  </InboxRow>
</div>

<style>
  .ai-row {
    display: flex;
    align-items: center;
    min-width: 0;
    border-radius: var(--radius-m);
  }
  .ai-row > :global(.tr-row) {
    flex: 1;
  }
  .ai-row.drag-over {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .ai-row[draggable='true'] :global(.tr-main) {
    cursor: grab;
  }
  .ai-check {
    flex: none;
    margin-inline: var(--sp-3) 0;
  }
  .ai-dim {
    flex: none;
  }
  .ai-prov,
  .ai-kind {
    flex: none;
    white-space: nowrap;
  }
  @media (hover: none), (pointer: coarse) {
    .ai-check {
      inline-size: 22px;
      block-size: 22px;
    }
  }
</style>
