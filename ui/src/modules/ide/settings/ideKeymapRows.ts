// Settings › Keymap table. The Standard preset rebinds only the editor keys it lists
// apart from the JetBrains column (cm-ide-keymap.ts, standard branch); every other
// row shows the JetBrains key that is live in both presets. ideIntelFixes.test.ts
// checks each differing Standard key is bound.

export interface KeymapRow {
  action: string;
  category: string;
  jetbrains: string;
  standard: string;
}

export const KEYMAP_ROWS: readonly KeymapRow[] = [
  { action: 'Settings & Project Structure', category: 'IDE', jetbrains: '⌘,', standard: '⌘,' },
  { action: 'Search Everywhere', category: 'Navigate', jetbrains: '⇧⇧', standard: '⇧⇧' },
  { action: 'Replace in Path', category: 'Navigate', jetbrains: '⌘⇧R', standard: '⌘⇧R' },
  { action: 'Recent Files', category: 'Navigate', jetbrains: '⌘E', standard: '⌘E' },
  { action: 'Go to Definition / Usages', category: 'Code', jetbrains: '⌘B', standard: 'F12' },
  { action: 'Find Usages', category: 'Code', jetbrains: '⌥F7', standard: '⇧F12' },
  { action: 'Rename Symbol', category: 'Refactor', jetbrains: '⇧F6', standard: 'F2' },
  { action: 'Quick Fix / Show Intentions', category: 'Code', jetbrains: '⌥⏎', standard: '⌘.' },
  { action: 'Reformat Code', category: 'Code', jetbrains: '⌘⌥L', standard: '⇧⌥F' },
  { action: 'Optimize Imports', category: 'Code', jetbrains: '⌃⌥O / ⌘⌥O', standard: '⌃⌥O / ⌘⌥O' },
  { action: 'Toggle Line Comment', category: 'Editor', jetbrains: '⌘/', standard: '⌘/' },
  { action: 'Duplicate Line / Selection', category: 'Editor', jetbrains: '⌘D', standard: '⇧⌘D' },
  { action: 'Delete Line', category: 'Editor', jetbrains: '⌘⌫', standard: '⇧⌘K' },
  { action: 'File Structure Popup', category: 'Navigate', jetbrains: '⌘F12', standard: '⌘F12' },
  { action: 'Go to Line : Column', category: 'Navigate', jetbrains: '⌘G', standard: '⌘G' },
  { action: 'Navigate Back / Forward', category: 'Navigate', jetbrains: '⌘[ / ⌘]', standard: '⌘[ / ⌘]' },
  { action: 'Toggle AI Agent Panel', category: 'Agent', jetbrains: '⇧⌘L', standard: '⇧⌘L' },
  { action: 'Close Active Editor Tab', category: 'Tabs', jetbrains: '⌘W', standard: '⌘W' },
  { action: 'Reopen Closed Editor Tab', category: 'Tabs', jetbrains: '⌘⇧T', standard: '⌘⇧T' },
  { action: 'Toggle Project Tool Window', category: 'Windows', jetbrains: '⌘1', standard: '⌘1' },
  { action: 'Run / Debug / Problems / Git Dock', category: 'Windows', jetbrains: '⌘4 / ⌘5 / ⌘6 / ⌘9', standard: '⌘4 / ⌘5 / ⌘6 / ⌘9' },
  { action: 'Terminal Tool Window', category: 'Windows', jetbrains: '⌥F12', standard: '⌥F12' },
  { action: 'Run / Debug Selected Target', category: 'Run', jetbrains: '⌃R / ⌃D', standard: '⌃R / ⌃D' },
  { action: 'Stop Active Run or Debug', category: 'Run', jetbrains: '⌘F2', standard: '⌘F2' },
  { action: 'Toggle Line Breakpoint', category: 'Debugger', jetbrains: '⌘F8', standard: '⌘F8' },
  { action: 'Step Over / Step Into / Step Out', category: 'Debugger', jetbrains: 'F8 / F7 / ⇧F8 (⌃⌥↓/→/↑)', standard: 'F8 / F7 / ⇧F8 (⌃⌥↓/→/↑)' },
  { action: 'Resume Program / Run to Cursor', category: 'Debugger', jetbrains: '⌘⌥R / F9 / ⌥F9', standard: '⌘⌥R / F9 / ⌥F9' },
  { action: 'Hard Reload Window', category: 'IDE', jetbrains: '⌘⌃⇧R', standard: '⌘⌃⇧R' },
];
