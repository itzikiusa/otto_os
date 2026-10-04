// `plural(1, 'row')` → "1 row", `plural(3, 'row')` → "3 rows",
// `plural(2, 'index', 'indexes')` → "2 indexes". Replaces "(s)" and "1 rows".
export function plural(n: number, word: string, pluralWord?: string): string {
  return `${n} ${n === 1 ? word : (pluralWord ?? `${word}s`)}`;
}
