<script lang="ts">
  // Import connections from another DB tool. No file picker, no path config: the
  // daemon reads each tool's config from its own standard location. Three steps —
  //   1. pick a tool (the daemon already told us which are present + how many),
  //   2. scan it and preview the parsed connections (supported rows checked,
  //      unsupported rows greyed with the reason),
  //   3. create the kept ones (passwords are never imported — the user sets them
  //      after, exactly like a hand-made connection).
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { importSources, importScan, importCreate } from '../../lib/api/client';
  import type {
    ConnectionKind,
    ImportSource,
    SourceStatus,
    ParsedConnection,
    ImportCreateItem,
  } from '../../lib/api/types';

  interface Props {
    /** Workspace the import endpoints are scoped to (same one the page uses). */
    wsId: string;
    onclose: () => void;
    /** Called after a successful create so the page can refresh its list. */
    onimported: () => void;
  }
  let { wsId, onclose, onimported }: Props = $props();

  // Same kind→icon mapping the connection list rows use, so a row reads the same.
  const kindIcons: Record<ConnectionKind, string> = {
    ssh: 'key',
    mysql: 'db',
    redis: 'zap',
    mongodb: 'db',
    clickhouse: 'db',
    custom: 'terminal',
  };

  type Step = 'pick' | 'preview';
  let step = $state<Step>('pick');

  // Step 1 — tool list.
  let sources = $state<SourceStatus[]>([]);
  let loadingSources = $state(true);

  // Step 2 — scan of one tool.
  let activeSource = $state<ImportSource | null>(null);
  let scanning = $state(false);
  let scanPath = $state<string | null>(null);
  let warnings = $state<string[]>([]);
  let rows = $state<ParsedConnection[]>([]);
  // Per-row keep flag (index-aligned with `rows`). Unsupported rows are never set.
  let keep = $state<boolean[]>([]);

  // Step 3 — create in flight.
  let creating = $state(false);

  const activeLabel = $derived(
    sources.find((s) => s.source === activeSource)?.label ?? activeSource ?? '',
  );
  const selectableCount = $derived(rows.filter((r) => r.supported).length);
  const selectedCount = $derived(keep.filter(Boolean).length);
  const allSelected = $derived(selectableCount > 0 && selectedCount === selectableCount);

  $effect(() => {
    void loadSources();
  });

  async function loadSources(): Promise<void> {
    loadingSources = true;
    try {
      sources = await importSources(wsId);
    } catch (e) {
      toasts.error('Could not read tools', e instanceof Error ? e.message : String(e));
      sources = [];
    } finally {
      loadingSources = false;
    }
  }

  async function pick(s: SourceStatus): Promise<void> {
    if (!s.present || scanning) return;
    activeSource = s.source;
    step = 'preview';
    scanning = true;
    rows = [];
    keep = [];
    warnings = [];
    scanPath = s.path ?? null;
    try {
      const res = await importScan(wsId, s.source);
      rows = res.connections;
      scanPath = res.path ?? s.path ?? null;
      warnings = res.warnings;
      // Supported rows are kept by default; unsupported can never be selected.
      keep = res.connections.map((c) => c.supported);
    } catch (e) {
      toasts.error('Scan failed', e instanceof Error ? e.message : String(e));
      // Drop back to the picker so the user can retry / choose another tool.
      step = 'pick';
      activeSource = null;
    } finally {
      scanning = false;
    }
  }

  function backToPick(): void {
    if (creating) return;
    step = 'pick';
    activeSource = null;
    rows = [];
    keep = [];
    warnings = [];
    scanPath = null;
  }

  function toggleRow(i: number): void {
    if (!rows[i]?.supported) return;
    keep[i] = !keep[i];
  }

  function selectAll(on: boolean): void {
    keep = rows.map((c, i) => (c.supported ? on : (keep[i] ?? false)));
  }

  // Best-effort one-line summary of a parsed connection's params (host / db),
  // falling back to the name when nothing obvious is there.
  function summarize(c: ParsedConnection): string {
    const p = (c.params ?? {}) as Record<string, unknown>;
    if (c.kind === 'mongodb') return String(p.connection_string ?? '');
    if (c.kind === 'custom') return String(p.template ?? '');
    const host = p.host !== undefined && p.host !== '' ? String(p.host) : '';
    if (!host) return '';
    const port = p.port !== undefined && p.port !== '' ? `:${p.port}` : '';
    const db = p.db !== undefined && p.db !== '' ? ` / ${p.db}` : '';
    const user = p.user !== undefined && p.user !== '' ? `${p.user}@` : '';
    return `${user}${host}${port}${db}`;
  }

  async function create(): Promise<void> {
    if (creating) return;
    const picked: ImportCreateItem[] = rows
      .filter((c, i) => c.supported && keep[i] && c.kind)
      .map((c) => ({ name: c.name, kind: c.kind as ConnectionKind, params: c.params }));
    if (picked.length === 0) {
      toasts.info('Nothing selected', 'Pick at least one connection to import.');
      return;
    }
    creating = true;
    try {
      const res = await importCreate(wsId, { connections: picked, section_id: null });
      const made = res.created.length;
      const failed = res.failed.length;
      if (made > 0) {
        toasts.success(
          `Imported ${made}`,
          failed > 0
            ? `${failed} failed — set passwords on imported connections before connecting.`
            : 'Set passwords on imported connections before connecting.',
        );
      }
      if (failed > 0) {
        // Surface the individual failures so they aren't silently lost.
        const detail = res.failed.map((f) => `${f.name}: ${f.error}`).join('\n');
        toasts.error(`${failed} could not be imported`, detail);
      }
      if (made === 0 && failed === 0) {
        toasts.info('Nothing imported');
      }
      onimported();
      onclose();
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  function close(): void {
    if (creating) return;
    onclose();
  }
</script>

<Modal title="Import connections" width={560} onclose={close}>
  {#if step === 'pick'}
    <div class="imp">
      <p class="imp-hint">
        Otto reads each tool's config from its standard location on this machine — no
        file to pick. Choose a tool to see the connections it has. Passwords are never
        imported; you set those after.
      </p>

      {#if loadingSources}
        <div class="imp-loading" role="status" aria-live="polite">
          <div class="imp-bar"><div class="imp-bar-fill"></div></div>
          <span class="imp-loading-text">Looking for installed tools…</span>
        </div>
      {:else}
        <div class="tool-grid">
          {#each sources as s (s.source)}
            <button
              class="tool"
              class:present={s.present}
              disabled={!s.present}
              onclick={() => void pick(s)}
              aria-label={s.present
                ? `${s.label} — ${s.count ?? 0} connections found`
                : `${s.label} — not found`}
            >
              <span class="tool-icon"><Icon name="plug" size={16} /></span>
              <span class="tool-body">
                <span class="tool-name">{s.label}</span>
                <span class="tool-sub" class:found={s.present}>
                  {#if s.present}
                    {s.count ?? 0} connection{(s.count ?? 0) === 1 ? '' : 's'} found
                  {:else}
                    Not found
                  {/if}
                </span>
              </span>
              {#if s.present}
                <span class="tool-caret"><Icon name="chevronRight" size={13} /></span>
              {/if}
            </button>
          {/each}
        </div>
        {#if sources.every((s) => !s.present)}
          <p class="imp-empty">
            None of the supported tools were found on this machine. Install one (and add a
            connection in it) to import from here.
          </p>
        {/if}
      {/if}
    </div>
  {:else}
    <div class="imp">
      <div class="prev-head">
        <button class="btn small" onclick={backToPick} disabled={creating}>
          <Icon name="chevronRight" size={11} /> Back
        </button>
        <span class="prev-title">{activeLabel}</span>
        {#if scanPath}
          <span class="prev-path mono ellipsis" title={scanPath}>read from {scanPath}</span>
        {/if}
      </div>

      {#if scanning}
        <div class="imp-loading" role="status" aria-live="polite">
          <div class="imp-bar"><div class="imp-bar-fill"></div></div>
          <span class="imp-loading-text">Scanning {activeLabel}…</span>
        </div>
      {:else}
        {#if warnings.length > 0}
          <div class="prev-warn">
            {#each warnings as w (w)}
              <div class="prev-warn-line"><Icon name="info" size={11} /> {w}</div>
            {/each}
          </div>
        {/if}

        {#if rows.length === 0}
          <p class="imp-empty">No connections were found in {activeLabel}.</p>
        {:else}
          <div class="prev-toolbar">
            <span class="prev-count">
              {selectedCount} of {selectableCount} selected
            </span>
            <span class="grow"></span>
            <button
              class="link-btn"
              disabled={selectableCount === 0 || allSelected}
              onclick={() => selectAll(true)}
            >
              Select all
            </button>
            <span class="dot-sep">·</span>
            <button
              class="link-btn"
              disabled={selectedCount === 0}
              onclick={() => selectAll(false)}
            >
              None
            </button>
          </div>

          <div class="rows" role="list">
            {#each rows as c, i (c.name + i)}
              <label class="row" class:disabled={!c.supported} role="listitem">
                <input
                  type="checkbox"
                  class="row-check"
                  checked={keep[i] ?? false}
                  disabled={!c.supported}
                  onchange={() => toggleRow(i)}
                  aria-label={`Import ${c.name}`}
                />
                <span class="row-kind" title={c.kind ?? 'unsupported'}>
                  <Icon name={c.kind ? kindIcons[c.kind] : 'plug'} size={13} />
                </span>
                <span class="row-main">
                  <span class="row-name ellipsis">{c.name}</span>
                  {#if summarize(c)}
                    <span class="row-desc mono ellipsis">{summarize(c)}</span>
                  {/if}
                </span>
                <span class="grow"></span>
                {#if c.supported}
                  {#if c.kind}<span class="kind-badge">{c.kind}</span>{/if}
                  {#if c.needs_password}
                    <span class="pill warn" title="No password was imported — set it before connecting">
                      needs password
                    </span>
                  {/if}
                {:else}
                  <span class="pill skip" title={c.note ?? 'Not supported'}>
                    {c.note ?? 'Not supported'}
                  </span>
                {/if}
              </label>
            {/each}
          </div>

          <p class="imp-note">
            <Icon name="key" size={11} /> Passwords are never imported — set them on each
            connection before connecting.
          </p>
        {/if}
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    {#if step === 'pick'}
      <button class="btn" onclick={close}>Cancel</button>
    {:else}
      <button class="btn" onclick={close} disabled={creating}>Cancel</button>
      <button
        class="btn primary"
        onclick={() => void create()}
        disabled={creating || scanning || selectedCount === 0}
      >
        {creating ? 'Importing…' : `Import ${selectedCount} selected`}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .imp {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .imp-hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .imp-empty {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
    line-height: 1.5;
  }

  /* Step 1 — tool tiles. */
  .tool-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    text-align: start;
    transition:
      border-color 120ms ease-out,
      background 120ms ease-out;
  }
  .tool:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .tool.present:hover {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, var(--surface-2));
  }
  .tool-icon {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .tool-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .tool-name {
    font-size: 13px;
    font-weight: 600;
  }
  .tool-sub {
    font-size: 11px;
    color: var(--text-dim);
  }
  .tool-sub.found {
    color: var(--accent);
  }
  .tool-caret {
    color: var(--text-dim);
    flex-shrink: 0;
  }

  /* Loading bar (shared by sources + scan). */
  .imp-loading {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 0;
  }
  .imp-loading-text {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .imp-bar {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .imp-bar-fill {
    height: 100%;
    width: 40%;
    border-radius: 999px;
    background: var(--accent);
    animation: imp-indet 1.1s ease-in-out infinite;
  }
  @keyframes imp-indet {
    0% {
      margin-left: -40%;
    }
    100% {
      margin-left: 100%;
    }
  }

  /* Step 2 — preview header. */
  .prev-head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  /* The Back button reuses chevronRight rotated to point left. */
  .prev-head .btn.small :global(svg) {
    transform: scaleX(-1);
  }
  .prev-title {
    font-size: 13px;
    font-weight: 600;
  }
  .prev-path {
    font-size: 11px;
    color: var(--text-dim);
    min-width: 0;
    flex: 0 1 auto;
  }
  .prev-warn {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, #b8860b 10%, transparent);
  }
  .prev-warn-line {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11.5px;
    color: #b8860b;
    line-height: 1.4;
  }
  .prev-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .prev-count {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .dot-sep {
    color: var(--text-dim);
    font-size: 11px;
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 11.5px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: 3px;
  }
  .link-btn:hover:not(:disabled) {
    text-decoration: underline;
  }
  .link-btn:disabled {
    color: var(--text-dim);
    opacity: 0.6;
    cursor: default;
  }

  /* Step 2 — rows. */
  .rows {
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 46vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    cursor: pointer;
  }
  .row:hover:not(.disabled) {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .row.disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }
  .row-check {
    flex-shrink: 0;
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
  }
  .row-kind {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .row.disabled .row-kind {
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .row-main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .row-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .row-desc {
    font-size: 11px;
    color: var(--text-dim);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind-badge {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .pill {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .pill.warn {
    color: #b8860b;
    background: color-mix(in srgb, #b8860b 14%, transparent);
  }
  .pill.skip {
    color: var(--text-dim);
    background: var(--surface-2);
  }
  .imp-note {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }

  @media (max-width: 640px) {
    .tool-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
