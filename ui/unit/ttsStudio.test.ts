import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULT_CHARACTER_PRESETS,
  formatAudioBytes,
  formatDurationMs,
  groupVoicesByLocale,
  localeLabel,
  matchPresetByCharacter,
  parseDialogueScript,
  resolveVoiceId,
  slugifyClipName,
  uniqueVoices,
} from '../src/modules/design-hall/tts/ttsPresets.ts';
import { formatForFile, formatLabel, parseDesignRoute, renderKind } from '../src/modules/design-hall/model.ts';

test('parseDialogueScript extracts CHARACTER: line entries and skips blank/comment lines', () => {
  const script = [
    '# Act 1 Intro',
    '',
    'Narrator: Beyond the northern ridge lies the sunken forge.',
    '// stage direction',
    'Hero: Keep your lantern lit!',
    'Plain narration without a speaker prefix.',
  ].join('\n');

  const lines = parseDialogueScript(script);
  assert.equal(lines.length, 3);
  assert.equal(lines[0].character, 'Narrator');
  assert.equal(lines[0].text, 'Beyond the northern ridge lies the sunken forge.');
  assert.equal(lines[1].character, 'Hero');
  assert.equal(lines[1].text, 'Keep your lantern lit!');
  assert.equal(lines[2].character, null);
  assert.equal(lines[2].text, 'Plain narration without a speaker prefix.');
});

test('matchPresetByCharacter matches case-insensitively and returns null for unknown speakers', () => {
  const hero = matchPresetByCharacter(DEFAULT_CHARACTER_PRESETS, 'HERO');
  assert.ok(hero);
  assert.equal(hero.id, 'hero');
  assert.equal(matchPresetByCharacter(DEFAULT_CHARACTER_PRESETS, 'Dragon'), null);
  assert.equal(matchPresetByCharacter(DEFAULT_CHARACTER_PRESETS, null), null);
});

test('slugifyClipName, formatDurationMs, and formatAudioBytes normalize audio metadata', () => {
  assert.equal(slugifyClipName('Hero: Hold the Gate!'), 'hero_hold_the_gate');
  assert.equal(slugifyClipName('   ', 'fallback'), 'fallback');
  assert.equal(formatDurationMs(0), '0:00');
  assert.equal(formatDurationMs(65_000), '1:05');
  assert.equal(formatDurationMs(null), '0:00');
  assert.equal(formatAudioBytes(512), '512 B');
  assert.equal(formatAudioBytes(2048), '2.0 KB');
  assert.equal(formatAudioBytes(2 * 1024 * 1024), '2.00 MB');
});

test('Design Hall route and format helpers recognize #/design/tts and .obj 3D models', () => {
  assert.deepEqual(parseDesignRoute(['design', 'tts']), { view: 'tts' });
  assert.equal(formatForFile('character.obj'), 'obj');
  assert.equal(renderKind('obj'), 'model');
  assert.equal(formatLabel('obj'), 'OBJ model');
});

test('uniqueVoices drops repeated ids (macOS lists some voices twice)', () => {
  const v = (id: string, locale: string | null = 'en_US') => ({ id, name: id, locale });
  const out = uniqueVoices([v('Samantha'), v('Carmit', 'he_IL'), v('Samantha'), v('Carmit', 'he_IL')]);
  assert.deepEqual(out.map((x) => x.id), ['Samantha', 'Carmit']);
});

test('groupVoicesByLocale sorts locales and names, locale-less voices last', () => {
  const groups = groupVoicesByLocale([
    { id: 'z', name: 'Zoe', locale: 'en_US' },
    { id: 'a', name: 'Alice', locale: 'it_IT' },
    { id: 'x', name: 'af_bella', locale: null },
    { id: 'b', name: 'Albert', locale: 'en_US' },
  ]);
  assert.deepEqual(groups.map((g) => g.locale), ['en_US', 'it_IT', '']);
  assert.deepEqual(groups[0].voices.map((v) => v.name), ['Albert', 'Zoe']);
});

test('localeLabel names a voice group instead of showing the raw code', () => {
  assert.equal(localeLabel('en_ZA'), 'English (South Africa)');
  assert.equal(localeLabel('es_ES'), 'Spanish (Spain)');
  assert.equal(localeLabel('not a locale!'), 'not a locale!');
});

test('resolveVoiceId maps a preset voice to the listed variant', () => {
  const list = [{ id: 'Albert', name: 'Albert', locale: 'en_US' }, { id: 'Samantha (English (US))', name: 'Samantha (English (US))', locale: 'en_US' }];
  assert.equal(resolveVoiceId(list, 'Albert'), 'Albert');
  assert.equal(resolveVoiceId(list, 'Samantha'), 'Samantha (English (US))');
  assert.equal(resolveVoiceId(list, 'Sam'), null);
  assert.equal(resolveVoiceId(list, null), null);
});
