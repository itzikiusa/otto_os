// The one HTML escaper. Every renderer that builds markup from strings (code
// highlighting, markdown, log lines, report HTML, SVG text) goes through it, so
// whether a given output is safe inside an ATTRIBUTE no longer depends on which
// hand-written copy it happened to use: all five significant characters are
// escaped, quotes included. `ui-guards` flags a hand-written
// `.replace(/&/g, '&amp;')` chain anywhere else.

const ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

/** Escape `& < > " '` — safe in element text and in a quoted attribute. */
export function escapeHtml(s: string): string {
  return String(s).replace(/[&<>"']/g, (c) => ESCAPES[c]);
}
