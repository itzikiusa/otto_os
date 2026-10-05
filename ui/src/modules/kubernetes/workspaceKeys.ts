// Scope + opt-out for the Cluster workspace's k9s-style single-key shortcuts
// (a11y P3, WCAG 2.1.4 Character Key Shortcuts). They used to act window-wide:
// a letter typed while focus sat in the sidebar, the right panel or a session
// pane opened logs or a shell in the workspace. Now they act only while focus
// is inside the workspace (or on nothing at all — the page body after a click
// on empty space), and the letters can be switched off per device.

/** Per-device localStorage key for the on/off switch ('0' = off). */
export const SINGLE_KEYS_PREF = 'otto_k8s_single_keys';

/** Keys that are plain characters (the letters and `/` `?`), as opposed to
 *  Enter / Escape, which every list honours and which stay on. */
export const SINGLE_KEYS = new Set(['/', '?', 'n', 'r', 'l', 's', 'd', 'y', 'j', 'k']);

/** Does a keydown with this target belong to the workspace rooted at `root`? */
export function inWorkspace(target: EventTarget | null, root: Element | null | undefined, doc: Document | null = typeof document === 'undefined' ? null : document): boolean {
  if (!root) return false;
  if (doc && (target === doc.body || target === doc.documentElement || target === doc)) return true;
  return target instanceof Node && root.contains(target);
}
