// `use:focusOnMount` — the accessible stand-in for the `autofocus` attribute.
// `autofocus` is applied inconsistently by WebKit inside dialogs (and Svelte
// flags it, a11y_autofocus); this moves focus once the element is in the DOM,
// which also tells a surrounding `dialogFocus` trap that focus is already
// inside. Use it only where moving focus is expected: the first field of a
// dialog or sheet, or an inline editor the user just opened.
//
//   <input use:focusOnMount />            focus
//   <input use:focusOnMount={{ select: true }} />   focus + select the text
export function focusOnMount(node: HTMLElement, opts: { select?: boolean } = {}): void {
  node.focus({ preventScroll: true });
  if (opts.select && (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement)) node.select();
}
