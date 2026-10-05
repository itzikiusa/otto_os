// `plural(1, 'row')` → "1 row", `plural(3, 'row')` → "3 rows",
// `plural(2, 'index', 'indexes')` → "2 indexes". Replaces "(s)" and "1 rows".
export function plural(n: number, word: string, pluralWord?: string): string {
  return `${n} ${pluralNoun(n, word, pluralWord)}`;
}

// The noun alone, for when the count is rendered separately — wrapped in
// <strong> or locale-formatted (`1,234 rows`): `pluralNoun(3, 'row')` → "rows".
export function pluralNoun(n: number, word: string, pluralWord?: string): string {
  return n === 1 ? word : (pluralWord ?? `${word}s`);
}
