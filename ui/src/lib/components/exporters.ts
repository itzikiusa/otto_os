// "Get the data out" helpers (T5). A consistent Copy-as-JSON / Export-CSV /
// Download-JSON affordance reused by Usage rollups, DB results, and Brokers peek.
// For data sets larger than the UI's preview cap, prefer a server-streamed
// export endpoint and feed the streamed text to `downloadText`; these client
// helpers cover what is already in memory.

/** Copy a value to the clipboard as pretty JSON (strings are copied verbatim). */
export async function copyAsJson(value: unknown): Promise<void> {
  const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  if (typeof navigator !== 'undefined' && navigator.clipboard) {
    await navigator.clipboard.writeText(text);
  }
}

/** Trigger a browser download of arbitrary text. */
export function downloadText(text: string, filename: string, mime = 'text/plain'): void {
  if (typeof document === 'undefined') return;
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** Download a value as a `.json` file. */
export function downloadJson(value: unknown, filename: string): void {
  const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
  downloadText(text, filename, 'application/json');
}

/**
 * Serialize an array of flat records to CSV and download it. Columns are the
 * union of all keys (stable first-seen order). Values containing `",\n` are
 * quoted/escaped per RFC 4180.
 */
export function exportCsv(rows: Record<string, unknown>[], filename: string): void {
  const cols: string[] = [];
  const seen = new Set<string>();
  for (const r of rows) {
    for (const k of Object.keys(r)) {
      if (!seen.has(k)) {
        seen.add(k);
        cols.push(k);
      }
    }
  }
  const esc = (v: unknown): string => {
    const s = v == null ? '' : String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const lines = [cols.map(esc).join(',')];
  for (const r of rows) lines.push(cols.map((c) => esc(r[c])).join(','));
  downloadText(lines.join('\n'), filename, 'text/csv');
}
