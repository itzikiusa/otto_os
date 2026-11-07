<script lang="ts" module>
  // One catalog fetch per page load, shared by every picker instance. The
  // /providers/models route may not exist yet (older daemon) — a 404/error
  // resolves to null ONCE and every picker falls back to free-text, silently.
  let catalogPromise: Promise<Record<string, CatalogModel[]> | null> | null = null;

  export interface CatalogModel {
    id: string;
    label?: string | null;
  }

  /** Normalize whatever shape the catalog route returns into
   *  provider → [{id, label}]. Accepts a grouped map (`{claude: [...]}`,
   *  possibly nested under `models`/`providers`), entries as bare id strings
   *  or objects keyed `id`/`model_id`/`model`. Unknown shapes → null. */
  function normalizeCatalog(raw: unknown): Record<string, CatalogModel[]> | null {
    if (raw === null || typeof raw !== 'object') return null;
    let obj = raw as Record<string, unknown>;
    for (const key of ['models', 'providers']) {
      const inner = obj[key];
      if (inner && typeof inner === 'object' && !Array.isArray(inner)) {
        obj = inner as Record<string, unknown>;
        break;
      }
    }
    const out: Record<string, CatalogModel[]> = {};
    for (const [provider, entries] of Object.entries(obj)) {
      if (!Array.isArray(entries)) continue;
      const models: CatalogModel[] = [];
      for (const e of entries) {
        if (typeof e === 'string') {
          if (e.trim() !== '') models.push({ id: e.trim() });
        } else if (e !== null && typeof e === 'object') {
          const r = e as Record<string, unknown>;
          const id = [r.id, r.model_id, r.model].find(
            (v): v is string => typeof v === 'string' && v.trim() !== '',
          );
          if (id) {
            models.push({
              id: id.trim(),
              label: typeof r.label === 'string' && r.label.trim() !== '' ? r.label : null,
            });
          }
        }
      }
      if (models.length > 0) out[provider] = models;
    }
    return Object.keys(out).length > 0 ? out : null;
  }
</script>

<script lang="ts">
  // Per-session model control: catalog dropdown (when the daemon's model
  // catalog route is up) + always-available free-text, with "Provider default"
  // (= empty) as the baseline. Renders NOTHING for a provider whose spec has
  // no model-flag template (`/meta.model_flags`) — a model the CLI would drop
  // is never offered.
  import { api } from '../api/client';
  import { providerSupportsModel } from '../providers';

  interface Props {
    provider: string;
    value: string;
    onchange: (model: string) => void;
    /** The line under the control (what the pin applies to). */
    hint?: string;
    /** DOM id of the control — unique when several pickers share a page. */
    id?: string;
    /** Table-cell use: the label is visually hidden (still announced) and there is no hint. */
    compact?: boolean;
    /** Models to offer besides the catalog route's (e.g. the known aliases),
     *  so a model chosen elsewhere never reads as "Custom…". */
    extra?: { id: string; label: string }[];
  }
  let {
    provider,
    value,
    onchange,
    hint = "Pins the model for this session only (passed as the CLI’s model flag).",
    id = 'model-picker',
    compact = false,
    extra = [],
  }: Props = $props();

  const CUSTOM = '\u0000custom'; // sentinel select value, never a real model id
  const supported = $derived(providerSupportsModel(provider));

  let catalog = $state<Record<string, CatalogModel[]> | null>(null);
  // Whether the user explicitly chose "Custom…" (keeps the text input open
  // even while it's empty).
  let customMode = $state(false);

  $effect(() => {
    catalogPromise ??= api
      .get<unknown>('/providers/models')
      .then(normalizeCatalog)
      .catch(() => null);
    void catalogPromise.then((c) => (catalog = c));
  });

  // The catalog route plus `extra` (the caller's known models).
  const models = $derived.by((): CatalogModel[] => {
    const out: CatalogModel[] = [...(catalog?.[provider] ?? [])];
    for (const m of extra) {
      if (!out.some((x) => x.id === m.id)) out.push({ id: m.id, label: m.label });
    }
    return out;
  });
  const inCatalog = $derived(value !== '' && models.some((m) => m.id === value));
  // A saved value not in the catalog behaves as custom, so it's never dropped.
  const selectValue = $derived(
    customMode || (value !== '' && !inCatalog) ? CUSTOM : value,
  );
  const showText = $derived(models.length === 0 || selectValue === CUSTOM);

  function onSelect(e: Event): void {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (v === CUSTOM) {
      customMode = true;
      onchange('');
    } else {
      customMode = false;
      onchange(v);
    }
  }
</script>

{#if supported}
  <!-- `.field` gives form stacks their 12px rhythm; inline (compact) in a
       toolbar row that margin only pushed the picker off the row's baseline. -->
  <div class="field" class:mp-inline={compact}>
    <label for={id} class:sr-only={compact}>Model <span class="dim">(optional)</span></label>
    <div class="controls">
      {#if models.length > 0}
        <select {id} class="select" value={selectValue} onchange={onSelect}>
          <option value="">Provider default</option>
          {#each models as m (m.id)}
            <option value={m.id}>{m.label ? `${m.label} — ${m.id}` : m.id}</option>
          {/each}
          <option value={CUSTOM}>Custom…</option>
        </select>
      {/if}
      {#if showText}
        <input dir="ltr"
          id={models.length === 0 ? id : undefined}
          aria-label={models.length > 0 ? 'Custom model id' : undefined}
          class="input mono"
          value={value}
          oninput={(e) => onchange(e.currentTarget.value)}
          placeholder="Provider default"
          spellcheck="false"
        />
      {/if}
    </div>
    {#if !compact}<span class="hint">{hint}</span>{/if}
  </div>
{/if}

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
  }
  .controls {
    display: flex;
    gap: 6px;
  }
  .select,
  .input {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    padding: 6px 8px;
    font-size: var(--fs-s);
    color: var(--text);
    min-width: 0;
  }
  .select {
    flex: 1;
  }
  .input {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }
  /* A model id is code; the "Provider default" placeholder is prose. */
  .mono:placeholder-shown {
    font-family: var(--font-ui);
  }
  .select:focus,
  .input:focus {
    outline: none;
    border-color: var(--accent-text); box-shadow: 0 0 0 3px var(--accent-soft-strong)
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .mp-inline {
    margin-bottom: 0;
  }
</style>
