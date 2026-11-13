<script module lang="ts">
  export function parseDotenvBlock(raw: string): { key: string; value: string }[] {
    const out: { key: string; value: string }[] = [];
    const lines = raw.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const trimmed = lines[i].trim();
      if (!trimmed || trimmed.startsWith('#')) continue;
      const stripped = trimmed.replace(/^export\s+/, '');
      const eq = stripped.indexOf('=');
      if (eq <= 0) continue;
      const key = stripped.slice(0, eq).trim();
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) continue;
      let val = stripped.slice(eq + 1).trim();
      if (
        (val.startsWith('"') && val.endsWith('"') && val.length >= 2) ||
        (val.startsWith("'") && val.endsWith("'") && val.length >= 2)
      ) {
        val = val.slice(1, -1);
      }
      out.push({ key, value: val });
    }
    return out;
  }
</script>

<script lang="ts">
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Switch from '../../../lib/components/Switch.svelte';
  import { plural } from '../../../lib/plural';
  import { toasts } from '../../../lib/toast.svelte';
  import { ide } from '../ideStore.svelte';

  interface Props {
    inheritShellEnv: boolean;
    onchangeInherit: (next: boolean) => void;
    onpasteVars?: (items: { key: string; value: string }[]) => void;
  }

  let { inheritShellEnv, onchangeInherit, onpasteVars }: Props = $props();

  let previewOpen = $state(false);
  let filterQuery = $state('');
  let pasteOpen = $state(false);
  let pasteRaw = $state('');

  $effect(() => {
    if (!ide.hasTarget) return;
    if (!ide.shellEnv && !ide.shellEnvLoading) {
      void ide.loadShellEnv(false);
    }
  });

  const HIGHLIGHT_RE = /^(GO_DEBUGGING|GOPRIVATE|GONOSUMDB|GONOPROXY|GOROOT|GOPATH|GOFLAGS|CGO_|DELVE|DLV_|JAVA_HOME|RUST|CARGO_|NODE_|PYTHONPATH|VIRTUAL_ENV|PATH$)/i;

  const sortedShellEntries = $derived.by((): [string, string][] => {
    const vars = ide.shellEnv?.vars ?? ide.effectiveEnv ?? {};
    const q = filterQuery.trim().toLowerCase();
    const entries = Object.entries(vars).filter(([k, v]) => {
      if (!q) return true;
      return k.toLowerCase().includes(q) || v.toLowerCase().includes(q);
    });
    entries.sort(([a], [b]) => {
      const aHi = HIGHLIGHT_RE.test(a) ? 0 : 1;
      const bHi = HIGHLIGHT_RE.test(b) ? 0 : 1;
      if (aHi !== bHi) return aHi - bHi;
      return a.localeCompare(b);
    });
    return entries;
  });

  const visibleEntries = $derived(sortedShellEntries.slice(0, 80));
  const totalCount = $derived(Object.keys(ide.shellEnv?.vars ?? ide.effectiveEnv ?? {}).length);
  const shellLabel = $derived(ide.shellEnv?.shell || '/bin/zsh');

  async function handleReload(): Promise<void> {
    const res = await ide.reloadShellEnv();
    if (res) {
      previewOpen = true;
      toasts.info('Reloaded login shell environment', `${plural(Object.keys(res.vars ?? {}).length, 'variable')} from ${res.shell || 'login shell'}`);
    }
  }

  function applyPastedBlock(): void {
    const items = parseDotenvBlock(pasteRaw);
    if (items.length === 0) {
      toasts.info('No valid KEY=VALUE pairs found');
      return;
    }
    onpasteVars?.(items);
    pasteRaw = '';
    pasteOpen = false;
    toasts.success(`Imported ${plural(items.length, 'variable')}`);
  }
</script>

<section class="senv-card" aria-labelledby="senv-heading" data-testid="ide-shell-env-card">
  <div class="senv-head">
    <div class="senv-title-row">
      <Switch
        checked={inheritShellEnv}
        label="Inherit login shell environment (~/.zshrc, ~/.zshenv)"
        onchange={onchangeInherit}
      />
      <div class="senv-copy">
        <div class="senv-headline">
          <h3 id="senv-heading" class="senv-title">Inherit shell environment (~/.zshrc / ~/.zshenv)</h3>
          <Badge tone={inheritShellEnv ? 'accent' : 'neutral'} variant="outline" label={`${shellLabel} · ${plural(totalCount, 'var')}`} />
        </div>
        <p class="senv-sub">
          Loads variables exported in your interactive login shell (such as <code>GO_DEBUGGING_*</code>, <code>GOPRIVATE</code>, <code>GOROOT</code>, and <code>PATH</code>) into Run &amp; Debug sessions.
        </p>
      </div>
    </div>

    <div class="senv-actions">
      <button
        type="button"
        class="btn small ghost"
        aria-expanded={previewOpen}
        onclick={() => (previewOpen = !previewOpen)}
        data-testid="ide-shell-env-preview-btn"
      >
        <Icon name={previewOpen ? 'eyeOff' : 'eye'} size={12} />
        {previewOpen ? 'Hide shell vars' : 'Inspect shell vars'}
      </button>
      <button
        type="button"
        class="btn small"
        disabled={ide.shellEnvLoading}
        onclick={() => void handleReload()}
        data-testid="ide-reload-shell-env"
      >
        <Icon name="refresh" size={12} />
        {ide.shellEnvLoading ? 'Reloading…' : 'Reload shell env'}
      </button>
      {#if onpasteVars}
        <button
          type="button"
          class="btn small"
          aria-expanded={pasteOpen}
          onclick={() => (pasteOpen = !pasteOpen)}
          data-testid="ide-paste-dotenv-btn"
        >
          <Icon name="file" size={12} />
          Paste .env…
        </button>
      {/if}
    </div>
  </div>

  {#if pasteOpen && onpasteVars}
    <div class="senv-paste-box">
      <label class="senv-paste-lbl" for="senv-paste-input">
        Paste <code>KEY=VALUE</code> or <code>export KEY="VALUE"</code> lines
      </label>
      <textarea
        id="senv-paste-input"
        class="input senv-paste-ta"
        rows="4"
        dir="ltr"
        placeholder={'GO_DEBUGGING_PARAMS="--headless"\nGOPRIVATE=bitbucket.org/myorg/*'}
        bind:value={pasteRaw}
      ></textarea>
      <div class="senv-paste-foot">
        <button type="button" class="btn small ghost" onclick={() => { pasteOpen = false; pasteRaw = ''; }}>Cancel</button>
        <button type="button" class="btn small primary" disabled={!pasteRaw.trim()} onclick={applyPastedBlock}>
          Import variables
        </button>
      </div>
    </div>
  {/if}

  {#if previewOpen}
    <div class="senv-preview" data-testid="ide-shell-env-preview">
      <div class="input-group senv-filter">
        <Icon name="search" size={12} />
        <input
          type="search"
          dir="ltr"
          aria-label="Filter inherited shell variables"
          placeholder="Filter shell variables (e.g. GO_DEBUGGING, GOPRIVATE, PATH)…"
          bind:value={filterQuery}
          data-testid="ide-shell-env-search"
        />
      </div>

      {#if sortedShellEntries.length === 0}
        <p class="senv-empty">
          {ide.shellEnvLoading ? 'Probing login shell environment…' : 'No shell variables match this filter.'}
        </p>
      {:else}
        <div class="senv-table" role="list">
          {#each visibleEntries as [k, v] (k)}
            {@const highlighted = /^GO_DEBUGGING|^GOPRIVATE|^GONOSUMDB/i.test(k)}
            <div class="senv-row" class:is-hi={highlighted} role="listitem">
              <span class="senv-key" dir="ltr">{k}</span>
              {#if highlighted}<Badge tone="accent" label="Go / Debug" />{/if}
              <span class="senv-val" dir="ltr" title={v}>{v || '(empty)'}</span>
            </div>
          {/each}
        </div>
        {#if sortedShellEntries.length > visibleEntries.length}
          <p class="senv-more">Showing {visibleEntries.length} of {sortedShellEntries.length} variables — refine filter to inspect more.</p>
        {/if}
      {/if}
    </div>
  {/if}
</section>

<style>
  .senv-card {
    display: flex; flex-direction: column; gap: 10px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .senv-head { display: flex; flex-wrap: wrap; align-items: flex-start; justify-content: space-between; gap: 10px; }
  .senv-title-row { display: flex; align-items: flex-start; gap: 10px; flex: 1; min-inline-size: 240px; }
  .senv-copy { display: flex; flex-direction: column; gap: 4px; min-inline-size: 0; }
  .senv-headline { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .senv-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .senv-sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }
  .senv-sub code, .senv-paste-lbl code { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); }
  .senv-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .senv-paste-box {
    display: flex; flex-direction: column; gap: 6px; padding: 10px;
    border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border);
  }
  .senv-paste-lbl { font-size: var(--fs-xs); font-weight: 500; color: var(--text); }
  .senv-paste-ta { font-family: var(--font-mono); font-size: var(--fs-xs); resize: vertical; }
  .senv-paste-foot { display: flex; justify-content: flex-end; gap: 6px; }
  .senv-preview {
    display: flex; flex-direction: column; gap: 8px; padding-block-start: 8px;
    border-block-start: 1px solid var(--separator);
  }
  .senv-filter { block-size: 28px; font-size: var(--fs-xs); }
  .senv-table {
    display: flex; flex-direction: column; gap: 2px; max-block-size: 180px; overflow-y: auto;
    padding: 4px; border-radius: var(--radius-s); background: var(--surface); border: 1px solid var(--border);
  }
  .senv-row {
    display: flex; align-items: center; gap: 8px; padding-block: 4px; padding-inline: 6px;
    border-radius: var(--radius-s); font-family: var(--font-mono); font-size: var(--fs-xs); min-inline-size: 0;
  }
  .senv-row.is-hi { background: var(--accent-soft); }
  .senv-key { font-weight: 600; color: var(--text); flex-shrink: 0; }
  .senv-val { flex: 1; min-inline-size: 0; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .senv-empty, .senv-more { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
</style>
