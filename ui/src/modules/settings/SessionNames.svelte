<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Session name themes: pick the theme new agent sessions are auto-named from
  // (e.g. "Ronaldo", "Messi") and manage your own custom name lists (family
  // names, …). Per-user; backed by /name-themes.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import SectionIntro from './SectionIntro.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import type {
    NameThemesResp,
    NameThemeInfo,
    CustomThemeResp,
  } from '../../lib/api/types';

  let resp = $state<NameThemesResp | null>(null);
  let loading = $state(true);
  // A failed load renders inline with Retry (it used to toast and leave the
  // page blank under the intro).
  let loadError = $state('');
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
    loadError = '';
    try {
      resp = await api.get<NameThemesResp>('/name-themes');
    } catch (e) {
      loadError = loadErrorText(e);
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
      toasts.error("Couldn't switch the name theme", loadErrorText(e));
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
    if (!label || names.length === 0) return;
    creating = true;
    try {
      await api.post<CustomThemeResp>('/name-themes', { label, names });
      newLabel = '';
      newNames = '';
      await load();
      toasts.success('Custom theme created', `${label} · ${names.length} names`);
    } catch (e) {
      toasts.error("Couldn't create the theme", loadErrorText(e));
    } finally {
      creating = false;
    }
  }

  async function deleteTheme(t: NameThemeInfo): Promise<void> {
    const active = resp?.active === t.id;
    if (
      !(await confirmer.ask(
        `Delete custom theme “${t.label}” and its ${t.capacity} name${t.capacity === 1 ? '' : 's'}?${active ? ' New sessions go back to numbered names.' : ''} Sessions already named from it keep their names.`,
        { title: 'Delete theme' },
      ))
    )
      return;
    try {
      await api.del(`/name-themes/${t.id}`);
      await load();
      toasts.success('Theme deleted', t.label);
    } catch (e) {
      toasts.error("Couldn't delete the theme", loadErrorText(e));
    }
  }

  const builtins = $derived((resp?.themes ?? []).filter((t) => t.kind === 'builtin'));
  const customs = $derived((resp?.themes ?? []).filter((t) => t.kind === 'custom'));
  const newNameCount = $derived(newNames.split('\n').filter((n) => n.trim()).length);
  const canCreate = $derived(newLabel.trim() !== '' && newNameCount > 0);
  // A half-typed custom theme isn't lost to a stray sidebar click.
  $effect(() => guardUnsaved(() => !creating && (newLabel.trim() !== '' || newNames.trim() !== ''), { what: 'the new custom theme' }));
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('session-names')} subtitle="Auto-names for new agent sessions" />
  <PageBody width="readable">
  <SectionIntro>New agent sessions are named from your active theme (e.g. <strong>Ronaldo</strong>) instead of <code>claude #3</code>, unique among your open sessions. Address one by name from ⌘I or Broadcast: <code>ronaldo: run the tests</code>.</SectionIntro>

  <LoadState what="name themes" {loading} error={loadError} empty={!resp} onretry={() => void load()} rows={3}>
  {#if resp}
    {#snippet themeCard(id: string, label: string, sample: string, cap: string, custom: boolean)}
      {@const on = resp?.active === id}
      <button
        type="button"
        class="theme-card"
        class:active={on}
        aria-pressed={on}
        disabled={saving}
        onclick={() => setActive(id)}
      >
        <span class="theme-head">
          <span class="theme-label" title={label}>{label}</span>
          {#if custom}<span class="chip">Custom</span>{/if}
          {#if on}<span class="on-mark" aria-hidden="true"><Icon name="check" size={12} /></span>{/if}
        </span>
        <span class="theme-sample" title={sample}>{sample}</span>
        {#if cap}<span class="theme-cap">{cap}</span>{/if}
      </button>
    {/snippet}

    <div class="section-title">Active theme</div>
    <div class="theme-grid" role="group" aria-label="Name theme">
      {@render themeCard(NONE_ID, 'Numbered', 'claude #1 · codex #2 · shell #3', '', false)}
      {#each builtins as t (t.id)}
        {@render themeCard(t.id, t.label, t.sample.join(' · '), `${t.capacity.toLocaleString()} names`, false)}
      {/each}
      {#each customs as t (t.id)}
        {@render themeCard(
          t.id,
          t.label,
          t.sample.join(' · ') || 'No names yet',
          `${t.capacity} name${t.capacity === 1 ? '' : 's'} · then #2, #3…`,
          true,
        )}
      {/each}
    </div>

    <div class="section-title">Custom themes</div>
    {#if customs.length > 0}
      <ul class="custom-list card">
        {#each customs as t (t.id)}
          <li>
            <span class="cl-label" title={t.label}>{t.label}</span>
            <span class="cl-names" title={t.sample.join(', ')}>{t.sample.join(', ')}{t.capacity > t.sample.length ? '…' : ''}</span>
            <button class="icon-btn" title="Delete {t.label}" aria-label="Delete {t.label}" onclick={() => deleteTheme(t)}>
              <Icon name="trash" size={14} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="card s-card new-form">
      <div class="new-title">New custom theme</div>
      <p class="new-sub">Your own list of names — family, a team, a band. Sessions use them in order.</p>
      <div class="field">
        <label for="nt-label">Theme name</label>
        <input id="nt-label" class="input" placeholder="Family" bind:value={newLabel} />
      </div>
      <div class="field">
        <label for="nt-names">Names <span class="dim">(one per line, most-used first)</span></label>
        <textarea
          id="nt-names"
          class="input names-area"
          rows="5"
          placeholder={'Dad\nMom\nSister\nBrother'}
          bind:value={newNames}
        ></textarea>
        <span class="hint">{newNameCount} name{newNameCount === 1 ? '' : 's'}</span>
      </div>
      <div class="form-actions">
        <button
          class="btn primary"
          disabled={creating || !canCreate}
          title={canCreate ? undefined : 'Enter a theme name and at least one name'}
          onclick={createTheme}
        >
          <Icon name="plus" size={13} /> {creating ? 'Creating…' : 'Create theme'}
        </button>
      </div>
    </div>
  {/if}
  </LoadState>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .s-card {
    padding: 14px 16px;
  }
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 8px;
    max-width: var(--settings-col);
  }
  .theme-card {
    text-align: start;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    color: var(--text);
    transition: border-color 120ms ease-out, background 120ms ease-out;
  }
  .theme-card:hover:not(:disabled) {
    background: var(--hover);
  }
  /* Selection = the accent tint + a strong border (never green: green means success). */
  .theme-card.active {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .theme-card:disabled {
    cursor: default;
  }
  .theme-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .theme-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .on-mark {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    flex-shrink: 0;
  }
  .theme-sample,
  .theme-cap {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .custom-list {
    list-style: none;
    margin: 0 0 8px;
    padding: 4px 8px 4px 14px;
    max-width: var(--settings-col);
    box-sizing: border-box;
  }
  .custom-list li {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 32px;
  }
  .custom-list li + li {
    border-top: 1px solid var(--border);
  }
  .cl-label {
    font-size: var(--fs-m);
    font-weight: 600;
    width: 140px;
    flex-shrink: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cl-names {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .new-form {
    max-width: var(--settings-col);
    box-sizing: border-box;
  }
  .new-title {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .new-sub {
    margin: 2px 0 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .new-form .field {
    max-width: 380px;
  }
  .names-area {
    font-family: var(--font-mono);
  }
  .form-actions {
    display: flex;
  }
</style>
