// Curated reviewer presets and installed review-skill preset builder,
// split out of ReviewPanel.svelte to keep the panel focused on review UI.

export interface LibrarySkillMeta {
  name: string;
  description: string;
  category?: string | null;
  version?: number | null;
  body?: string;
}

export interface ReviewerPreset {
  name: string;
  focus: string;
}

export const JSON_INSTR =
  'Output ONLY a JSON array (no prose, no markdown fence) of objects {"path":string,"line":number,"severity":"info"|"warn"|"bug","body":string} for issues you find in the diff. Empty array if none.';

export const REVIEWER_PRESETS: ReviewerPreset[] = [
  { name: 'Security & vulnerabilities', focus: 'Focus on security: injection, broken authn/authz, secret/credential handling, unsafe deserialization, SSRF, path traversal, and unvalidated input.' },
  { name: 'Performance & efficiency', focus: 'Focus on performance: N+1 queries, unnecessary allocations, O(n^2) hot paths, blocking I/O, missing caching/indexes, and oversized payloads.' },
  { name: 'Correctness & bugs', focus: 'Focus on correctness: logic errors, off-by-one, null/undefined handling, wrong edge cases, and broken invariants.' },
  { name: 'Tests & coverage', focus: 'Focus on testing: missing tests for new/changed behavior, untested edge cases, brittle or flaky tests, and missing assertions.' },
  { name: 'Error handling & resilience', focus: 'Focus on error handling: swallowed errors, missing propagation, unhandled failures, retries/timeouts, and resource cleanup.' },
  { name: 'Concurrency & races', focus: 'Focus on concurrency: data races, deadlocks, unsynchronized shared state, and incorrect async/await or locking.' },
  { name: 'API & interface design', focus: 'Focus on API design: breaking changes, inconsistent naming, unclear contracts, leaky abstractions, and backward compatibility.' },
  { name: 'Readability & style', focus: 'Focus on readability: unclear naming, dead code, overly complex functions, and inconsistent style.' },
  { name: 'Documentation', focus: 'Focus on documentation: missing or outdated doc comments, unclear public APIs, and undocumented behavior changes.' },
  { name: 'Dependencies & licensing', focus: 'Focus on dependencies: risky new deps, loose version pinning, known-vulnerable packages, and license concerns.' },
];

/** Prettify a skill name for display: "security-review" -> "Security review". */
export function prettifySkillName(name: string): string {
  const spaced = name.replace(/[-_]+/g, ' ').trim();
  if (spaced === '') return name;
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

/** Build a preset whose lens applies the named skill (materialized in-session). */
export function presetFromSkill(s: LibrarySkillMeta): ReviewerPreset {
  const desc = (s.description ?? '').trim();
  return {
    name: prettifySkillName(s.name),
    focus: `Apply the \`${s.name}\` skill (it is available to you): ${desc} ${JSON_INSTR}`,
  };
}
