// Minimal fuzzy matcher for the ⌘K palette: subsequence match with a score
// favouring word-boundary and consecutive hits.

export interface FuzzyResult {
  score: number;
  /** indices of matched chars in the haystack (for highlighting) */
  indices: number[];
}

export function fuzzyMatch(needle: string, haystack: string): FuzzyResult | null {
  const n = needle.toLowerCase();
  const h = haystack.toLowerCase();
  if (n.length === 0) return { score: 0, indices: [] };

  let score = 0;
  let hi = 0;
  let prev = -2;
  const indices: number[] = [];

  for (let ni = 0; ni < n.length; ni++) {
    const c = n[ni];
    const found = h.indexOf(c, hi);
    if (found === -1) return null;
    // boundary bonus
    if (found === 0 || h[found - 1] === ' ' || h[found - 1] === '-' || h[found - 1] === '/') {
      score += 8;
    }
    if (found === prev + 1) score += 5; // consecutive
    score -= (found - hi) * 0.5; // gap penalty
    indices.push(found);
    prev = found;
    hi = found + 1;
  }
  // shorter targets rank higher on equal matches
  score += Math.max(0, 10 - haystack.length * 0.05);
  return { score, indices };
}
