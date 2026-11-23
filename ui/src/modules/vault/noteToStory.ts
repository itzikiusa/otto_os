function stripNoteFrontmatter(raw: string): string {
  if (!raw.startsWith('---')) return raw;
  const m = raw.match(/^---\r?\n[\s\S]*?\r?\n(?:---|\.\.\.)[^\n]*\r?\n?/);
  return m ? raw.slice(m[0].length) : raw;
}

export interface NoteToStoryInput {
  vaultId: number;
  notePath: string;
  raw: string;
  includeBody?: boolean;
}

/** Build the canonical `otto://vault/<vaultId>/<path>` back-link URI. */
export function vaultRefUri(vaultId: number, notePath: string): string {
  const cleanPath = notePath.replace(/^\/+/, '');
  return `otto://vault/${vaultId}/${cleanPath}`;
}

/** Pick the best initial story title from meta title, first H1 heading, or filename. */
export function extractNoteTitle(notePath: string, raw: string, metaTitle?: string | null): string {
  const explicit = metaTitle?.trim();
  if (explicit) return explicit;
  const body = stripNoteFrontmatter(raw);
  const h1 = /^\s*#\s+(.+?)\s*$/m.exec(body);
  if (h1 && h1[1].trim()) return h1[1].trim();
  const base = notePath.split('/').pop() ?? notePath;
  return base.replace(/\.md$/i, '').trim() || 'Untitled story';
}

/** Build the Product Story draft markdown body seeded from a Vault note + `otto://vault/...` back-link. */
export function buildStoryBodyFromNote(input: NoteToStoryInput): string {
  const ref = vaultRefUri(input.vaultId, input.notePath);
  const header = `> Seeded from Vault note [${input.notePath}](${ref}) (\`${ref}\`).`;
  if (input.includeBody === false) return `${header}\n`;
  const body = stripNoteFrontmatter(input.raw).trim();
  if (!body) return `${header}\n`;
  return `${header}\n\n${body}\n`;
}
