// Runtime custom-plugins: the enabled set (for sidebar) is fetched from the
// daemon (GET /plugins) — plugins are installed/removed at runtime, not bundled.

import { api } from '../api/client';
import { asIcon, type IconName } from '../components/Icon.svelte';

/** Enabled-plugin nav info (GET /plugins). */
export interface PluginNav {
  slug: string;
  name: string;
  /** Normalized on load: a manifest icon we don't ship falls back to 'box'. */
  icon: IconName;
  has_ui: boolean;
  /** Its service (sidecar) is up. The page is served even when it is not, so
   *  the frame checks this to show "service stopped" instead of a dead page. */
  running: boolean;
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
  /** The service is up right now (admin list only). */
  running: boolean;
  /** Why an enabled plugin is not running (spawn error or how it exited). */
  last_error: string | null;
}

class PluginsStore {
  /** Enabled plugins for the current user's sidebar (UI filters by canPlugin). */
  list = $state<PluginNav[]>([]);

  /** Refresh the enabled-plugin list (non-fatal on error). */
  async load(): Promise<void> {
    try {
      const raw = await api.get<(Omit<PluginNav, 'icon'> & { icon: string })[]>('/plugins');
      this.list = raw.map((p) => ({ ...p, icon: asIcon(p.icon, 'box') }));
    } catch {
      // non-fatal: keep the previous list (or empty)
    }
  }

  get(slug: string): PluginNav | undefined {
    return this.list.find((p) => p.slug === slug);
  }
}

export const plugins = new PluginsStore();
