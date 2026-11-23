import type { ProductStory } from './types';

export type PromoteDocKind = 'rfc' | 'adr';

export interface PromoteNoteInput {
  story: Pick<ProductStory, 'id' | 'source_key' | 'title' | 'stage' | 'url'>;
  kind: PromoteDocKind;
  bodyMd?: string | null;
  planMd?: string | null;
  nowIso?: string;
}

/** Slugify a story key + title into a safe, human-readable vault filename. */
export function slugifyFilename(raw: string, fallback = 'untitled'): string {
  const slug = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return slug || fallback;
}

/** Normalize a user-edited vault note path (POSIX slashes, no leading slash, `.md` suffix, no `..`). */
export function normalizeVaultNotePath(raw: string): string | null {
  const cleaned = raw.trim().replace(/\\/g, '/').replace(/^\/+/, '');
  if (!cleaned) return null;
  const parts = cleaned.split('/').filter(Boolean);
  if (parts.length === 0 || parts.some((p) => p === '.' || p === '..')) return null;
  const joined = parts.join('/');
  return /\.md$/i.test(joined) ? joined : `${joined}.md`;
}

/** Default OKF folder + filename for promoting a story to an RFC or ADR. */
export function defaultVaultNotePath(
  story: Pick<ProductStory, 'source_key' | 'title' | 'id'>,
  kind: PromoteDocKind,
): string {
  const dir = kind === 'adr' ? 'adrs' : 'rfcs';
  const keyPart = story.source_key?.trim() ? `${story.source_key.trim()} ` : '';
  const name = slugifyFilename(`${keyPart}${story.title || story.id}`, kind);
  return `${dir}/${name}.md`;
}

function yamlScalar(v: string): string {
  return JSON.stringify(v.replace(/\r\n/g, '\n'));
}

/** Build an OKF-formatted Markdown RFC/ADR note with frontmatter back-linking the Product story. */
export function buildOkfStoryNote(input: PromoteNoteInput): string {
  const { story, kind, bodyMd, planMd } = input;
  const label = kind === 'adr' ? 'ADR' : 'RFC';
  const date = (input.nowIso ?? new Date().toISOString()).slice(0, 10);
  const storyKey = story.source_key?.trim() || story.id;
  const storyRef = `otto://story/${story.id}`;
  const keyTag = slugifyFilename(storyKey, 'story');

  const frontmatter = [
    '---',
    `type: ${kind}`,
    'status: draft',
    `title: ${yamlScalar(`${label}: ${story.title || storyKey}`)}`,
    `story_id: ${yamlScalar(story.id)}`,
    `story_key: ${yamlScalar(storyKey)}`,
    `story_ref: ${yamlScalar(storyRef)}`,
    ...(story.stage ? [`story_stage: ${yamlScalar(story.stage)}`] : []),
    ...(story.url ? [`source_url: ${yamlScalar(story.url)}`] : []),
    `created: ${date}`,
    'tags:',
    `  - product/${kind}`,
    `  - story/${keyTag}`,
    '---',
  ].join('\n');

  const sections: string[] = [
    frontmatter,
    '',
    `# ${label}: ${story.title || storyKey}`,
    '',
    `> Promoted from Product story [${storyKey}](${storyRef}).`,
  ];

  const trimmedBody = bodyMd?.trim();
  if (trimmedBody) {
    sections.push('', '## Context & Specification', '', trimmedBody);
  }

  const trimmedPlan = planMd?.trim();
  if (trimmedPlan) {
    sections.push('', '## Implementation Plan', '', trimmedPlan);
  }

  sections.push(
    '',
    kind === 'adr' ? '## Decision & Consequences' : '## Open Questions & Decisions',
    '',
    '- ',
    '',
  );

  return sections.join('\n');
}
