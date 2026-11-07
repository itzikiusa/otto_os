// Personality presets — the ONE source is
// docs/contracts/personal-agent-personalities.json (the daemon include_str!s
// the same file into the agent's persona). Tone only: a personality never
// changes an agent's rules, permissions or approvals.

import raw from '../../../../docs/contracts/personal-agent-personalities.json';

export interface PersonalityPreset {
  id: string;
  label: string;
  description: string;
  prompt: string;
  /** How the agent would answer "Can you look into this for me?". */
  sample: string;
  default_for?: 'main' | 'agent';
}

const ALL = raw.personalities as PersonalityPreset[];

export function personalityPresets(): PersonalityPreset[] {
  return ALL;
}

/** The preset an agent shows: its own, else its role's default. */
export function personalityOf(preset: string | undefined, main: boolean): PersonalityPreset | undefined {
  return ALL.find((p) => p.id === preset) ?? ALL.find((p) => p.default_for === (main ? 'main' : 'agent'));
}
