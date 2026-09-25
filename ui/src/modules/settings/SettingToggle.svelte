<script lang="ts">
  // One on/off row for every Settings page: the checkbox on the leading side,
  // then a title and an optional one-line hint — the `.checkbox-row`
  // convention from docs/design/guidelines (layout.md → Settings form). Pages
  // used to mix hand-rolled switches (with an invisible keyboard focus), bare
  // trailing checkboxes and "☑ On" labels; this keeps them one control.
  //
  // It applies immediately: `onchange` gets the new value. When the handler
  // returns a promise the box re-syncs to `checked` afterwards, so a failed
  // save the parent reverted (or never applied) can't leave it showing a
  // state that isn't in effect.
  import type { Snippet } from 'svelte';

  interface Props {
    label: string;
    /** One dim line under the label (or pass `children` for rich text). */
    hint?: string;
    checked: boolean;
    disabled?: boolean;
    /** Tooltip — say WHY when disabled. */
    title?: string;
    onchange: (checked: boolean) => void | Promise<void>;
    testid?: string;
    children?: Snippet;
  }
  let { label, hint, checked, disabled = false, title, onchange, testid, children }: Props = $props();

  async function change(e: Event & { currentTarget: HTMLInputElement }): Promise<void> {
    const el = e.currentTarget;
    const r = onchange(el.checked);
    if (r instanceof Promise) {
      await r;
      el.checked = checked;
    }
  }
</script>

<label class="st" class:disabled {title}>
  <input type="checkbox" {checked} {disabled} onchange={change} data-testid={testid} />
  <span class="st-text">
    <span class="st-label">{label}</span>
    {#if children}
      <span class="st-hint">{@render children()}</span>
    {:else if hint}
      <span class="st-hint">{hint}</span>
    {/if}
  </span>
</label>

<style>
  .st {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 0;
    cursor: pointer;
    user-select: none;
  }
  /* Hairlines between stacked rows are the caller's (scoped as
     `.list > :global(.st + .st)`, see InsightsSettings) — sibling instances
     are separate component scopes, and a global `.st + .st` would leak into
     other modules that use a `.st` class. */
  .st.disabled {
    cursor: default;
  }
  .st input {
    flex-shrink: 0;
    width: 15px;
    height: 15px;
    margin: 1px 0 0;
    accent-color: var(--accent);
    cursor: inherit;
  }
  .st-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .st-label {
    font-size: var(--fs-m);
    color: var(--text);
    line-height: 1.35;
  }
  .st.disabled .st-label {
    color: var(--text-dim);
  }
  .st-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .st-hint :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
</style>
