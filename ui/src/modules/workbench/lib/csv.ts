// RFC 4180 CSV/TSV parse + stringify for the Workbench (format + table preview).
// Pure and dependency-free: quoted fields may hold the delimiter, `""` escapes
// and embedded newlines (CRLF or LF); a trailing newline does not add a row.

/** Parse `text` into rows of fields. `delimiter` defaults to a sniffed one
 *  (tab when the first line has tabs and no commas, else comma). */
export function parseCsv(text: string, delimiter?: string): string[][] {
  const d = delimiter ?? sniffDelimiter(text);
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        quoted = false;
        i++;
        continue;
      }
      field += c;
      i++;
      continue;
    }
    if (c === '"' && field === '') {
      quoted = true;
      i++;
      continue;
    }
    if (c === d) {
      row.push(field);
      field = '';
      i++;
      continue;
    }
    if (c === '\r' || c === '\n') {
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
      i += c === '\r' && text[i + 1] === '\n' ? 2 : 1;
      continue;
    }
    field += c;
    i++;
  }
  if (field !== '' || row.length > 0) {
    row.push(field);
    rows.push(row);
  }
  return rows;
}

/** Quote a field only when it needs it (delimiter, quote, CR/LF, or leading /
 *  trailing whitespace that would otherwise be ambiguous). */
function quoteField(f: string, d: string): string {
  if (f.includes(d) || f.includes('"') || f.includes('\n') || f.includes('\r') || /^\s|\s$/.test(f)) {
    return `"${f.replace(/"/g, '""')}"`;
  }
  return f;
}

/** Serialize rows with `\n` line endings (no trailing newline). */
export function stringifyCsv(rows: string[][], delimiter = ','): string {
  return rows.map((r) => r.map((f) => quoteField(f, delimiter)).join(delimiter)).join('\n');
}

/** Tab when the first line has a tab and no comma; otherwise comma. */
export function sniffDelimiter(text: string): string {
  const first = text.split(/\r?\n/, 1)[0] ?? '';
  return first.includes('\t') && !first.includes(',') ? '\t' : ',';
}
