// Pure TTS Studio helpers: game-dev character voice presets (stored per
// workspace in localStorage) and multi-line dialogue script parsing
// (`CHARACTER: line` → speaker + text pairs). Unit-tested in ui/unit/ttsStudio.test.ts.

export type TtsAudioFormat = 'wav' | 'mp3' | 'm4a' | 'aiff';

export interface CharacterVoicePreset {
  id: string;
  name: string;
  role: string;
  provider_id: string;
  voice_id: string;
  speed: number;
  format: TtsAudioFormat;
  style_notes: string;
}

export interface ParsedDialogueLine {
  index: number;
  character: string | null;
  text: string;
  suggestedTitle: string;
}

export const DEFAULT_CHARACTER_PRESETS: readonly CharacterVoicePreset[] = [
  {
    id: 'narrator',
    name: 'Narrator',
    role: 'Story & lore voiceover',
    provider_id: 'macos-say',
    voice_id: 'Samantha',
    speed: 0.95,
    format: 'wav',
    style_notes: 'Calm, measured pacing for cutscenes and codex entries.',
  },
  {
    id: 'hero',
    name: 'Hero',
    role: 'Protagonist dialogue',
    provider_id: 'macos-say',
    voice_id: 'Daniel',
    speed: 1.0,
    format: 'wav',
    style_notes: 'Clear, direct delivery for in-game barks and dialogue choices.',
  },
  {
    id: 'merchant',
    name: 'Merchant',
    role: 'Shopkeeper & quest NPC',
    provider_id: 'macos-say',
    voice_id: 'Moira',
    speed: 1.05,
    format: 'wav',
    style_notes: 'Warm, lively cadence for greetings and vendor lines.',
  },
  {
    id: 'boss',
    name: 'Boss',
    role: 'Antagonist monologue',
    provider_id: 'macos-say',
    voice_id: 'Fred',
    speed: 0.85,
    format: 'wav',
    style_notes: 'Slower, dramatic emphasis for phase transitions.',
  },
];

const STORAGE_PREFIX = 'otto.tts.presets.v1.';

export function loadCharacterPresets(workspaceId: string | null): CharacterVoicePreset[] {
  if (typeof localStorage === 'undefined' || !workspaceId) return DEFAULT_CHARACTER_PRESETS.map((p) => ({ ...p }));
  try {
    const raw = localStorage.getItem(`${STORAGE_PREFIX}${workspaceId}`);
    if (!raw) return DEFAULT_CHARACTER_PRESETS.map((p) => ({ ...p }));
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed) || parsed.length === 0) return DEFAULT_CHARACTER_PRESETS.map((p) => ({ ...p }));
    return parsed.filter(
      (x): x is CharacterVoicePreset =>
        Boolean(x && typeof x === 'object' && typeof x.id === 'string' && typeof x.name === 'string'),
    );
  } catch {
    return DEFAULT_CHARACTER_PRESETS.map((p) => ({ ...p }));
  }
}

export function saveCharacterPresets(workspaceId: string | null, presets: readonly CharacterVoicePreset[]): void {
  if (typeof localStorage === 'undefined' || !workspaceId) return;
  try {
    localStorage.setItem(`${STORAGE_PREFIX}${workspaceId}`, JSON.stringify(presets));
  } catch {
    /* ignore storage quota failures */
  }
}

/** Turn a clip title or dialogue line into a clean workspace filename stem (e.g. `hero_we_hold_the_gate`). */
export function slugifyClipName(raw: string, fallback = 'voice_line'): string {
  const clean = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 48);
  return clean || fallback;
}

/**
 * Parse a dialogue script into speakable lines. Lines starting with `SPEAKER: text`
 * extract the character name; comment lines (`# …` or `// …`) and blank lines are skipped.
 */
export function parseDialogueScript(script: string): ParsedDialogueLine[] {
  const out: ParsedDialogueLine[] = [];
  const rawLines = script.split(/\r?\n/);
  for (const raw of rawLines) {
    const trimmed = raw.trim();
    if (!trimmed || trimmed.startsWith('#') || trimmed.startsWith('//')) continue;
    const match = /^([A-Za-z][A-Za-z0-9 _-]{0,30}):\s+(.+)$/.exec(trimmed);
    const character = match ? match[1].trim() : null;
    const text = (match ? match[2] : trimmed).trim();
    if (!text) continue;
    const idx = out.length + 1;
    const prefix = character ? `${character} ` : '';
    const snippet = text.slice(0, 32).trim();
    out.push({
      index: idx,
      character,
      text,
      suggestedTitle: `${prefix}${snippet}${text.length > 32 ? '…' : ''}`,
    });
  }
  return out;
}

/** Find a preset matching a parsed character tag (case-insensitive). */
export function matchPresetByCharacter(
  presets: readonly CharacterVoicePreset[],
  character: string | null,
): CharacterVoicePreset | null {
  if (!character) return null;
  const target = character.trim().toLowerCase();
  return presets.find((p) => p.name.trim().toLowerCase() === target) ?? null;
}

type VoiceLike = { id: string; name: string; locale: string | null };

/** One entry per voice id, first wins. A provider can list a voice twice
 *  (macOS `say -v ?` repeats some voices), and a keyed `{#each}` throws on
 *  the duplicate, which aborts the render and leaves the picker stuck. */
export function uniqueVoices<V extends VoiceLike>(voices: readonly V[]): V[] {
  const seen = new Set<string>();
  return voices.filter((v) => !seen.has(v.id) && Boolean(seen.add(v.id)));
}

/** The listed voice id for `wanted`: an exact id, else a variant of the same
 *  voice (macOS lists "Samantha" as "Samantha (English (US))"), else null. */
export function resolveVoiceId(voices: readonly VoiceLike[], wanted: string | null | undefined): string | null {
  if (!wanted) return null;
  return (voices.find((v) => v.id === wanted) ?? voices.find((v) => v.id.startsWith(`${wanted} (`)))?.id ?? null;
}

/** The voice picker's `<optgroup>`s: voices grouped by locale (A–Z), names
 *  A–Z inside a group; voices without a locale come last under ''. */
export function groupVoicesByLocale<V extends VoiceLike>(voices: readonly V[]): { locale: string; voices: V[] }[] {
  const groups = new Map<string, V[]>();
  for (const v of voices) {
    const key = v.locale ?? '';
    groups.set(key, [...(groups.get(key) ?? []), v]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => (a === '' ? 1 : b === '' ? -1 : a.localeCompare(b)))
    .map(([locale, list]) => ({ locale, voices: list.sort((x, y) => x.name.localeCompare(y.name)) }));
}

/** A voice group's readable name: "en_ZA" → "English (South Africa)"; the raw code if the
 *  browser can't name it. */
export function localeLabel(locale: string): string {
  try {
    return new Intl.DisplayNames(['en'], { type: 'language', languageDisplay: 'standard' }).of(locale.replace('_', '-')) ?? locale;
  } catch {
    return locale;
  }
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return '0:00';
  const totalSec = Math.floor(ms / 1000);
  const mins = Math.floor(totalSec / 60);
  const secs = totalSec % 60;
  return `${mins}:${secs.toString().padStart(2, '0')}`;
}

export function formatAudioBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
