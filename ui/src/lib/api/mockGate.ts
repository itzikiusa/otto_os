// The mock-API switch, kept apart from the 100 KB fixture module (perf F10):
// main.ts asks this and imports `./mock` only when the answer is yes, so the
// production entry never carries the fixtures.
export function mockEnabled(): boolean {
  try {
    if (import.meta.env.VITE_OTTO_MOCK === '1') return true;
    return localStorage.getItem('otto_mock') === '1';
  } catch {
    return false;
  }
}
