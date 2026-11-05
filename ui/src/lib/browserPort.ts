// Port for the Browser module's view, for features that host it inside their
// own chrome (Threads' inspector tab). lib/ never imports modules/, and one
// feature never reaches into another, so the shell binds the loader at boot
// (shell/installShell.svelte.ts) and hosts load it through here.
import type { Component } from 'svelte';

type BrowserViewModule = { default: Component<{ embedded?: boolean; targetSessionId?: string | null }> };
let loader: (() => Promise<BrowserViewModule>) | null = null;

export function registerBrowserView(load: () => Promise<BrowserViewModule>): void {
  loader = load;
}

export function loadBrowserView(): Promise<BrowserViewModule> {
  if (!loader) return Promise.reject(new Error('The browser isn’t available in this window'));
  return loader();
}
