// MCP server templates for the "Add server" form (design/product-design-arena.md
// §4.4). Static, UI-side only: nothing is seeded into `mcp_servers` at boot
// (rows need a workspace + creator). Picking a template pre-fills the form; the
// user reviews it and saves — every template starts DISABLED and with the most
// restrictive default tool access so nothing runs until they opt in.
import type { McpInjectionRisk, McpToolAccess, McpTransport } from '../../lib/api/types';

export interface McpServerTemplate {
  id: string;
  name: string;
  transport: McpTransport;
  description: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  injection_risk: McpInjectionRisk;
  default_tool_access: McpToolAccess;
  /** Setup notes shown under the picker (markdown-ish plain text + a link). */
  notes?: string;
  link?: string;
}

export const MCP_SERVER_TEMPLATES: readonly McpServerTemplate[] = [
  {
    id: 'blender',
    name: 'blender',
    transport: 'stdio',
    command: 'uvx',
    args: ['blender-mcp'],
    description:
      'Blender MCP — lets the design agents drive a running Blender (create/modify objects, materials, render) instead of only emitting scene3d files. Requires Blender 3.x+ with the blender-mcp addon enabled and its socket server started, and `uv` on PATH.',
    injection_risk: 'medium',
    // The design asks for "ask"; the control plane only knows allow | deny, so
    // start at deny (the user grants tools explicitly) — see §8 hand-offs.
    default_tool_access: 'deny',
    notes:
      'Setup: install the addon from the blender-mcp repo (Edit → Preferences → Add-ons → Install… → addon.py), tick it, then in the 3D viewport sidebar (N) → BlenderMCP → "Connect to Claude". Keep "Enable now" off until the addon is running.',
    link: 'https://github.com/ahujasid/blender-mcp',
  },
];
