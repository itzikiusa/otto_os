// MCP profiles, shared by the MCP → Profiles page, the new-session "MCP tools"
// picker and the session menu's "MCP tools" row. One cached `GET
// /mcp/profiles` per page life (reloaded after an edit); the daemon resolves
// what a session actually gets, this only labels and offers.
import { mcpCpApi } from '../api/mcp';
import { loadErrorText } from '../loadError';
import type { McpProfile, McpToolGroup } from '../api/types';
import { selectionLabel } from '../mcpProfiles';

class McpProfilesStore {
  profiles = $state<McpProfile[]>([]);
  groups = $state<McpToolGroup[]>([]);
  defaultId = $state('full');
  loaded = $state(false);
  loading = $state(false);
  error = $state<string | null>(null);
  #inflight: Promise<void> | null = null;

  /** Fetch once (or again with `force`); concurrent callers share one request. */
  load(force = false): Promise<void> {
    if (this.loaded && !force) return Promise.resolve();
    if (this.#inflight) return this.#inflight;
    this.loading = true;
    this.error = null;
    this.#inflight = mcpCpApi
      .cpProfiles()
      .then((r) => {
        this.profiles = r.profiles;
        this.groups = r.groups;
        this.defaultId = r.default_id;
        this.loaded = true;
      })
      .catch((e) => {
        this.error = loadErrorText(e);
      })
      .finally(() => {
        this.loading = false;
        this.#inflight = null;
      });
    return this.#inflight;
  }

  /** "Database + Issues", or "Full (default)" for a session storing none. */
  labelFor(ids: string[]): string {
    return selectionLabel(ids, this.profiles, this.defaultId);
  }
}

export const mcpProfiles = new McpProfilesStore();
