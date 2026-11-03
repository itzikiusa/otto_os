<script lang="ts">
  // "Add row" form: one field per column of the target table, derived from its
  // schema (InsertFlow.open → insert-form.ts), so an EMPTY table can be
  // filled without hand-writing an INSERT. Each field is a Value / NULL /
  // Default choice — Default leaves the column out so the engine applies its
  // default or auto-increment. "Review INSERT…" builds the statement and opens
  // the normal review modal; nothing runs from here. Mounted by InsertRowHost
  // while `flow.insert.form` is set, inside the shared Modal.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import { fieldBadges, fieldError, fieldKind, type InsertField, type InsertMode } from './insert-form';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();

  const form = $derived(flow.insert.form);
  const MODES: { mode: InsertMode; label: string; title: string }[] = [
    { mode: 'value', label: 'Value', title: 'Insert the typed value' },
    { mode: 'null', label: 'NULL', title: 'Insert an explicit NULL' },
    { mode: 'default', label: 'Default', title: 'Leave the column out — the database applies its default or auto value' },
  ];

  let touched = $state(false);
  function setMode(f: InsertField, mode: InsertMode): void {
    if (f.generated) return;
    f.mode = mode;
    touched = true;
    if (form) form.err = null;
  }
  function setRaw(f: InsertField, raw: string): void {
    f.raw = raw;
    f.mode = 'value';
    touched = true;
    if (form) form.err = null;
  }

  let closing = false;
  async function close(): Promise<void> {
    if (closing) return;
    if (touched) {
      closing = true;
      const discard = await confirmer.ask('Discard the values you entered?', {
        title: 'Discard new row',
        confirmLabel: 'Discard',
        danger: true,
      });
      closing = false;
      if (!discard) return;
    }
    flow.insert.close();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      flow.insert.submit();
    }
  }
  const target = $derived(form ? (form.db ? `${form.db}.${form.table}` : form.table) : '');
</script>

{#if form}
  <Modal title="Add row to {form.table}" width={680} onclose={close}>
    <div class="ir" role="presentation" onkeydown={onKeydown}>
      <p class="ir-hint">
        Fill in the new row for <span class="mono">{target}</span>. <strong>Default</strong> leaves a column out so the
        database fills it. You review the INSERT before it runs.
      </p>
      <LoadState
        what="the table’s columns"
        loading={form.loading}
        error={form.loadError}
        empty={form.fields.length === 0}
        onretry={form.db ? () => void flow.insert.open() : undefined}
        rows={3}
      >
        {#if form.partial}
          <p class="ir-note"><Icon name="info" size={12} />Column types and defaults couldn’t be loaded — only columns you fill in are inserted.</p>
        {/if}
        <div class="ir-fields">
          {#each form.fields as f (f.name)}
            {@const err = fieldError(f)}
            {@const kind = fieldKind(f.dataType)}
            {@const badges = fieldBadges(f)}
            <div class="ir-row" class:locked={f.generated}>
              <div class="ir-label">
                <span class="ir-name mono" title={f.name}>{f.name}</span>
                <span class="ir-meta">
                  {#if f.dataType}<span class="ir-type mono">{f.dataType}</span>{/if}
                  {#each badges as b (b)}<span class="ir-badge" class:req={b === 'required'}>{b}</span>{/each}
                </span>
              </div>
              <div class="ir-input">
                {#if f.generated}
                  <span class="ir-gen">Computed by the database</span>
                {:else if kind === 'bool'}
                  <select
                    class="input ir-value"
                    aria-label={`Value for ${f.name}`}
                    value={f.mode === 'value' ? f.raw : ''}
                    onchange={(e) => setRaw(f, e.currentTarget.value)}
                  >
                    <option value="" disabled>{f.mode === 'null' ? 'NULL' : f.mode === 'default' ? 'Default' : 'Choose…'}</option>
                    <option value="true">true</option>
                    <option value="false">false</option>
                  </select>
                {:else}
                  <input
                    dir="auto"
                    class="input mono ir-value"
                    class:dim={f.mode !== 'value'}
                    type="text"
                    inputmode={kind === 'number' ? 'decimal' : undefined}
                    aria-label={`Value for ${f.name}`}
                    aria-invalid={!!err}
                    placeholder={f.mode === 'null' ? 'NULL' : f.mode === 'default' ? (f.auto ? 'auto' : f.defaultExpr ?? 'default') : ''}
                    value={f.mode === 'value' ? f.raw : ''}
                    oninput={(e) => setRaw(f, e.currentTarget.value)}
                    spellcheck="false"
                    autocomplete="off"
                  />
                {/if}
                {#if !f.generated}
                  <div class="ir-modes" role="group" aria-label={`How to set ${f.name}`}>
                    {#each MODES as m (m.mode)}
                      <button
                        type="button"
                        class="ir-mode"
                        class:on={f.mode === m.mode}
                        aria-pressed={f.mode === m.mode}
                        disabled={m.mode === 'null' && !f.nullable}
                        title={m.mode === 'null' && !f.nullable ? 'NOT NULL column' : m.title}
                        onclick={() => setMode(f, m.mode)}
                      >{m.label}</button>
                    {/each}
                  </div>
                {/if}
              </div>
              {#if err && touched}<span class="ir-err">{err}</span>{/if}
            </div>
          {/each}
        </div>
      </LoadState>
      <div class="ir-form-err" class:shown={!!form.err} role="alert">
        {#if form.err}<Icon name="warning" size={12} /><span>{form.err}</span>{/if}
      </div>
    </div>
    {#snippet footer()}
      <span class="ir-kbd mono">⌘↵ to review · Esc to cancel</span>
      <span class="grow"></span>
      <button class="btn" onclick={close}>Cancel</button>
      <button class="btn primary" onclick={() => flow.insert.submit()} disabled={form.loading || !!form.loadError}>
        <Icon name="eye" size={12} />Review INSERT…
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ir {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-height: 0;
  }
  .ir-hint,
  .ir-note {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .ir-note {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .ir-fields {
    display: flex;
    flex-direction: column;
    max-height: 56vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .ir-row {
    display: grid;
    grid-template-columns: minmax(140px, 0.9fr) minmax(0, 1.6fr);
    align-items: center;
    gap: 4px 12px;
    padding: 6px 10px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  }
  .ir-row:last-child {
    border-bottom: none;
  }
  .ir-row.locked {
    opacity: var(--disabled-opacity);
  }
  .ir-label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ir-name {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ir-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    align-items: center;
  }
  .ir-type {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ir-badge {
    font-size: var(--fs-xs);
    padding: 0 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ir-badge.req {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .ir-input {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .ir-value {
    flex: 1;
    min-width: 0;
  }
  .ir-value.dim::placeholder {
    font-style: italic;
  }
  .ir-gen {
    flex: 1;
    font-size: var(--fs-s);
    font-style: italic;
    color: var(--text-dim);
  }
  .ir-modes {
    display: inline-flex;
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .ir-mode {
    padding: 2px 6px;
    font-size: var(--fs-xs);
    background: transparent;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .ir-mode + .ir-mode {
    border-inline-start: 1px solid var(--border);
  }
  .ir-mode.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ir-mode:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .ir-mode:focus-visible {
    outline: 2px solid var(--accent-line-strong);
    outline-offset: -2px;
  }
  .ir-err {
    grid-column: 2;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .ir-form-err.shown {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .ir-kbd {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .ir-row {
      grid-template-columns: minmax(0, 1fr);
    }
    .ir-err {
      grid-column: 1;
    }
  }
</style>
