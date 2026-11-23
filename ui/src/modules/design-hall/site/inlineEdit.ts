// Site Studio inline text-edit helpers shared by SiteCanvas.svelte.
// Reads authored text from a contentEditable element without CSS transforms
// (e.g. text-transform: uppercase on eyebrows would corrupt saved copy).

export const MULTILINE_KEYS = new Set([
  'headline',
  'subhead',
  'heading',
  'subheading',
  'body',
  'quote',
  'answer',
  'blurb',
  'tagline',
]);

export function readAuthoredText(el: HTMLElement): string {
  let out = '';
  const walk = (n: Node) => {
    for (const c of Array.from(n.childNodes)) {
      if (c.nodeType === Node.TEXT_NODE) out += c.textContent ?? '';
      else if (c.nodeName === 'BR') out += '\n';
      else {
        if (c.nodeName === 'P' && out.trim()) out = out.replace(/\n*$/, '') + '\n\n';
        else if (c.nodeName === 'DIV' && out && !out.endsWith('\n')) out += '\n';
        walk(c);
      }
    }
  };
  walk(el);
  return out;
}
