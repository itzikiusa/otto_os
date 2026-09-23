// Brand Kit — the three starter kits the "New brand kit" sheet offers. Each is
// a complete `otto-brand/1` document (colours with checked contrast, a type
// scale, spacing + radius, logo slots, imagery and voice) so every studio has
// something sensible to read on day one. Fonts are local system stacks only —
// no web-font CDN (the app works offline). Pure; unit-tested
// (ui/unit/brandTokens.test.ts checks each one validates).

import type { BrandDoc } from '../../../lib/api/types';

export interface StarterKit {
  id: 'vivid' | 'editorial' | 'minimal';
  label: string;
  blurb: string;
  build: (name: string) => BrandDoc;
}

const SANS = '-apple-system, "SF Pro Display", "Helvetica Neue", system-ui, sans-serif';
const TEXT = '-apple-system, "SF Pro Text", "Helvetica Neue", system-ui, sans-serif';
const SERIF = '"New York", "Iowan Old Style", Georgia, serif';
const MONO = 'ui-monospace, "SF Mono", Menlo, monospace';

const SPACE = { '2xs': 4, xs: 8, sm: 12, md: 16, lg: 24, xl: 32, '2xl': 48, '3xl': 64 };

function space(): BrandDoc['space'] {
  const out: BrandDoc['space'] = {};
  for (const [k, v] of Object.entries(SPACE)) out[k] = { $value: v };
  return out;
}

function logos(): BrandDoc['logos'] {
  return [
    { name: 'Full logo', kind: 'full', asset: '' },
    { name: 'Mark', kind: 'mark', asset: '' },
    { name: 'Mono', kind: 'mono', asset: '' },
  ];
}

export const STARTER_KITS: readonly StarterKit[] = [
  {
    id: 'vivid',
    label: 'Vivid',
    blurb: 'Violet and amber, heavy display type, rounded cards. Loud enough for launches.',
    build: (name) => ({
      $schema: 'otto-brand/1',
      name,
      color: {
        primary: { $value: '#5B3DF5', $description: 'Buttons, links and the one thing to click' },
        accent: { $value: '#FFB547', $description: 'Highlights and shapes. Never text on white' },
        ink: { $value: '#14122B', $description: 'Body text and dark surfaces' },
        surface: { $value: '#FFFFFF' },
        'surface-alt': { $value: '#F6F4FF' },
        success: { $value: '#1F9D55' },
      },
      font: {
        display: { $value: SANS, weights: [700, 800] },
        body: { $value: TEXT, weights: [400, 600] },
        mono: { $value: MONO },
      },
      type: {
        display: { size: 64, line: 72, weight: 800 },
        h2: { size: 40, line: 48, weight: 700 },
        body: { size: 17, line: 28, weight: 400 },
      },
      radius: { sm: { $value: 8 }, md: { $value: 14 }, lg: { $value: 24 }, pill: { $value: 999 } },
      space: space(),
      logos: logos(),
      imagery: {
        summary: 'Product renders and bold type over stock photography.',
        do: ['Product renders with soft light', 'Bold type next to the card'],
        dont: ['Stock photos of people shopping'],
      },
      voice: {
        summary: 'Warm, confident, never salesy.',
        do: ['Earn on every purchase, spend anywhere.', 'Reach Gold in about 6 weeks.', 'Join free'],
        dont: ['Unlock INCREDIBLE savings NOW!!!', 'Become an elite VIP insider today.', 'Submit'],
      },
    }),
  },
  {
    id: 'editorial',
    label: 'Editorial',
    blurb: 'Deep teal and coral with a serif display face. Calm, readable, trustworthy.',
    build: (name) => ({
      $schema: 'otto-brand/1',
      name,
      color: {
        primary: { $value: '#0F766E', $description: 'Buttons and links' },
        accent: { $value: '#F97360', $description: 'Illustrations and highlights' },
        ink: { $value: '#10201F', $description: 'Body text' },
        surface: { $value: '#FFFFFF' },
        'surface-alt': { $value: '#F2FAF8' },
        success: { $value: '#15803D' },
      },
      font: {
        display: { $value: SERIF, weights: [600, 700] },
        body: { $value: TEXT, weights: [400, 600] },
        mono: { $value: MONO },
      },
      type: {
        display: { size: 56, line: 64, weight: 700 },
        h2: { size: 34, line: 42, weight: 600 },
        body: { size: 18, line: 30, weight: 400 },
      },
      radius: { sm: { $value: 4 }, md: { $value: 8 }, lg: { $value: 16 }, pill: { $value: 999 } },
      space: space(),
      logos: logos(),
      imagery: {
        summary: 'Natural light and real places; illustration for abstract ideas.',
        do: ['Photos in natural light', 'Simple two-colour illustration'],
        dont: ['Heavy filters or neon gradients'],
      },
      voice: {
        summary: 'Clear, calm and helpful.',
        do: ['Here is what changes for you.', 'It takes about two minutes.'],
        dont: ['Revolutionary, game-changing experience!', 'Click here'],
      },
    }),
  },
  {
    id: 'minimal',
    label: 'Minimal',
    blurb: 'Graphite and one blue. Tight radius, quiet type. For tools and dashboards.',
    build: (name) => ({
      $schema: 'otto-brand/1',
      name,
      color: {
        primary: { $value: '#2563EB', $description: 'The one accent: actions and focus' },
        accent: { $value: '#1D1D1F', $description: 'Dark fills and headlines' },
        ink: { $value: '#111114', $description: 'Body text' },
        surface: { $value: '#FFFFFF' },
        'surface-alt': { $value: '#F5F5F7' },
        success: { $value: '#1F7A45' },
      },
      font: {
        display: { $value: SANS, weights: [600, 700] },
        body: { $value: TEXT, weights: [400, 500] },
        mono: { $value: MONO },
      },
      type: {
        display: { size: 48, line: 56, weight: 700 },
        h2: { size: 28, line: 36, weight: 600 },
        body: { size: 15, line: 24, weight: 400 },
      },
      radius: { sm: { $value: 4 }, md: { $value: 6 }, lg: { $value: 10 }, pill: { $value: 999 } },
      space: space(),
      logos: logos(),
      imagery: {
        summary: 'Screenshots and diagrams, never decoration.',
        do: ['Real product screenshots', 'Simple line diagrams'],
        dont: ['Abstract 3D blobs'],
      },
      voice: {
        summary: 'Precise and plain-spoken.',
        do: ['Deploys finish in about 40 seconds.', 'Save'],
        dont: ['Supercharge your workflow!', 'Submit'],
      },
    }),
  },
];

export function starterKit(id: string): StarterKit {
  return STARTER_KITS.find((k) => k.id === id) ?? STARTER_KITS[0];
}
