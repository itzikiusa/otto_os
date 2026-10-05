// Copy for the shared "Needs you" inbox link (S20-07) — pure so the wording is
// unit-tested; the count always comes from the Home Today store.
export function needsYouLinkLabel(count: number): string {
  if (count <= 0) return 'Needs you inbox';
  return `${count} ${count === 1 ? 'needs' : 'need'} you · Open inbox`;
}
