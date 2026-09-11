<script lang="ts">
  // Inline typed editor for ONE leaf value in the Vertical view: a kind picker
  // (pre-selected from the stored value) + the text input, validated per kind
  // on commit. Enter commits, Escape cancels, focus leaving the editor commits
  // (like the grid's cell input). The engine narrows the kinds: SQL columns
  // have no BSON types to offer.
  import { tick, untrack } from 'svelte';
  import type { DbEngine } from '../../lib/api/types';
  import type { TypedKind, TypedValue } from './edit-types';

  interface Props {
    kind: TypedKind;
    raw: string;
    engine: DbEngine | null;
    onsave: (tv: TypedValue) => void;
    oncancel: () => void;
  }
  let { kind: initialKind, raw: initialRaw, engine, onsave, oncancel }: Props = $props();

  const MONGO_KINDS: TypedKind[] = ['string', 'number', 'bool', 'null', 'objectId', 'date', 'long', 'decimal', 'json'];
  const SQL_KINDS: TypedKind[] = ['string', 'number', 'bool', 'null', 'json'];
  const kinds = $derived(engine === 'mongodb' ? MONGO_KINDS : SQL_KINDS);
  const LABEL: Record<TypedKind, string> = {
    string: 'string',
    number: 'number',
    bool: 'boolean',
    null: 'null',
    objectId: 'ObjectId',
    date: 'Date',
    long: 'Long',
    decimal: 'Decimal128',
    json: 'JSON',
  };

  // Seeded once from the props (the editor is mounted per edit, so they never
  // change underneath it). A BSON kind the engine can't take (a SQL column
  // holding an EJSON-shaped object) degrades to raw JSON of the same text.
  function seed(): { kind: TypedKind; raw: string } {
    const allowed = engine === 'mongodb' ? MONGO_KINDS : SQL_KINDS;
    return { kind: allowed.includes(initialKind) ? initialKind : 'json', raw: initialRaw };
  }
  const init = seed();
  let kind = $state<TypedKind>(init.kind);
  let raw = $state(init.raw);
  let err = $state<string | null>(null);
  let root = $state<HTMLElement | null>(null);

  /** Validate + normalise per kind; null when the text doesn't fit the kind. */
  function validate(): TypedValue | null {
    const t = raw.trim();
    switch (kind) {
      case 'null':
        return { kind, raw: '' };
      case 'bool':
        if (t !== 'true' && t !== 'false') return fail('true or false');
        return { kind, raw: t };
      case 'number': {
        if (t === '' || !Number.isFinite(Number(t))) return fail('a finite number');
        return { kind, raw: t };
      }
      case 'objectId':
        if (!/^[a-f0-9]{24}$/i.test(t)) return fail('24 hex characters');
        return { kind, raw: t.toLowerCase() };
      case 'date': {
        const ms = Date.parse(t);
        if (!Number.isFinite(ms)) return fail('a parseable date (ISO 8601)');
        return { kind, raw: new Date(ms).toISOString() };
      }
      case 'long':
        if (!/^-?\d+$/.test(t)) return fail('an integer');
        return { kind, raw: t };
      case 'decimal':
        if (!/^-?\d+(\.\d+)?([eE][-+]?\d+)?$/.test(t)) return fail('a decimal number');
        return { kind, raw: t };
      case 'json': {
        try {
          const v: unknown = JSON.parse(t);
          if (v === null || typeof v !== 'object') return fail('a JSON object or array');
          return { kind, raw: JSON.stringify(v) };
        } catch {
          return fail('valid JSON');
        }
      }
      default:
        return { kind: 'string', raw };
    }
  }
  function fail(want: string): null {
    err = `Expected ${want}`;
    return null;
  }

  // Closing tears the editor down; a focusout raised by that teardown must
  // not commit a second time.
  let done = false;
  function finish(tv: TypedValue | null): void {
    if (done) return;
    done = true;
    if (tv) onsave(tv);
    else oncancel();
  }
  function save(): void {
    const tv = validate();
    if (tv) finish(tv);
  }
  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      save();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      finish(null);
    }
  }
  // Commit when focus leaves the whole editor (kind select + input), not when
  // it merely moves between the two. A draft that doesn't validate is dropped
  // (the row goes back to its stored value) rather than trapping focus.
  function onFocusOut(e: FocusEvent): void {
    const to = e.relatedTarget as Node | null;
    if (to && root?.contains(to)) return;
    finish(validate());
  }

  function focusInput(node: HTMLInputElement): void {
    void tick().then(() => {
      node.focus();
      node.select();
    });
  }
  // Switching kind clears a stale error; a boolean needs one of its two values.
  $effect(() => {
    void kind;
    err = null;
    if (kind === 'bool') untrack(() => { if (raw !== 'true' && raw !== 'false') raw = 'false'; });
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<span class="ve" bind:this={root} onkeydown={onKeydown} onfocusout={onFocusOut}>
  <select class="ve-kind mono" bind:value={kind} aria-label="Value type" title="Value type">
    {#each kinds as k (k)}
      <option value={k}>{LABEL[k]}</option>
    {/each}
  </select>
  {#if kind === 'bool'}
    <select class="ve-kind mono" bind:value={raw} aria-label="Boolean value">
      <option value="true">true</option>
      <option value="false">false</option>
    </select>
  {:else if kind !== 'null'}
    <!-- svelte-ignore a11y_autofocus -->
    <input
      class="ve-input mono"
      class:bad={err !== null}
      bind:value={raw}
      use:focusInput
      spellcheck="false"
      autocomplete="off"
      aria-label="Value"
      placeholder={kind === 'date' ? '2026-01-31T00:00:00Z' : kind === 'objectId' ? '24 hex chars' : ''}
    />
  {/if}
  {#if err}<span class="ve-err">{err}</span>{/if}
</span>

<style>
  .ve {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    max-width: 100%;
    flex-wrap: wrap;
  }
  .ve-kind {
    height: 20px;
    padding: 0 4px;
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .ve-input {
    flex: 1;
    min-width: 120px;
    height: 20px;
    padding: 0 6px;
    font-size: 11.5px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    outline: none;
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .ve-input.bad {
    box-shadow: inset 0 0 0 1px var(--status-exited);
  }
  .ve-err {
    font-size: 10.5px;
    color: var(--status-exited);
  }
</style>
