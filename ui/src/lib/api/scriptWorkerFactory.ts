/** Keep Vite's module-worker URL statically visible to its bundler. */
export function createScriptWorker(): Worker {
  return new Worker(new URL('./scriptWorker.ts', import.meta.url), { type: 'module' });
}
