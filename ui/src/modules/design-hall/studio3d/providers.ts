// Text → 3D / Image → 3D providers for the 3D Studio's ✨ Generate menu
// (proposal §5.4). Pluggable, OFF by default, and honest about where a prompt
// goes:
//
//   • local-blender — runs ONE Design Hall agent turn on this scene. The agent
//     builds the object as scene3d primitives and may drive Blender through
//     the Blender MCP server when it is configured; every MCP tool call goes
//     through the MCP control plane's approval (the Blender template ships
//     deny-by-default), so it always asks. Nothing leaves the Mac except what
//     the chosen agent provider (Claude/Codex) already sees.
//   • tripo / meshy — opt-in cloud APIs. They stay disabled until an API key is
//     stored in the Keychain from Settings; this build has no key store for
//     them yet, so they render as disabled with their cost / privacy note and
//     `generate` refuses before any network call.
//
// Pure (type imports only) so the availability rules are unit-tested.

export type Gen3dKind = 'text' | 'image';
export type Gen3dProviderId = 'local-blender' | 'tripo' | 'meshy';
export type Gen3dQuality = 'draft' | 'standard';

export interface Gen3dProvider {
  id: Gen3dProviderId;
  label: string;
  where: 'local' | 'cloud';
  kinds: Gen3dKind[];
  /** What leaves the Mac, in one line (shown under the picker). */
  privacy: string;
  /** Rough cost per generation (null = no per-call cost). */
  cost: string | null;
  /** Hosts the daemon would call (netguard allow-list entries when enabled). */
  hosts: string[];
}

export const GEN3D_PROVIDERS: readonly Gen3dProvider[] = [
  {
    id: 'local-blender',
    label: 'Local: Otto + Blender MCP',
    where: 'local',
    kinds: ['text', 'image'],
    privacy: 'Runs an Otto agent turn on this scene; Blender MCP tool calls ask for approval first.',
    cost: null,
    hosts: [],
  },
  {
    id: 'tripo',
    label: 'Cloud: Tripo',
    where: 'cloud',
    kinds: ['text', 'image'],
    privacy: 'Uses an API key from Keychain · sends your prompt (and image) to Tripo.',
    cost: '≈ $0.20–$0.40 per model (credits)',
    hosts: ['api.tripo3d.ai'],
  },
  {
    id: 'meshy',
    label: 'Cloud: Meshy',
    where: 'cloud',
    kinds: ['text', 'image'],
    privacy: 'Uses an API key from Keychain · sends your prompt (and image) to Meshy.',
    cost: '≈ 20–30 credits per model (Pro: $20 / 1 000 credits)',
    hosts: ['api.meshy.ai'],
  },
];

export interface Gen3dContext {
  /** Cloud providers with a Keychain key (none in this build). */
  cloudKeys: Partial<Record<Gen3dProviderId, boolean>>;
  /** The person may edit this design. */
  canEdit: boolean;
  /** An agent turn already holds the artifact (the daemon allows one). */
  busy: boolean;
  /** Unsaved edits would be overwritten by / conflict with the agent version. */
  dirty: boolean;
}

/** Why `p` can't run for `kind` right now — null when it can. */
export function unavailableReason(p: Gen3dProvider, kind: Gen3dKind, ctx: Gen3dContext): string | null {
  if (!p.kinds.includes(kind)) return `${p.label} doesn’t do ${kind} → 3D`;
  if (!ctx.canEdit) return 'You can’t edit this design';
  if (p.where === 'cloud' && !ctx.cloudKeys[p.id]) return 'Add an API key in Settings to enable (off by default)';
  if (p.where === 'local' && ctx.busy) return 'Otto is already working on this design';
  if (p.where === 'local' && ctx.dirty) return 'Save your edits first — the result lands as a new version';
  return null;
}

/** The provider the picker preselects: the first one that can run, else the local one. */
export function defaultProvider(kind: Gen3dKind, ctx: Gen3dContext): Gen3dProvider {
  return GEN3D_PROVIDERS.find((p) => !unavailableReason(p, kind, ctx)) ?? GEN3D_PROVIDERS[0];
}

/**
 * The instruction for the local agent turn. The scene3d skill is already in
 * the agent's brief; this pins WHAT to add and the house rules for it.
 */
export function localPrompt(kind: Gen3dKind, prompt: string, quality: Gen3dQuality, imageRef?: string | null): string {
  const what = prompt.trim() || (kind === 'image' ? 'the object in the reference image' : 'a simple prop');
  const detail =
    quality === 'draft'
      ? 'Keep it a quick blockout: 3–8 primitives.'
      : 'Aim for a clean, readable model: up to ~30 primitives, rounded boxes (radius) where edges should soften, physical material presets (glossy-plastic, brushed-metal, frosted-glass, matte-paper, satin) and brand token colours (token:color.<name>) when the brief mentions the brand.';
  const image = kind === 'image' && imageRef ? ` Match the shape and colours of the reference image [${imageRef}].` : '';
  return (
    `Add ${what} to this scene.${image} Build it as a NEW group of scene3d objects placed beside the existing ` +
    `objects (don’t move or change anything else). ${detail} If the Blender MCP tools are available you may ` +
    'model it in Blender first, but the saved file must stay a valid scene3d v2 document. Name the group after the object.'
  );
}

/** "Blockout from prompt (scene JSON)" — a fresh scene from a brief (assist mode `generate`). */
export function blockoutPrompt(prompt: string): string {
  return (
    `${prompt.trim()}\n\nWrite a scene3d v2 blockout: metres, y-up, origin at the floor. Use the studio-soft ` +
    'environment, a key + rim light, physical material presets, a "Hero angle" camera (cameras[].id = "hero") and ' +
    'Idle/Hover states when something should react on hover.'
  );
}

/** "Refine in Blender (MCP)" — a refine turn that may drive Blender through its MCP server. */
export function blenderRefinePrompt(prompt: string): string {
  return (
    `${prompt.trim() || 'Refine the scene: better proportions, softer edges, nicer lighting.'}\n\n` +
    'Use the Blender MCP tools if they are available (each call asks for approval) to inspect or refine the ' +
    'model, then write the result back as a valid scene3d v2 document (primitives, presets, lights). If Blender ' +
    'MCP is not available, refine the scene3d document directly.'
  );
}
