// Help → guide markdown → sanitized HTML for {@html}.
//
// The app's GFM renderer (marked + the allowlist sanitizer, lib/md.ts) does the
// parsing and the scrubbing; this pass only reshapes the SAFE output:
//   • key combos become <kbd> chips — every code span in the first column of a
//     "Keys" table, and any short code span in prose that starts with a
//     modifier (`⌘K` → [⌘][K]);
//   • tables get a scroll wrapper (no horizontal page scroll on a phone);
//   • external links open outside the app; in-app `#/…` links stay as they are
//     (the hash router takes them — `[Git](#/walkthroughs/git)` opens a guide).
// Browser-only (DOMParser); the pure parts live in ./guide.ts.

import { renderMarkdownGfm } from '../../lib/md';
import { isKeyCombo, splitChord } from './guide';

function chips(doc: Document, text: string): HTMLElement {
  const wrap = doc.createElement('span');
  wrap.className = 'keys';
  for (const part of splitChord(text)) {
    const k = doc.createElement('kbd');
    k.textContent = part;
    wrap.appendChild(k);
  }
  return wrap;
}

export function renderGuideHtml(markdown: string): string {
  const safe = renderMarkdownGfm(markdown);
  const doc = new DOMParser().parseFromString(`<body>${safe}</body>`, 'text/html');
  const body = doc.body;

  for (const table of Array.from(body.querySelectorAll('table'))) {
    const head = table.querySelector('th')?.textContent?.trim().toLowerCase() ?? '';
    if (head === 'keys' || head === 'key' || head === 'shortcut') {
      table.classList.add('keys-table');
      for (const row of Array.from(table.querySelectorAll('tbody tr'))) {
        const cell = row.querySelector('td');
        if (!cell) continue;
        for (const code of Array.from(cell.querySelectorAll('code'))) code.replaceWith(chips(doc, code.textContent ?? ''));
      }
    }
    const wrap = doc.createElement('div');
    wrap.className = 'table-wrap';
    table.replaceWith(wrap);
    wrap.appendChild(table);
  }

  for (const code of Array.from(body.querySelectorAll('code'))) {
    if (code.closest('pre')) continue;
    const t = code.textContent ?? '';
    if (isKeyCombo(t)) code.replaceWith(chips(doc, t));
  }

  for (const a of Array.from(body.querySelectorAll('a[href]'))) {
    const href = a.getAttribute('href') ?? '';
    if (/^https?:/i.test(href)) {
      a.setAttribute('target', '_blank');
      a.setAttribute('rel', 'noopener noreferrer');
    }
  }
  return body.innerHTML;
}
