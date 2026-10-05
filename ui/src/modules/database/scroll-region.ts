// `use:scrollRegion={'Statement'}` — a scrollable block a keyboard user can
// reach and scroll (WCAG 2.1.1): role="region", its name, and tabIndex 0.
// Applied at runtime, like lib/paneResizer, because Svelte's a11y linter
// counts a focusable region as a non-interactive element with a tabindex.
export function scrollRegion(node: HTMLElement, label: string) {
  const apply = (name: string): void => {
    node.setAttribute('role', 'region');
    node.setAttribute('aria-label', name);
    node.tabIndex = 0;
  };
  apply(label);
  return { update: apply };
}
