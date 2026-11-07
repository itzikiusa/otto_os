// Port for the Threads conversation view of one structured-runtime thread,
// for features that host it inside their own chrome (a personal agent's
// chat). lib/ never imports modules/, and one feature never reaches into
// another, so the shell binds the loader at boot
// (shell/installShell.svelte.ts) and hosts load it through here.
import type { Component } from 'svelte';

export type RuntimeThreadViewProps = { sessionId: string; workspaceId: string; agentName?: string };
type RuntimeThreadViewModule = { default: Component<RuntimeThreadViewProps> };
let loader: (() => Promise<RuntimeThreadViewModule>) | null = null;

export function registerRuntimeThreadView(load: () => Promise<RuntimeThreadViewModule>): void {
  loader = load;
}

export function loadRuntimeThreadView(): Promise<RuntimeThreadViewModule> {
  if (!loader) return Promise.reject(new Error('Threads aren’t available in this window'));
  return loader();
}
