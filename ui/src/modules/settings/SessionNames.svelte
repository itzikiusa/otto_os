<script lang="ts">
  // Session name themes: pick the theme new agent sessions are auto-named from
  // (e.g. "Ronaldo", "Messi") and manage your own custom name lists (family
  // names, …). Per-user; backed by /name-themes.
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    NameThemesResp,
    NameThemeInfo,
    CustomThemeResp,
  } from '../../lib/api/types';

  let resp = $state<NameThemesResp | null>(null);
  let loading = $state(true);
  let saving = $state(false);

  // New custom-theme form.
  let newLabel = $state('');
  let newNames = $state('');
  let creating = $state(false);

  const NONE_ID = 'none';

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      resp = await api.get<NameThemesResp>('/name-themes');
    } catch (e) {
      toasts.error('Could not load name themes', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function setActive(id: string): Promise<void> {
    if (saving || resp?.active === id) return;
    saving = true;
    const prev = resp?.active;
    if (resp) resp.active = id; // optimistic
    try {
      resp = await api.put<NameThemesResp>('/name-themes/active', { theme_id: id });
    } catch (e) {
      if (resp && prev) resp.active = prev; // revert
      toasts.error('Could not set theme', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function createTheme(): Promise<void> {
    const label = newLabel.trim();
    const names = newNames
      .split('\n')
      .map((n) => n.trim())
      .filter((n) => n.length > 0);
    if (!label) {
      toasts.warn('Name your theme', 'Give the custom theme a label first.');
      return;
    }
    if (names.length === 0) {
      toasts.warn('Add some names', 'Enter at least one name (one per line).');
      return;
    }
    creating = true;
    try {
      await api.post<CustomThemeResp>('/name-themes', { label, names });
      newLabel = '';
      newNames = '';
      await load();
      toasts.success('Custom theme created', `${label} · ${names.length} names`);
    } catch (e) {
      toasts.error('Could not create theme', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  async function deleteTheme(t: NameThemeInfo): Promise<void> {
    if (!confirm(`Delete custom theme “${t.label}”?`)) return;
    try {
      await api.del(`/name-themes/${t.id}`);
      await load();
    } catch (e) {
      toasts.error('Could not delete theme', e instanceof Error ? e.message : String(e));
    }
  }

  const builtins = $derived((resp?.themes ?? []).filter((t) => t.kind === 'builtin'));
  const customs = $derived((resp?.themes ?? []).filter((t) => t.kind === 'custom'));
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Session Names</h1>
      <div class="sub">
        New agent sessions are auto-named from your active theme (e.g.
        <strong>Ronaldo</strong>) instead of <code>claude #3</code> — unique among your open
        sessions. Address one by name from ⌘I or Broadcast: <code>ronaldo: run the tests</code>.
      </div>
    </div>
  </div>

  {#if loading}
    <div class="card pad dim">Loading…</div>
  {:else if resp}
    <div class="section-title">Active theme</div>
    <div class="theme-grid">
      <!-- Numbered (legacy) -->
      <button
        type="button"
        class="theme-card"
        class:active={resp.active === NONE_ID}
        disabled={saving}
        onclick={() => setActive(NONE_ID)}
      >
        <div class="theme-head">
          <span class="theme-label">Numbered</span>
          {#if resp.active === NONE_ID}<span class="badge-on">Active</span>{/if}
        </div>
        <div class="theme-sample dim">claude #1 · codex #2 · shell #3</div>
      </button>

      {#each builtins as t (t.id)}
        <button
          type="button"
          class="theme-card"
          class:active={resp.active === t.id}
          disabled={saving}
          onclick={() => setActive(t.id)}
        >
          <div class="theme-head">
            <span class="theme-label">{t.label}</span>
            {#if resp.active === t.id}<span class="badge-on">Active</span>{/if}
          </div>
          <div class="theme-sample dim">{t.sample.join(' · ')} …</div>
          <div class="theme-cap dim">{t.capacity.toLocaleString()} names</div>
        </button>
      {/each}

      {#each customs as t (t.id)}
        <button
          type="button"
          class="theme-card"
          class:active={resp.active === t.id}
          disabled={saving}
          onclick={() => setActive(t.id)}
        >
          <div class="theme-head">
            <span class="theme-label">{t.label} <span class="tag">custom</span></span>
            {#if resp.active === t.id}<span class="badge-on">Active</span>{/if}
          </div>
          <div class="theme-sample dim">{t.sample.join(' · ') || '(no names yet)'}</div>
          <div class="theme-cap dim">
            {t.capacity} name{t.capacity === 1 ? '' : 's'} · recycles with #2, #3… when exhausted
          </div>
        </button>
      {/each}
    </div>

    <div class="section-title">Your custom themes</div>
    <div class="card pad">
      {#if customs.length > 0}
        <ul class="custom-list">
          {#each customs as t (t.id)}
            <li>
              <span class="cl-label">{t.label}</span>
              <span class="cl-names dim">{t.sample.join(', ')}{t.capacity > t.sample.length ? '…' : ''}</span>
              <button class="link-danger" onclick={() => deleteTheme(t)}>Delete</button>
            </li>
          {/each}
        </ul>
      {:else}
        <div class="dim">No custom themes yet. Add one below — e.g. your family names.</div>
      {/if}

      <div class="new-form">
        <div class="field">
          <label for="nt-label">New custom theme</label>
          <input id="nt-label" class="input" placeholder="e.g. Family" bind:value={newLabel} />
        </div>
        <div class="field">
          <label for="nt-names">Names (one per line, most-used first)</label>
          <textarea
            id="nt-names"
            class="input names-area"
            rows="5"
            placeholder={'Dad\nMom\nSister\nBrother'}
            bind:value={newNames}
          ></textarea>
        </div>
        <button class="btn" disabled={creating} onclick={createTheme}>
          {creating ? 'Creating…' : 'Create theme'}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .card.pad {
    padding: 14px 16px;
    max-width: 640px;
    margin-bottom: 8px;
  }
  .sub code {
    background: var(--surface-2);
    border-radius: 4px;
    padding: 0 4px;
    font-size: 11px;
  }
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 10px;
    max-width: 640px;
    margin-bottom: 10px;
  }
  .theme-card {
    text-align: start;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 4px;
    transition: border-color 120ms ease-out, background 120ms ease-out;
  }
  .theme-card:hover {
    border-color: var(--accent);
  }
  .theme-card.active {
    border-color: #7ee787;
    background: color-mix(in srgb, #7ee787 12%, var(--surface));
  }
  .theme-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .theme-label {
    font-size: 12.5px;
    font-weight: 600;
  }
  .tag {
    font-size: 9.5px;
    font-weight: 500;
    text-transform: uppercase;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 3px;
    vertical-align: middle;
  }
  .badge-on {
    font-size: 10px;
    font-weight: 600;
    color: #052e10;
    background: #7ee787;
    border-radius: 4px;
    padding: 1px 5px;
  }
  .theme-sample {
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .theme-cap {
    font-size: 10.5px;
  }
  .custom-list {
    list-style: none;
    margin: 0 0 10px;
    padding: 0;
  }
  .custom-list li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .cl-label {
    font-size: 12.5px;
    font-weight: 600;
    min-width: 100px;
  }
  .cl-names {
    flex: 1;
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .link-danger {
    background: none;
    border: none;
    color: var(--danger, #f97583);
    cursor: pointer;
    font-size: 11.5px;
  }
  .new-form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 360px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .names-area {
    resize: vertical;
    font-family: var(--mono, monospace);
    line-height: 1.5;
  }
</style>
