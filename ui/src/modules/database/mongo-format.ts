// Pretty-printer for the Mongo query editor's shorthand (`db.coll.op({…})`,
// aggregate pipelines, and bare JSON commands). The argument is mongosh-ish — not
// valid JSON we could `JSON.parse` — so this is a structural re-indenter that is
// string / regex / comment aware, never reaching inside a literal.
//
// Layout rules: indent the contents of `{ }` / `[ ]`; keep an empty `{}` / `[]`
// inline; put each object entry / array element on its own line; one space after
// `:`. Method-call parens (`find(`, `aggregate(`) stay inline so the inner object
// drives the shape. It only RE-LAYS-OUT whitespace + line breaks — it never adds,
// removes, or reorders meaningful tokens, so a valid query stays valid (and an
// invalid one is only reflowed, not "fixed").

const INDENT = '  ';

/** True when a `/` at this point starts a regex literal (a value position) vs a
 *  division operator — decided by the last meaningful char already emitted. */
function isRegexStart(out: string): boolean {
  const last = out.replace(/\s+$/, '').slice(-1);
  return last === '' || ':,([{=!&|?'.includes(last);
}

export function formatMongo(src: string): string {
  const s = src;
  const n = s.length;
  let out = '';
  let depth = 0;
  const pad = (d = depth) => INDENT.repeat(Math.max(0, d));
  const trimLine = () => {
    out = out.replace(/[ \t]+$/, '');
  };
  let i = 0;

  while (i < n) {
    const c = s[i];

    // --- string literals (copied verbatim, escapes preserved) ---
    if (c === '"' || c === "'" || c === '`') {
      out += c;
      i++;
      while (i < n) {
        out += s[i];
        if (s[i] === '\\') {
          i++;
          if (i < n) {
            out += s[i];
            i++;
          }
          continue;
        }
        if (s[i] === c) {
          i++;
          break;
        }
        i++;
      }
      continue;
    }

    // --- comments (copied verbatim) ---
    if (c === '/' && s[i + 1] === '/') {
      while (i < n && s[i] !== '\n') {
        out += s[i];
        i++;
      }
      continue;
    }
    if (c === '/' && s[i + 1] === '*') {
      out += '/*';
      i += 2;
      while (i < n && !(s[i] === '*' && s[i + 1] === '/')) {
        out += s[i];
        i++;
      }
      out += '*/';
      i += 2;
      continue;
    }

    // --- regex literal in a value position (e.g. { name: /jo{1,2}n/i }) ---
    if (c === '/' && isRegexStart(out)) {
      out += '/';
      i++;
      let inClass = false;
      while (i < n) {
        const ch = s[i];
        if (ch === '\\') {
          out += ch + (s[i + 1] ?? '');
          i += 2;
          continue;
        }
        if (ch === '[') inClass = true;
        else if (ch === ']') inClass = false;
        out += ch;
        i++;
        if (ch === '/' && !inClass) break;
      }
      while (i < n && /[a-z]/i.test(s[i])) {
        out += s[i];
        i++;
      }
      continue;
    }

    // --- structure ---
    if (c === '{' || c === '[') {
      const close = c === '{' ? '}' : ']';
      let j = i + 1;
      while (j < n && /\s/.test(s[j])) j++;
      if (s[j] === close) {
        out += c + close; // empty {} / [] stays inline
        i = j + 1;
        continue;
      }
      depth++;
      out += c + '\n' + pad();
      i++;
      while (i < n && /\s/.test(s[i])) i++;
      continue;
    }
    if (c === '}' || c === ']') {
      depth = Math.max(0, depth - 1);
      trimLine();
      if (!out.endsWith('\n')) out += '\n';
      out += pad() + c;
      i++;
      continue;
    }
    if (c === ',') {
      trimLine();
      out += ',\n' + pad();
      i++;
      while (i < n && /[ \t\r\n]/.test(s[i])) i++;
      continue;
    }
    if (c === ':') {
      trimLine();
      out += ': ';
      i++;
      while (i < n && /[ \t]/.test(s[i])) i++;
      continue;
    }
    if (c === ';') {
      trimLine();
      out += ';';
      if (depth === 0) out += '\n';
      i++;
      while (i < n && /\s/.test(s[i])) i++;
      continue;
    }

    // --- whitespace: collapse to a single space between tokens ---
    if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
      if (out.length && !/[\s([{]$/.test(out)) out += ' ';
      i++;
      continue;
    }

    out += c;
    i++;
  }

  return out
    .replace(/[ \t]+$/gm, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}
