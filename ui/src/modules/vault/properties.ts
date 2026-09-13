import { isMap, parseDocument } from 'yaml';

export const PROPERTY_FIELDS = ['title', 'type', 'description', 'resource', 'tags', 'aliases', 'timestamp'] as const;

function frontmatter(raw: string) {
  const match = /^(?:\uFEFF)?---\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)(?:\r?\n|$)/.exec(raw);
  if (/^(?:\uFEFF)?---\r?\n/.test(raw) && !match) throw new Error('Close the frontmatter fence in the editor first.');
  const doc = parseDocument(match?.[1] ?? '{}');
  if (doc.errors.length || !isMap(doc.contents)) throw new Error('Fix malformed frontmatter in the editor first.');
  return {doc, body: match ? raw.slice(match[0].length) : raw};
}

export function readProperties(raw: string): Record<string, string> {
  const {doc} = frontmatter(raw);
  return Object.fromEntries(PROPERTY_FIELDS.map(key => {
    const value = doc.toJS()[key];
    return [key, Array.isArray(value) ? value.join(', ') : value == null ? '' : String(value)];
  }));
}

/** Patch known fields in the YAML document while retaining unknown nodes and
 * comments. The Markdown body is never parsed or reformatted. */
export function patchProperties(raw: string, updates: Record<string, string>): string {
  const {doc, body} = frontmatter(raw);
  for (const [key, value] of Object.entries(updates)) {
    if (!(PROPERTY_FIELDS as readonly string[]).includes(key)) throw new Error(`Unsupported property: ${key}`);
    if (!value.trim()) doc.delete(key);
    else doc.set(key, key === 'tags' || key === 'aliases'
      ? value.split(',').map(v => v.trim()).filter(Boolean) : value);
  }
  return `---\n${doc.toString()}---\n${body}`;
}
