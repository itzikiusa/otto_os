<script lang="ts">
  // New-note dialog: name + folder + (OKF vaults) a concept-template picker.
  import { vault } from './vault.svelte';

  let {
    open = $bindable(false),
    dir = $bindable(''),
  }: { open?: boolean; dir?: string } = $props();

  let name = $state('');
  let template = $state('blank');
  let input = $state<HTMLInputElement | undefined>();

  $effect(() => {
    if (open) {
      name = '';
      template = vault.current?.okf ? 'concept' : 'blank';
      requestAnimationFrame(() => input?.focus());
    }
  });

  const OKF_TYPES = ['Service', 'Reference', 'Decision', 'Runbook', 'Playbook', 'Metric', 'Dataset'];
  let okfType = $state('Reference');

  function body(title: string): string {
    if (template === 'concept') {
      const ts = new Date().toISOString().replace(/\.\d+Z$/, 'Z');
      return `---\ntype: ${okfType}\ntitle: ${title}\ndescription: \ntags: []\ntimestamp: ${ts}\n---\n\n# Overview\n\n\n\n# Citations\n\n`;
    }
    return `# ${title}\n\n`;
  }

  function create(): void {
    const n = name.trim();
    if (!n) return;
    const file = n.endsWith('.md') ? n : `${n}.md`;
    const path = dir ? `${dir}/${file}` : file;
    open = false;
    void vault.createNote(path, body(n.replace(/\.md$/i, '')));
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="overlay" onclick={() => (open = false)}>
    <div class="panel" role="dialog" tabindex="-1" aria-label="New note" onclick={(e) => e.stopPropagation()}>
      <h3>New note{dir ? ` in ${dir}/` : ''}</h3>
      <input
        bind:this={input}
        bind:value={name}
        placeholder="Note name"
        onkeydown={(e) => {
          if (e.key === 'Enter') create();
          if (e.key === 'Escape') open = false;
        }}
      />
      {#if vault.current?.okf}
        <div class="row">
          <label>
            <input type="radio" bind:group={template} value="concept" />
            OKF concept
          </label>
          <label>
            <input type="radio" bind:group={template} value="blank" />
            Blank
          </label>
          {#if template === 'concept'}
            <select bind:value={okfType}>
              {#each OKF_TYPES as t (t)}<option value={t}>{t}</option>{/each}
            </select>
          {/if}
        </div>
      {/if}
      <div class="actions">
        <button onclick={() => (open = false)}>Cancel</button>
        <button class="primary" disabled={!name.trim()} onclick={create}>Create</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 90;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 18vh;
  }
  .panel {
    width: min(440px, 92vw);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  input:not([type]) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    padding: 8px 10px;
  }
  .row {
    display: flex;
    gap: 14px;
    align-items: center;
    font-size: 12.5px;
  }
  select {
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 6px;
    font-size: 12px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: 7px;
    padding: 6px 14px;
    cursor: pointer;
    font-size: 12.5px;
  }
  .actions .primary {
    background: var(--accent, #4c6fff);
    border-color: transparent;
    color: var(--accent-contrast, #fff);
  }
  .actions .primary:disabled {
    opacity: 0.5;
  }
</style>
