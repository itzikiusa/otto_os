// The "Compare results" source (ConnectionComparison.svelte): module state, so
// the pin survives leaving the workbench for the connection list to open the
// other side.
export const comparePin = $state<{ value: { connId: string; tabId: number; scope: string } | null }>({ value: null });
