// Saved CloudWatch Logs Insights queries (per-account localStorage helper).
export interface SavedInsightsQuery {
  name: string;
  query: string;
}

export function readSavedQueries(key: string): SavedInsightsQuery[] {
  try {
    const raw = localStorage.getItem(key);
    const v = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? v.filter((x) => x && typeof x.name === 'string' && typeof x.query === 'string') : [];
  } catch {
    return [];
  }
}

export function writeSavedQueries(key: string, list: SavedInsightsQuery[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(list));
  } catch {
    // storage unavailable — the list still lives for this session
  }
}
