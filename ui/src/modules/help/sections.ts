// Help's guide text: every `./sections/*.md` file as a raw string (Vite glob,
// eager inside this module) parsed once. This module is ONE lazy chunk — the
// page imports it when Help opens; the rail and ⌘K use ./sectionsIndex.ts.
// The page lists whatever files exist, so a guide added by another branch
// shows up with no code change. The pure parsing lives in ./guide.ts.
import type { GuideSection } from './guide';
import { buildGuides } from './guideList';

const files = import.meta.glob<string>('./sections/*.md', { query: '?raw', import: 'default', eager: true });

/** Every guide with its text, in rail order. */
export const GUIDES: GuideSection[] = buildGuides(files);
