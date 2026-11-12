// Display helpers for binary result cells. The daemon sends MySQL BINARY /
// VARBINARY / BLOB and Postgres BYTEA cells as base64 text and marks the
// column `binary: true`; the cell viewer shows the bytes as hex with their
// length instead of base64 that reads like a string.
import { base64ToBytes } from '../b64.ts';
import { isBase64 } from './sql-dialect.ts';

/** Space-separated lowercase hex pairs, 16 bytes per line (a hex-dump body
 *  without offsets, so a copied UUID key stays one readable run per line). */
export function bytesToHex(bytes: Uint8Array, perLine = 16): string {
  const lines: string[] = [];
  for (let i = 0; i < bytes.length; i += perLine) {
    const row: string[] = [];
    for (let j = i; j < Math.min(i + perLine, bytes.length); j++) row.push(bytes[j].toString(16).padStart(2, '0'));
    lines.push(row.join(' '));
  }
  return lines.join('\n');
}

/** The bytes of a base64 cell, or null when the text isn't base64 (a capped
 *  or masked cell) — the viewer then shows the text as it came. */
export function binaryCellBytes(b64: string): Uint8Array | null {
  if (!isBase64(b64)) return null;
  try {
    return base64ToBytes(b64);
  } catch {
    return null;
  }
}

/** Compact hex (no spaces) for "Copy as hex" — pastes into `X'…'` / `0x…`. */
export function bytesToHexCompact(bytes: Uint8Array): string {
  let out = '';
  for (const b of bytes) out += b.toString(16).padStart(2, '0');
  return out;
}
