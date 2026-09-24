<script lang="ts">
  // One "Import" sheet for every way in: paste a curl command (opens it as a
  // request tab), a Postman / OpenAPI / HAR file (becomes a collection), or a
  // whole Postman account (every collection + environment via the Postman API).
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { detectAndParse } from '../../lib/api/importers';

  type Mode = 'curl' | 'file' | 'postman';
  interface Props {
    initial?: Mode;
    onclose: () => void;
    /** Called after a curl import opened its tab (so the page can show it). */
    onimported?: () => void;
  }
  let { initial = 'curl', onclose, onimported }: Props = $props();

  // svelte-ignore state_referenced_locally
  let mode = $state<Mode>(initial);
  const canEdit = $derived(ws.myRole !== 'viewer');
  const MODES: { id: Mode; label: string }[] = [
    { id: 'curl', label: 'curl command' },
    { id: 'file', label: 'File' },
    { id: 'postman', label: 'Postman account' },
  ];

  let curl = $state('');
  let busy = $state(false);
  let pmKey = $state('');
  let pmRemember = $state(true);

  async function importCurl(): Promise<void> {
    busy = true;
    try {
      if (await apiClient.importCurl(curl)) {
        onimported?.();
        onclose();
      }
    } finally {
      busy = false;
    }
  }

  async function importFile(input: HTMLInputElement): Promise<void> {
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    // Close right away — folders + requests are created one by one and the
    // result is reported with a toast.
    onclose();
    try {
      await apiClient.importParsed(detectAndParse(await file.text(), file.name));
    } catch (e) {
      toasts.error('Couldn’t import the file', e instanceof Error ? e.message : String(e));
    }
  }

  async function syncPostman(): Promise<void> {
    busy = true;
    try {
      if (await apiClient.postmanSync(pmKey.trim(), pmRemember)) onclose();
    } finally {
      busy = false;
    }
  }

  function onTabKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const next = MODES[(i + (e.key === 'ArrowRight' ? 1 : MODES.length - 1)) % MODES.length];
    mode = next.id;
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role=tab]')[MODES.indexOf(next)]?.focus();
  }
</script>

<Modal title="Import" width={520} {onclose}>
  <div class="imp">
    <div class="segmented" role="tablist" aria-label="Import from">
      {#each MODES as m, i (m.id)}
        <button role="tab" aria-selected={mode === m.id} class:active={mode === m.id} tabindex={mode === m.id ? 0 : -1}
          onclick={() => (mode = m.id)} onkeydown={(e) => onTabKey(e, i)}>{m.label}</button>
      {/each}
    </div>

    {#if mode === 'curl'}
      <p class="lead">Paste a <code>curl</code> command, for example from your browser’s dev tools (“Copy as cURL”). It opens as a new, unsaved request.</p>
      <textarea class="input mono curl" rows="6" bind:value={curl} spellcheck="false" aria-label="curl command"
        placeholder={"curl https://api.example.com/v1/users \\\n  -H 'Authorization: Bearer {{api_token}}'"}></textarea>
      <p class="hint">Tip: pasting a curl command straight into the URL field works too.</p>
    {:else if mode === 'file'}
      <p class="lead">Turn an exported file into a collection with its folders and requests. Otto recognises <strong>Postman v2.1</strong> collections and environments, <strong>OpenAPI 3 / Swagger</strong> (JSON or YAML) and <strong>HAR</strong> recordings.</p>
      <label class="btn pick" class:disabled={!canEdit}>
        <Icon name="file" size={13} />Choose file…
        <input type="file" accept=".json,.har,.yaml,.yml" hidden disabled={!canEdit} onchange={(e) => void importFile(e.currentTarget as HTMLInputElement)} />
      </label>
      {#if !canEdit}<p class="hint">You have viewer access to this workspace, so you can’t add collections.</p>{/if}
    {:else}
      <p class="lead">Fetch <strong>every collection and environment</strong> from your Postman account in one go, using a Postman API key (postman.co → Settings → API keys). Otto calls api.getpostman.com from the daemon.</p>
      <div class="field">
        <label for="pm-key">Postman API key</label>
        <input id="pm-key" class="input" type="password" bind:value={pmKey} placeholder="PMAK-…" autocomplete="off" disabled={!canEdit} />
        <span class="hint">Leave empty to reuse a key you saved before.</span>
      </div>
      <label class="checkbox-row"><input type="checkbox" bind:checked={pmRemember} disabled={!canEdit} /> Remember the key in the macOS Keychain</label>
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if mode === 'curl'}
      <button class="btn primary" onclick={importCurl} disabled={busy || !curl.trim()}>{busy ? 'Importing…' : 'Open as request'}</button>
    {:else if mode === 'postman'}
      <button class="btn primary" onclick={syncPostman} disabled={busy || !canEdit}>{busy ? 'Importing…' : 'Import everything'}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .imp {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .segmented {
    align-self: flex-start;
  }
  .lead {
    margin: 0;
    font-size: var(--fs-m);
    line-height: 1.5;
    color: var(--text);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .curl {
    width: 100%;
    font-size: var(--fs-s);
  }
  .pick {
    align-self: flex-start;
    cursor: pointer;
  }
  .pick.disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
</style>
