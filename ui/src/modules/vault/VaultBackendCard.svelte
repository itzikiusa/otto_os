<script lang="ts">
  // One remote-backend card (Qdrant / SurrealDB / Ollama): status, an edit form
  // (enabled / url / role / secret), a Test button, and a deliberate Install
  // flow — Install → show the plan in a confirm dialog → run → show the log.
  import { untrack } from 'svelte';
  import { vault } from './vault.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { VaultBackend, VaultBackendReq, VaultInstallPlan } from '../../lib/api/types';

  interface Props {
    kind: string;
    backend: VaultBackend | null;
  }
  let { kind, backend }: Props = $props();

  const META: Record<string, { label: string; icon: string; role: string; url: string; blurb: string }> = {
    qdrant: { label: 'Qdrant', icon: 'db', role: 'vector', url: 'http://127.0.0.1:6333', blurb: 'Vector search backend for semantic recall.' },
    surreal: { label: 'SurrealDB', icon: 'share', role: 'graph', url: 'http://127.0.0.1:8000', blurb: 'Graph backend for the knowledge + code graph.' },
    ollama: { label: 'Ollama', icon: 'box', role: 'embed', url: 'http://127.0.0.1:11434', blurb: 'Local embeddings provider.' },
  };
  const meta = $derived(META[kind] ?? { label: kind, icon: 'plug', role: 'vector', url: '', blurb: '' });

  // Local edit state, seeded once from the saved backend (or sensible defaults);
  // re-seeded by the $effect below when the saved backend changes. `untrack`
  // marks the initial read as intentional (not a reactive dependency).
  let enabled = $state(untrack(() => backend?.enabled ?? false));
  let url = $state(untrack(() => backend?.url || meta.url));
  let role = $state(untrack(() => backend?.role || meta.role));
  let secret = $state('');
  let saving = $state(false);
  let testing = $state(false);

  // Install flow.
  let plan = $state<VaultInstallPlan | null>(null);
  let showPlan = $state(false);
  let installing = $state(false);
  let installLog = $state('');

  // Re-seed when the saved backend changes (after a save/reload).
  $effect(() => {
    if (backend) {
      enabled = backend.enabled;
      url = backend.url || meta.url;
      role = backend.role || meta.role;
    }
  });

  const status = $derived(backend?.status ?? 'unknown');
  function statusClass(s: string): string {
    if (s === 'ok') return 'ok';
    if (s === 'installing') return 'busy';
    if (s === 'error') return 'err';
    return 'idle';
  }

  async function save() {
    saving = true;
    try {
      const req: VaultBackendReq = { enabled, url: url.trim(), role };
      if (secret.trim()) req.secret = secret.trim();
      const ok = await vault.saveBackend(kind, req);
      if (ok) secret = '';
    } finally {
      saving = false;
    }
  }

  async function test() {
    testing = true;
    try {
      // Persist the current form first: the health endpoint 404s on a backend
      // that was never saved, and we want to test the latest URL/secret.
      const req: VaultBackendReq = { enabled, url: url.trim(), role };
      if (secret.trim()) req.secret = secret.trim();
      const ok = await vault.saveBackend(kind, req);
      if (ok) {
        secret = '';
        await vault.testBackend(kind);
      }
    } finally {
      testing = false;
    }
  }

  async function openInstall() {
    plan = await vault.planInstall(kind);
    if (plan) showPlan = true;
  }

  async function confirmInstall() {
    installing = true;
    installLog = '';
    try {
      const r = await vault.installBackend(kind);
      if (r) installLog = r.log;
    } finally {
      installing = false;
    }
  }
</script>

<article class="bk-card">
  <header>
    <span class="bk-icon" style:--c={status === 'ok' ? '#7ee787' : 'var(--text-dim)'}>
      <Icon name={meta.icon} size={16} />
    </span>
    <div class="bk-title">
      <span class="bk-name">{meta.label}</span>
      <span class="bk-kind">{kind}</span>
    </div>
    <span class="status {statusClass(status)}">{status}</span>
  </header>
  <p class="bk-blurb">{meta.blurb}</p>
  {#if backend?.message}
    <p class="bk-msg {statusClass(status)}">{backend.message}</p>
  {/if}

  <div class="bk-form">
    <label class="bk-row toggle">
      <input type="checkbox" bind:checked={enabled} />
      <span>Enabled</span>
    </label>
    <label class="bk-row">
      <span>Role</span>
      <select bind:value={role}>
        <option value="vector">vector</option>
        <option value="graph">graph</option>
        <option value="embed">embed</option>
      </select>
    </label>
    <label class="bk-row">
      <span>URL</span>
      <input type="text" bind:value={url} placeholder={meta.url} />
    </label>
    <label class="bk-row">
      <span>Secret</span>
      <input type="password" bind:value={secret} placeholder="api key / password (optional)" />
    </label>
  </div>

  <footer>
    <button class="bk-btn primary" disabled={saving} onclick={save}>
      {saving ? 'Saving…' : 'Save'}
    </button>
    <button class="bk-btn" disabled={testing} onclick={test}>
      {#if testing}Testing…{:else}<Icon name="radar" size={13} /> Test{/if}
    </button>
    <button class="bk-btn install" onclick={openInstall} title="Install / provision this backend">
      <Icon name="arrowDown" size={13} /> Install
    </button>
  </footer>

  {#if installLog}
    <pre class="bk-log">{installLog}</pre>
  {/if}
</article>

<!-- Install confirm dialog -->
{#if showPlan && plan}
  <div
    class="modal-back"
    role="button"
    tabindex="0"
    aria-label="Close install dialog"
    onclick={() => (showPlan = false)}
    onkeydown={(e) => e.key === 'Escape' && (showPlan = false)}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label={`Install ${meta.label}`}
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <h3>Install {meta.label}</h3>
      <div class="plan-meta">
        <span class="pill">method: <b>{plan.method}</b></span>
        <span class="pill" class:ready={plan.ready}>{plan.ready ? 'already available' : 'not installed'}</span>
      </div>
      {#if plan.steps.length}
        <div class="plan-steps">
          <div class="plan-label">Steps</div>
          <ol>
            {#each plan.steps as step, i (i)}
              <li><code>{step}</code></li>
            {/each}
          </ol>
        </div>
      {/if}
      {#if plan.health_url}
        <p class="plan-health">Health check: <code>{plan.health_url}</code></p>
      {/if}
      {#if plan.notes}
        <p class="plan-notes">{plan.notes}</p>
      {/if}
      <div class="modal-actions">
        <button class="bk-btn" onclick={() => (showPlan = false)}>Cancel</button>
        <button
          class="bk-btn primary"
          disabled={installing || plan.method === 'none'}
          onclick={confirmInstall}
        >
          {#if installing}Installing…{:else}Confirm install{/if}
        </button>
      </div>
      {#if installLog}
        <pre class="bk-log">{installLog}</pre>
      {/if}
    </div>
  </div>
{/if}

<style>
  .bk-card {
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 14px 16px;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  header { display: flex; align-items: center; gap: 9px; }
  .bk-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--c) 16%, transparent);
    color: var(--c);
  }
  .bk-title { flex: 1; display: flex; flex-direction: column; }
  .bk-name { font-weight: 700; font-size: 14px; }
  .bk-kind { font-size: 10.5px; color: var(--text-dim); }
  .status {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .status.ok { background: color-mix(in srgb, #7ee787 22%, transparent); color: #4cae5a; }
  .status.busy { background: color-mix(in srgb, #ffd43b 22%, transparent); color: #c79400; }
  .status.err { background: color-mix(in srgb, #ff6b6b 22%, transparent); color: #ff6b6b; }
  .status.idle { background: var(--surface-2); color: var(--text-dim); }
  .bk-blurb { font-size: 12px; color: var(--text-dim); margin: 0; }
  .bk-msg { font-size: 11.5px; margin: 0; }
  .bk-msg.err { color: #ff6b6b; }

  .bk-form { display: flex; flex-direction: column; gap: 6px; }
  .bk-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .bk-row > span { width: 56px; flex: none; }
  .bk-row.toggle { gap: 7px; }
  .bk-row.toggle input { accent-color: #7ee787; }
  .bk-row select,
  .bk-row input {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }

  footer { display: flex; gap: 7px; flex-wrap: wrap; }
  .bk-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    padding: 6px 12px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .bk-btn:hover:not(:disabled) { color: var(--text); }
  .bk-btn:disabled { opacity: 0.45; cursor: default; }
  .bk-btn.primary {
    background: #7ee787;
    color: #0b0b0b;
    border-color: #7ee787;
    font-weight: 600;
  }
  .bk-btn.install { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
  .bk-log {
    margin: 4px 0 0;
    max-height: 200px;
    overflow: auto;
    font-size: 11px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* modal */
  .modal-back {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
  }
  .modal {
    width: min(560px, 100%);
    max-height: 86vh;
    overflow: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 18px 20px;
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.5);
  }
  .modal h3 { margin: 0 0 10px; font-size: 16px; }
  .plan-meta { display: flex; gap: 8px; flex-wrap: wrap; margin-bottom: 12px; }
  .pill {
    font-size: 11.5px;
    padding: 3px 9px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .pill.ready { background: color-mix(in srgb, #7ee787 22%, transparent); color: #4cae5a; }
  .plan-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .plan-steps ol { margin: 0 0 12px; padding-inline-start: 20px; }
  .plan-steps li { margin: 4px 0; }
  .plan-steps code,
  .plan-health code {
    font-size: 11.5px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    word-break: break-all;
  }
  .plan-health { font-size: 12px; color: var(--text-dim); }
  .plan-notes {
    font-size: 12px;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 7px;
    padding: 8px 10px;
  }
  .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
</style>
