<script lang="ts">
  // MCP → Profiles: named variants of the Otto MCP connection, each exposing
  // only a subset of the tools (Database, Issues, Git & PRs, Cloud, or your
  // own). One profile is the default every new session preselects; Full —
  // every tool, the connection as it always was — can't be edited or deleted.
  // Each profile also gets its own connection config, so an outside Claude
  // Code or Codex can attach "Otto · Database" only.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { baseUrl } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
  import type { McpProfile } from '../../lib/api/types';
  import {
    connectionConfigs,
    draftOf,
    duplicateName,
    emptyDraft,
    profileSummary,
    profileTools,
    type ProfileDraft,
  } from '../../lib/mcpProfiles';
  import ProfileEditor from './ProfileEditor.svelte';

  const isAdmin = $derived(auth.can('mcp', 'admin'));
  let selectedId = $state<string | null>(null);
  let editor = $state<{ editing: McpProfile | null; initial: ProfileDraft } | null>(null);
  let busy = $state(false);
  const CONFIG_TABS = [
    { id: 'stdio', label: '.mcp.json' },
    { id: 'http', label: 'HTTP' },
    { id: 'codex', label: 'Codex' },
  ] as const;
  let configTab = $state<'stdio' | 'http' | 'codex'>('stdio');

  // Untracked: `load` reads the store's `loaded`, so a tracked call re-ran
  // this effect when the first response flipped it and fetched twice.
  $effect(() => {
    untrack(() => void mcpProfiles.load(true));
  });

  const profiles = $derived(mcpProfiles.profiles);
  const groups = $derived(mcpProfiles.groups);
  // List/detail opens on an item: the selection, else the default.
  const selected = $derived(
    profiles.find((p) => p.id === selectedId) ?? profiles.find((p) => p.is_default) ?? profiles[0] ?? null,
  );
  const cfg = $derived(selected ? connectionConfigs(selected, baseUrl()) : null);
  const tools = $derived(selected ? profileTools(groups, selected) : []);
  const configText = $derived(
    !cfg ? '' : configTab === 'stdio' ? cfg.stdioJson : configTab === 'http' ? cfg.claudeHttp : cfg.codexToml,
  );

  async function reload(selectId?: string): Promise<void> {
    await mcpProfiles.load(true);
    if (selectId) selectedId = selectId;
  }

  function newProfile(): void {
    editor = { editing: null, initial: emptyDraft() };
  }

  function duplicate(p: McpProfile): void {
    editor = { editing: null, initial: { ...draftOf(p), name: duplicateName(p.name, profiles) } };
  }

  async function makeDefault(p: McpProfile): Promise<void> {
    busy = true;
    try {
      await mcpCpApi.cpSetDefaultProfile(p.id);
      toasts.success(`${p.name} is the default`, 'New sessions preselect it; sessions without a profile use it on their next start.');
      await reload(p.id);
    } catch (e) {
      toastError('Couldn’t change the default', e);
    } finally {
      busy = false;
    }
  }

  async function remove(p: McpProfile): Promise<void> {
    const tail = p.is_default ? ' It is the default — Full becomes the default again.' : '';
    if (
      !(await confirmer.ask(
        `Delete “${p.name}”? Sessions that use it fall back to their other profiles, or the default, the next time they start.${tail}`,
        { title: 'Delete MCP profile', confirmLabel: 'Delete', danger: true },
      ))
    )
      return;
    busy = true;
    try {
      await mcpCpApi.cpDeleteProfile(p.id);
      toasts.success('Profile deleted', p.name);
      selectedId = null;
      await reload();
    } catch (e) {
      toastError('Couldn’t delete the profile', e);
    } finally {
      busy = false;
    }
  }

  /** Why a profile action is greyed out for a non-admin (a disabled control is never unexplained). */
  const NEEDS_ADMIN = 'Needs MCP admin';

  function menu(e: MouseEvent | KeyboardEvent, p: McpProfile): void {
    // The locked-profile reason wins: it holds even for an admin.
    const locked = p.locked ? 'Built in — duplicate it to make your own' : undefined;
    const admin = isAdmin ? undefined : NEEDS_ADMIN;
    const items: MenuItem[] = [
      { label: 'Edit…', icon: 'edit', disabled: !isAdmin || p.locked, title: locked ?? admin, action: () => (editor = { editing: p, initial: draftOf(p) }) },
      { label: 'Duplicate…', icon: 'copy', disabled: !isAdmin, title: admin, action: () => duplicate(p) },
      { label: 'Make default', icon: 'star', disabled: !isAdmin || p.is_default, title: admin, action: () => void makeDefault(p) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, disabled: !isAdmin || p.locked, title: (p.locked ? 'Full can’t be deleted' : undefined) ?? admin, action: () => void remove(p) },
    ];
    ctxMenu.show(e, items);
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Couldn’t copy', 'Select and copy manually.');
    }
  }
</script>

<div class="profiles" data-testid="mcp-profiles">
  <div class="head">
    <p class="muted small intro">
      A profile is a named slice of Otto’s MCP tools. Sessions pick one or more when they start (the
      default is preselected) and see only those tools; outside agents can attach a single profile.
      Profiles only narrow — permissions, the Otto server checklist and approvals still apply.
    </p>
    <button class="btn small" data-testid="mcp-profile-new" disabled={!isAdmin || !mcpProfiles.loaded} title={isAdmin ? undefined : NEEDS_ADMIN} onclick={newProfile}>
      <Icon name="plus" size={12} /> New profile
    </button>
  </div>

  <LoadState what="MCP profiles" loading={mcpProfiles.loading && !mcpProfiles.loaded} error={mcpProfiles.error} empty={mcpProfiles.loaded && profiles.length === 0} rows={3} onretry={() => void reload()}>
    {#snippet emptyView()}
      <EmptyState icon="layers" title="No MCP profiles" body="The built-in profiles are missing — restart the daemon to restore them." />
    {/snippet}
    <div class="split">
      <ul class="list" aria-label="MCP profiles">
        {#each profiles as p (p.id)}
          <li>
            <button
              class="prof-row"
              class:active={selected?.id === p.id}
              aria-current={selected?.id === p.id ? 'true' : undefined}
              data-testid="mcp-profile-{p.id}"
              onclick={() => (selectedId = p.id)}
              oncontextmenu={(e) => { e.preventDefault(); menu(e, p); }}
            >
              <span class="name">
                {p.name}
                {#if p.is_default}<Badge tone="accent" label="Default" />{/if}
                {#if p.locked}<span class="lock" title="Built in, read-only" aria-label="Built in, read-only"><Icon name="lock" size={12} /></span>{/if}
              </span>
              <span class="sum">{profileSummary(groups, p)}</span>
            </button>
          </li>
        {/each}
      </ul>

      {#if selected && cfg}
        <section class="detail" aria-label="{selected.name} profile">
          <div class="d-head">
            <div class="d-title">
              <h3>{selected.name}</h3>
              {#if selected.is_default}<Badge tone="accent" label="Default" />{/if}
              {#if selected.builtin}<Badge label="Built in" />{/if}
            </div>
            <div class="d-actions">
              {#if !selected.is_default}
                <button class="btn small" disabled={!isAdmin || busy} title={isAdmin ? undefined : NEEDS_ADMIN} onclick={() => void makeDefault(selected)} data-testid="mcp-profile-make-default">Make default</button>
              {/if}
              <button class="icon-btn" aria-label="More actions for {selected.name}" title="More…" aria-haspopup="menu" onclick={(e) => menu(e, selected)}><Icon name="more" size={14} /></button>
            </div>
          </div>
          {#if selected.description}<p class="muted small">{selected.description}</p>{/if}
          <p class="small">{profileSummary(groups, selected)}</p>
          {#if !selected.all_tools}
            <ul class="tool-chips" aria-label="Tools in {selected.name}">
              {#each tools as t (t)}<li class="tchip mono">{t}</li>{/each}
            </ul>
          {/if}

          <h4 class="sec">Connect from outside</h4>
          <p class="muted small">
            Attach an external Claude Code or Codex as <strong>{cfg.label}</strong> only. Mint a token under
            <em>Otto server → Tokens</em> and replace the placeholder.{selected.locked ? ' Full is the plain all-tools connection.' : ''}
          </p>
          <Tabs label="Config format" size="s" tabs={CONFIG_TABS} value={configTab} onchange={(id) => (configTab = id)} />
          <div class="snippet" data-testid="mcp-profile-config">
            <button class="btn small copy" onclick={() => void copy(configText, `${cfg.label} config`)}>
              <Icon name="copy" size={12} /> Copy
            </button>
            <pre>{configText}</pre>
          </div>
          {#if configTab === 'http'}<p class="muted small">Endpoint: <code class="mono">{cfg.httpUrl}</code></p>{/if}
        </section>
      {/if}
    </div>
  </LoadState>
</div>

{#if editor}
  <ProfileEditor
    editing={editor.editing}
    initial={editor.initial}
    {profiles}
    {groups}
    onclose={() => (editor = null)}
    onsaved={(p) => void reload(p.id)}
  />
{/if}

<style>
  .profiles {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .intro {
    margin: 0;
    max-width: 72ch;
  }
  .split {
    display: grid;
    grid-template-columns: minmax(200px, 280px) minmax(0, 1fr);
    gap: 14px;
    align-items: start;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
  }
  .prof-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    width: 100%;
    padding: 8px 12px;
    font: inherit;
    text-align: start;
    color: var(--text);
    background: transparent;
    border: none;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    cursor: pointer;
  }
  li:last-child .prof-row {
    border-bottom: none;
  }
  .prof-row:hover {
    background: var(--surface-2);
  }
  .prof-row.active {
    background: var(--accent-soft);
  }
  .prof-row:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-m);
    overflow-wrap: anywhere;
  }
  .sum {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .lock {
    display: inline-flex;
    color: var(--text-dim);
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 14px;
  }
  .d-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .d-title {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .d-title h3 {
    margin: 0;
    font-size: var(--fs-l);
    overflow-wrap: anywhere;
  }
  .d-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tool-chips {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    max-height: 160px;
    overflow-y: auto;
  }
  .tchip {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 6px;
    overflow-wrap: anywhere;
  }
  .sec {
    margin: 8px 0 0;
    font-size: var(--fs-s);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .snippet {
    position: relative;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .snippet pre {
    margin: 0;
    padding: 10px 12px;
    padding-inline-end: 80px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .copy {
    position: absolute;
    inset-block-start: 6px;
    inset-inline-end: 6px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
    margin: 0;
  }
  @media (max-width: 640px) {
    .split {
      grid-template-columns: minmax(0, 1fr);
    }
    .head {
      flex-direction: column;
    }
  }
</style>
