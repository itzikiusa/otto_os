// Code blocks in chat prose get a small toolbar — language label, Wrap, Copy —
// and a capped height with "Show all N lines" for long ones. The markdown is
// rendered to an HTML STRING once and cached (mdCache.ts), so the toolbar is
// added to that string here (after the sanitizer: it is our own markup around
// already-escaped code) and driven by ONE delegated click handler per block of
// prose (`runCodeAction`) that toggles classes on the live DOM. No component
// per code block, nothing re-rendered on click, the cache stays a string memo.

/** Blocks longer than this start collapsed to about this many lines. */
export const CODE_CAP_LINES = 18;

// vault/mdRender.ts emits `<pre><code class="hljs[ language-x]">…</code></pre>`
// for every fenced block (diagram fences render as `.diagram-block` instead).
// A block already wrapped (its `<pre>` directly follows our `.code-head`) is
// skipped, so decorating twice never nests toolbars.
const PRE = /(?<!<\/div>)<pre><code class="hljs(?: language-([\w+#.-]+))?">([\s\S]*?)<\/code><\/pre>/g;

const esc = (s: string): string => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Wrap each fenced code block of rendered markdown in the chat's code-block
 *  chrome. Idempotent. */
export function decorateCodeBlocks(html: string): string {
  if (!html.includes('<pre><code class="hljs')) return html;
  return html.replace(PRE, (_m, lang: string | undefined, body: string) => {
    const lines = body.replace(/\n$/, '').split('\n').length;
    const long = lines > CODE_CAP_LINES;
    const label = lang ? esc(lang) : '';
    const cls = lang ? `hljs language-${esc(lang)}` : 'hljs';
    return (
      `<div class="code-block${long ? ' capped' : ''}" data-lines="${lines}">` +
      `<div class="code-head"><span class="code-lang">${label}</span>` +
      `<button type="button" class="code-btn" data-code-act="wrap" aria-pressed="false" title="Wrap long lines">Wrap</button>` +
      `<button type="button" class="code-btn" data-code-act="copy" title="Copy code">Copy</button></div>` +
      `<pre><code class="${cls}">${body}</code></pre>` +
      (long
        ? `<button type="button" class="code-more" data-code-act="expand" aria-expanded="false">Show all ${lines} lines</button>`
        : '') +
      `</div>`
    );
  });
}

/** Handle a click inside rendered prose. Returns true when it was a code-block
 *  control (the caller stops there). `copy` is injected for tests/fallbacks. */
export function runCodeAction(
  target: EventTarget | null,
  copy: (text: string) => Promise<void> = (t) => navigator.clipboard.writeText(t),
): boolean {
  if (!(target instanceof Element)) return false;
  const btn = target.closest<HTMLElement>('[data-code-act]');
  const block = btn?.closest<HTMLElement>('.code-block');
  if (!btn || !block) return false;
  const act = btn.dataset.codeAct;
  if (act === 'copy') {
    const text = block.querySelector('code')?.textContent ?? '';
    void copy(text).then(
      () => flash(btn, 'Copied'),
      () => flash(btn, 'Copy failed'),
    );
  } else if (act === 'wrap') {
    const on = block.classList.toggle('wrap');
    btn.setAttribute('aria-pressed', String(on));
  } else if (act === 'expand') {
    const open = block.classList.toggle('expanded');
    btn.setAttribute('aria-expanded', String(open));
    btn.textContent = open ? 'Show less' : `Show all ${block.dataset.lines ?? ''} lines`;
  }
  return true;
}

function flash(btn: HTMLElement, text: string): void {
  const prev = btn.dataset.label ?? btn.textContent ?? '';
  btn.dataset.label = prev;
  btn.textContent = text;
  setTimeout(() => {
    btn.textContent = prev;
    delete btn.dataset.label;
  }, 1400);
}
