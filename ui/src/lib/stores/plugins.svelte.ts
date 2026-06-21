// Runtime custom-plugins: the enabled set (for sidebar) is fetched from the
// daemon (GET /plugins) — plugins are installed/removed at runtime, not bundled.

import { api } from '../api/client';

/** Enabled-plugin nav info (GET /plugins). */
export interface PluginNav {
  slug: string;
  name: string;
  icon: string;
  has_ui: boolean;
}

/** Full installed-plugin record (GET /plugin-admin, root only). */
export interface PluginRecord {
  slug: string;
  name: string;
  icon: string;
  version: string;
  description: string;
  source: string;
  exec: string[];
  ui_dir: string | null;
  health: string;
  enabled: boolean;
  installed_at: string;
}

class PluginsStore {
  /** Enabled plugins for the current user's sidebar (UI filters by canPlugin). */
  list = $state<PluginNav[]>([]);

  /** Refresh the enabled-plugin list (non-fatal on error). */
  async load(): Promise<void> {
    try {
      this.list = await api.get<PluginNav[]>('/plugins');
    } catch {
      // non-fatal: keep the previous list (or empty)
    }
  }

  get(slug: string): PluginNav | undefined {
    return this.list.find((p) => p.slug === slug);
  }
}

export const plugins = new PluginsStore();
