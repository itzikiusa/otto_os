<script lang="ts">
  // New-note dialog: name + folder + (OKF vaults) a concept-template picker.
  import { vault } from './vault.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { okfConceptTemplate } from './okfTemplate';
  import Modal from '../../lib/components/Modal.svelte';

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

  const OKF_TYPES = ['Service', 'Reference', 'Decision', 'Runbook', 'Playbook', 'Metric', 'Dataset', 'Attested Computation'];
  let okfType = $state('Reference');

  function body(title: string): string {
    if (template === 'concept') {
      return okfConceptTemplate(okfType, title, auth.me ? `human:${auth.me.id}` : 'process:otto-note-editor');
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
  <Modal title="New note{dir ? ` in ${dir}/` : ''}" width={440} onclose={() => (open = false)}>
    <div class="nn-body">
      <input
        bind:this={input}
        bind:value={name}
        class="nn-name"
        placeholder="Note name"
        aria-label="Note name"
        onkeydown={(e) => {
          if (e.key === 'Enter') create();
        }}
      />
      {#if vault.current?.okf}
        <div class="nn-template">
          <label>
            <input type="radio" bind:group={template} value="concept" />
            OKF concept
          </label>
          <label>
            <input type="radio" bind:group={template} value="blank" />
            Blank
          </label>
          {#if template === 'concept'}
            <select bind:value={okfType} aria-label="OKF concept type">
              {#each OKF_TYPES as t (t)}<option value={t}>{t}</option>{/each}
            </select>
          {/if}
        </div>
      {/if}
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (open = false)}>Cancel</button>
      <button class="btn primary" disabled={!name.trim()} onclick={create}>Create</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .nn-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .nn-name {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    padding: 8px 10px;
  }
  .nn-template {
    display: flex;
    gap: 14px;
    align-items: center;
    font-size: var(--fs-s);
  }
  .nn-template select {
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    font-size: var(--fs-s);
  }
</style>
