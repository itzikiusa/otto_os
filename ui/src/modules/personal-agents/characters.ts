// Personal-agent character avatars (ui/src/lib/assets/agents, one per
// personality) and the Otto mascot. Stored on the agent as `avatar:
// "char:<id>"`; anything else is a glyph / initials (AgentAvatar). Imported
// as URLs from this lazily loaded module: Vite emits files, nothing is
// inlined into a JS chunk, and an image is fetched only when shown.

import autodidact from '../../lib/assets/agents/autodidact.webp?url';
import blunt from '../../lib/assets/agents/blunt.webp?url';
import builder from '../../lib/assets/agents/builder.webp?url';
import chief from '../../lib/assets/agents/chief.webp?url';
import cynic from '../../lib/assets/agents/cynic.webp?url';
import formal from '../../lib/assets/agents/formal.webp?url';
import free from '../../lib/assets/agents/free.webp?url';
import nerd from '../../lib/assets/agents/nerd.webp?url';
import playful from '../../lib/assets/agents/playful.webp?url';
import strict from '../../lib/assets/agents/strict.webp?url';
import terse from '../../lib/assets/agents/terse.webp?url';
import warm from '../../lib/assets/agents/warm.webp?url';
import celebrate from '../../lib/assets/mascot/celebrate.webp?url';
import sleep from '../../lib/assets/mascot/sleep.webp?url';
import thinking from '../../lib/assets/mascot/thinking.webp?url';
import wave from '../../lib/assets/mascot/wave.webp?url';

export interface Character {
  id: string;
  label: string;
  url: string;
}

export const CHARACTERS: readonly Character[] = [
  { id: 'chief', label: 'Chief — headset', url: chief },
  { id: 'warm', label: 'Warm — peach', url: warm },
  { id: 'nerd', label: 'Nerd — glasses', url: nerd },
  { id: 'autodidact', label: 'Autodidact — book and bulb', url: autodidact },
  { id: 'free', label: 'Free-willed — wavy blob', url: free },
  { id: 'cynic', label: 'Cynic — side-eye', url: cynic },
  { id: 'strict', label: 'Strict — bow tie', url: strict },
  { id: 'blunt', label: 'Blunt — red cube', url: blunt },
  { id: 'playful', label: 'Playful — winking star', url: playful },
  { id: 'formal', label: 'Formal — monocle', url: formal },
  { id: 'terse', label: 'Terse — small circle', url: terse },
  { id: 'builder', label: 'Builder — hard hat', url: builder },
];

/** The character each personality preset defaults to. */
const BY_PERSONALITY: Record<string, string> = {
  chief: 'chief',
  warm: 'warm',
  blunt: 'blunt',
  nerdy: 'nerd',
  cynical: 'cynic',
  strict: 'strict',
  independent: 'free',
  autodidact: 'autodidact',
  playful: 'playful',
  formal: 'formal',
  terse: 'terse',
};

export const CHAR_PREFIX = 'char:';

/** The image URL for an `avatar` value, or null for a glyph / initials. */
export function characterUrl(avatar: string | null | undefined): string | null {
  const a = (avatar ?? '').trim();
  if (!a.startsWith(CHAR_PREFIX)) return null;
  const id = a.slice(CHAR_PREFIX.length);
  return CHARACTERS.find((c) => c.id === id)?.url ?? null;
}

/** The `avatar` value of a personality's default character. */
export function avatarForPersonality(preset: string): string {
  return `${CHAR_PREFIX}${BY_PERSONALITY[preset] ?? 'warm'}`;
}

export const MASCOT = { wave, thinking, celebrate, sleep } as const;
