<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  interface Props {
    editing: string | null;
    name: string;
    cmd: string;
    args: string;
    resumeArgs: string;
    updateCmd: string;
    modelArgs: string;
    modelFamily: string;
    formError: string;
    saving: boolean;
    onclose: () => void;
    onsave: () => void;
  }

  let {
    editing,
    name = $bindable(),
    cmd = $bindable(),
    args = $bindable(),
    resumeArgs = $bindable(),
    updateCmd = $bindable(),
    modelArgs = $bindable(),
    modelFamily = $bindable(),
    formError,
    saving,
    onclose,
    onsave,
  }: Props = $props();
</script>

<Modal title={editing ? `Edit ${editing}` : 'New provider'} width={560} {onclose}>
  <form
    class="grid"
    id="provider-form"
    onsubmit={(e) => {
      e.preventDefault();
      onsave();
    }}
  >
    <div class="field">
      <label for="pv-name">Name</label>
      <input dir="ltr" id="pv-name" class="input mono-in" bind:value={name} placeholder="kilo" spellcheck="false" autocomplete="off" />
    </div>
    <div class="field">
      <label for="pv-cmd">Command</label>
      <input dir="ltr" id="pv-cmd" class="input mono-in" bind:value={cmd} placeholder="kilo" spellcheck="false" autocomplete="off" />
    </div>
    <div class="field">
      <label for="pv-args">Arguments (optional)</label>
      <input dir="ltr" id="pv-args" class="input mono-in" bind:value={args} placeholder={'--session {sid}'} spellcheck="false" />
    </div>
    <div class="field">
      <label for="pv-resume">Resume arguments (optional)</label>
      <input dir="ltr" id="pv-resume" class="input mono-in" bind:value={resumeArgs} placeholder={'--resume {sid}'} spellcheck="false" />
    </div>
    <div class="field">
      <label for="pv-update">Update command (optional)</label>
      <input dir="ltr" id="pv-update" class="input mono-in" bind:value={updateCmd} placeholder={'npm i -g @kilocode/cli'} spellcheck="false" />
    </div>
    <div class="field">
      <label for="pv-model">Model flag template (optional)</label>
      <input dir="ltr" id="pv-model" class="input mono-in" bind:value={modelArgs} placeholder={'--model {model}'} spellcheck="false" />
    </div>
    <div class="field">
      <label for="pv-family">CLI family (optional)</label>
      <select id="pv-family" class="input" bind:value={modelFamily}>
        <option value="">Auto-detect from name or command</option>
        <option value="claude">Claude Code</option>
        <option value="codex">Codex</option>
        <option value="agy">Antigravity</option>
        <option value="gemini">Gemini CLI</option>
        <option value="grok">Grok</option>
        <option value="opencode">OpenCode</option>
        <option value="pi">Pi</option>
      </select>
    </div>
  </form>
  <p class="hint">
    <code>{'{sid}'}</code> and <code>{'{cwd}'}</code> expand in arguments; <code>{'{model}'}</code> in the model flag
    template. Quote an argument that contains spaces (<code>--system 'be terse'</code>). An empty model template
    uses the CLI family's default flag.
  </p>
  {#if formError}<p class="field-err" use:announceShown={{ text: formError, assertive: true }}>{formError}</p>{/if}
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" type="submit" form="provider-form" disabled={saving}>
      {saving ? 'Saving…' : editing ? 'Save provider' : 'Add provider'}
    </button>
  {/snippet}
</Modal>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 12px;
  }
  .mono-in {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .hint {
    margin: 0;
    max-width: 78ch;
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .field-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: var(--radius-s);
  }
  @media (max-width: 640px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
