<script lang="ts">
  // Placeholders detected in the active workbench file — the SAME syntax as the
  // DB Explorer's variables / Run on… (`:name` in SQL, `{name}`, `{{name}}`).
  // Values live with the file (the parent persists them) and feed Send to…:
  // "Copy with placeholders filled", the API client / session / vault targets,
  // and the DB Run on… sheet, where a list (`1,2,3,4`) becomes one run each.
  import { detectPlaceholders, isSqlLike } from './lib/placeholders';

  interface Props {
    content: string;
    lang: string;
    values: Record<string, string>;
    onchange: (values: Record<string, string>) => void;
  }
  let { content, lang, values, onchange }: Props = $props();

  const found = $derived(detectPlaceholders(content, lang));
  const sql = $derived(isSqlLike(lang));

  /** The Run on… sheet's list rule: newline-separated if multi-line, else
   *  comma-separated (mirrors database/multi-run.ts `parseValues`). */
  function parseListValue(text: string): string[] {
    const parts = text.includes('\n') ? text.split('\n') : text.split(',');
    return parts.map((v) => v.trim()).filter((v) => v.length > 0);
  }

  function set(name: string, value: string): void {
    onchange({ ...values, [name]: value });
  }
</script>

<section class="ph" data-testid="wb-placeholders" aria-label="Placeholders">
  {#if found.length === 0}
    <p class="ph-empty">
      No placeholders — use
      {#if sql}<code>:name</code>,{/if}
      <code>{'{name}'}</code> or <code>{'{{name}}'}</code>.
    </p>
  {:else}
    <p class="ph-hint">
      Values fill in on Send to… and Copy. {#if sql}A list like <code>1,2,3,4</code> runs once per value in Database — Run on….{:else}Empty ones stay as-is.{/if}
    </p>
    <ul class="ph-list">
      {#each found as p (p.name)}
        {@const v = values[p.name] ?? ''}
        {@const list = parseListValue(v)}
        <li class="ph-row">
          <div class="ph-head">
            <span class="ph-name mono" title={p.name}>{p.name}</span>
            <span class="ph-badge mono" title="Placeholder syntax">{p.syntax}</span>
            {#if p.count > 1}<span class="ph-count" title="Occurrences">×{p.count}</span>{/if}
          </div>
          <input
            class="input ph-input mono"
            type="text"
            value={v}
            placeholder={sql ? 'value, or 1,2,3 to sweep' : 'value'}
            spellcheck="false"
            aria-label="Value for {p.name}"
            oninput={(e) => set(p.name, (e.currentTarget as HTMLInputElement).value)}
          />
          {#if list.length > 1}
            <span class="ph-sweep" title="Database — Run on… runs once per value">
              {list.length} values · sweep in Run on…
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .ph {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    min-inline-size: 0;
  }
  .ph-empty,
  .ph-hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    line-height: 1.45;
  }
  code,
  .mono {
    font-family: var(--font-mono);
  }
  .ph-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ph-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .ph-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }
  .ph-name {
    font-size: var(--fs-s);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ph-badge {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: var(--radius-s);
    padding-inline: 5px;
  }
  .ph-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ph-input {
    inline-size: 100%;
    font-size: var(--fs-s);
  }
  .ph-sweep {
    font-size: var(--fs-xs);
    color: var(--info);
  }
</style>
