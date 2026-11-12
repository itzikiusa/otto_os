// Full-text index completeness — shared wording for the header chip, the
// search panel notice and the rebuild result. Pure: unit-tested in
// ui/unit/vaultSearchIndex.test.ts.
import type { VaultSearchIndexStatus } from '../../lib/api/types';

const fmt = (n: number) => n.toLocaleString();

/** 0–100, never NaN (an empty vault is complete). */
export function searchIndexPercent(ix: VaultSearchIndexStatus): number {
  if (ix.total <= 0) return 100;
  return Math.max(0, Math.min(100, Math.floor((ix.indexed / ix.total) * 100)));
}

/** Short chip label: "Search index 41%". */
export function searchIndexChip(ix: VaultSearchIndexStatus): string {
  return `Search index ${searchIndexPercent(ix)}%`;
}

/** The sentence a partial result must carry. */
export function searchIndexNotice(ix: VaultSearchIndexStatus): string {
  const verb = ix.state === 'pending' ? 'is waiting to rebuild' : 'is rebuilding';
  return `The search index ${verb}: ${fmt(ix.indexed)} of ${fmt(ix.total)} notes indexed. Results may be incomplete.`;
}

/** Toast body when a rebuild that this page watched has finished. */
export function searchIndexDone(total: number): string {
  return `${fmt(total)} ${total === 1 ? 'note' : 'notes'} indexed. Search results are complete again.`;
}
