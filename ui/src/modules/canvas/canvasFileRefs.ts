// Pure half of canvasFiles.ts (no imports, so `node --test` can load it):
// ref parsing, the autosave `files` map, and the content-address digest.
// See canvasFiles.ts for the why.

export const FILE_REF_PREFIX = 'otto-canvas-file:';

/** The sha of a ref data URL, or null for an inline one. */
export function refSha(dataURL: unknown): string | null {
  if (typeof dataURL !== 'string' || !dataURL.startsWith(FILE_REF_PREFIX)) return null;
  const sha = dataURL.slice(FILE_REF_PREFIX.length);
  return /^[0-9a-f]{64}$/i.test(sha) ? sha.toLowerCase() : null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type ExFile = { id: string; dataURL: string; [k: string]: any };

/**
 * The `files` map an autosave sends: every file the server already holds
 * (`known`: file id → sha) becomes a ref; the rest stay inline. Returns the
 * map plus the inline entries, whose shas are learned after the save lands.
 */
export function filesForSave(
  files: Record<string, ExFile> | null | undefined,
  known: ReadonlyMap<string, string>,
): { files: Record<string, ExFile>; inline: ExFile[] } {
  const out: Record<string, ExFile> = {};
  const inline: ExFile[] = [];
  for (const [id, f] of Object.entries(files ?? {})) {
    if (!f || typeof f !== 'object') continue;
    const sha = known.get(id);
    if (sha) {
      out[id] = { ...f, dataURL: FILE_REF_PREFIX + sha };
    } else if (refSha(f.dataURL)) {
      out[id] = f; // already a ref (not yet resolved) — send it as is
    } else {
      out[id] = f;
      if (typeof f.dataURL === 'string' && f.dataURL.startsWith('data:')) inline.push(f);
    }
  }
  return { files: out, inline };
}

/** sha256 hex of a data URL's text — the server's content address. */
export async function sha256Hex(text: string): Promise<string> {
  const buf = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, '0')).join('');
}
