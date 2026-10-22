// Inline-edit focus contract (accessibility-responsive-4-03). An inline
// editor REPLACES its Edit trigger (`{#if editing}<editor>{:else}<Edit>{/if}`),
// so without help focus falls to <body> on both edges and a keyboard or
// screen-reader user has to Tab from the top of the page to find the field.
//
// - Open: the editor's container gets `use:focusFirstField` (or its one field
//   `use:focusOnMount`), which focuses the first input once it is mounted.
// - Close (Cancel, Escape or a successful save): `refocusEditTrigger(key)`
//   waits a tick for the trigger to re-render and focuses the button marked
//   `data-edit-for={key}`.
// - Name each trigger after its field: `aria-label="Edit {field name}"`.
import { tick } from 'svelte';

const FIELD = 'input:not([type="hidden"]), textarea, select, [contenteditable="true"]';

/** Svelte action: focus the first form field inside `node` (else `node`). */
export function focusFirstField(node: HTMLElement): void {
  const field = node.querySelector<HTMLElement>(FIELD);
  (field ?? node).focus({ preventScroll: true });
}

/** After an inline editor closes, put focus back on its Edit trigger. */
export async function refocusEditTrigger(key: string, root?: ParentNode): Promise<void> {
  await tick();
  const scope = root ?? (typeof document === 'undefined' ? null : document);
  if (!scope) return;
  const sel = `[data-edit-for="${typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(key) : key}"]`;
  scope.querySelector<HTMLElement>(sel)?.focus({ preventScroll: true });
}
