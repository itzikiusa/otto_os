<script lang="ts">
  // 1-Click "Sync to all providers…" modal: shows every drifted provider copy
  // and its diff against the Otto Library copy before syncing. Writing the
  // library copy triggers `refresh_provider_copies` on the daemon, backing up
  // hand-edited Otto-managed copies to `provider-skills-backup/` first.
  import { onDestroy } from 'svelte';
  import { skillLabApi } from '../../lib/api/skillLab';
  import { loadErrorText } from '../../lib/loadError';
  import { toastError } from '../../lib/toastError';
  import { toasts } from '../../lib/toast.svelte';
  import { plural } from '../../lib/plural';
  import Modal from '../../lib/components/Modal.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { sourceLabel } from './skillGroups';
  import { BACKUP_FOLDER, noteKeptCopies } from './bundledActions';

  interface Props {
    name: string;
    providers: string[];
    onclose: () => void;
    onsynced: () => void;
  }
  let { name, providers, onclose, onsynced }: Props = $props();

  let activeProvider = $state('');
  let libraryBody = $state<string | null>(null);
  let providerBodies = $state<Record<string, string>>({});
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let syncing = $state(false);
  let seq = 0;
  let disposed = false;

  onDestroy(() => {
    disposed = true;
    seq++;
  });

  function providerPath(p: string): string {
    if (p === 'claude') return `~/.claude/skills/${name}`;
    if (p === 'codex') return `~/.codex/skills/${name}`;
    if (p === 'agy') return `~/.gemini/skills/${name}`;
    return `~/.${p}/skills/${name}`;
  }

  async function loadDiffs(): Promise<void> {
    const my = ++seq;
    loading = true;
    loadError = null;
    try {
      const [lib, ...provResults] = await Promise.all([
        skillLabApi.getFile(name, 'SKILL.md'),
        ...providers.map((p) => skillLabApi.getProvider(p, name).then((r) => [p, r.body] as const)),
      ]);
      if (disposed || my !== seq) return;
      libraryBody = lib.content;
      const map: Record<string, string> = {};
      for (const [p, body] of provResults) map[p] = body;
      providerBodies = map;
      if (!activeProvider || !providers.includes(activeProvider)) {
        activeProvider = providers[0] ?? '';
      }
    } catch (e) {
      if (disposed || my !== seq) return;
      loadError = loadErrorText(e);
    } finally {
      if (!disposed && my === seq) loading = false;
    }
  }

  $effect(() => {
    void name;
    void providers;
    void loadDiffs();
  });

  async function syncAll(): Promise<void> {
    if (syncing || libraryBody == null) return;
    syncing = true;
    try {
      const res = await skillLabApi.putFile(name, { path: 'SKILL.md', content: libraryBody });
      noteKeptCopies(res.kept_copies);
      if (disposed) return;
      const labels = providers.map(sourceLabel).join(', ');
      toasts.success(`Synced “${name}” to ${plural(providers.length, 'provider')}`, labels);
      onsynced();
      onclose();
    } catch (e) {
      toastError(`Couldn’t sync ${name} to providers`, e);
    } finally {
      syncing = false;
    }
  }
</script>

<Modal title="Sync “{name}” to providers" width={680} dismissable={!syncing} {onclose}>
  {#snippet children()}
    <p class="sp-lede">
      Updates Otto-managed provider copies from the Library copy of <strong class="mono">{name}</strong>.
      Any hand-edited managed copy is backed up to <span class="mono" dir="ltr">{BACKUP_FOLDER}</span> first;
      user-authored provider skills are never overwritten.
    </p>
    <div class="sp-targets" role="group" aria-label="Drifted providers">
      {#each providers as p (p)}
        <button
          type="button"
          class="sp-chip"
          class:active={activeProvider === p}
          aria-pressed={activeProvider === p}
          title={providerPath(p)}
          onclick={() => (activeProvider = p)}
        >
          <ProviderIcon provider={p} size={12} />
          <span>{sourceLabel(p)}</span>
          <span class="sp-path mono" dir="ltr">{providerPath(p)}</span>
        </button>
      {/each}
    </div>
    <LoadState what="provider diffs" variant="compact" {loading} error={loadError} empty={libraryBody == null} onretry={() => void loadDiffs()}>
      {#if activeProvider && libraryBody != null}
        <div class="sp-diff-head">
          <span class="section-title">{sourceLabel(activeProvider)} copy → Library copy</span>
        </div>
        <div class="sp-diff" data-testid="sync-providers-diff">
          <DiffView before={providerBodies[activeProvider] ?? ''} after={libraryBody} mode="word" contextLines={3} />
        </div>
      {/if}
    </LoadState>
  {/snippet}
  {#snippet footer()}
    <button type="button" class="btn" disabled={syncing} onclick={onclose}>Cancel</button>
    <button
      type="button"
      class="btn primary"
      disabled={syncing || loading || libraryBody == null}
      onclick={() => void syncAll()}
      data-testid="sync-providers-confirm"
    >
      <Icon name="refresh" size={12} />
      {syncing ? 'Syncing…' : `Sync to ${plural(providers.length, 'provider')}`}
    </button>
  {/snippet}
</Modal>

<style>
  .sp-lede {
    margin: 0 0 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .sp-targets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 10px;
  }
  .sp-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .sp-chip.active {
    border-color: var(--accent-text);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .sp-path {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sp-diff-head {
    margin-block-end: 4px;
  }
  .sp-diff {
    max-block-size: 320px;
    overflow: auto;
    border-radius: var(--radius-s);
  }
</style>
