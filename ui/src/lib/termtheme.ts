// Terminal palettes per app theme (xterm theme objects, spec §7.4).

import type { ITheme } from '@xterm/xterm';
import type { ThemeName } from './stores/ui.svelte';

const base: ITheme = {
  black: '#21222c',
  red: '#ff5f57',
  green: '#28c840',
  yellow: '#febc2e',
  blue: '#0a84ff',
  magenta: '#bf5af2',
  cyan: '#5ac8fa',
  white: '#e8e8ee',
  brightBlack: '#5b5b66',
  brightRed: '#ff7a72',
  brightGreen: '#4fd964',
  brightYellow: '#ffd75e',
  brightBlue: '#409cff',
  brightMagenta: '#da8fff',
  brightCyan: '#8fe3ff',
  brightWhite: '#ffffff',
};

// ANSI palette tuned for a light background (darker, higher-contrast colors).
const lightBase: ITheme = {
  black: '#1a1a1a',
  red: '#c41a16',
  green: '#0f8a16',
  yellow: '#a86500',
  blue: '#0a64c8',
  magenta: '#9b1ea0',
  cyan: '#0087a8',
  white: '#3a3a44',
  brightBlack: '#6a6a76',
  brightRed: '#d70000',
  brightGreen: '#16a016',
  brightYellow: '#b87800',
  brightBlue: '#1478e6',
  brightMagenta: '#bf20c0',
  brightCyan: '#0098c0',
  brightWhite: '#1a1a1a',
};

export function terminalTheme(theme: ThemeName, scheme: 'light' | 'dark' = 'dark'): ITheme {
  // Light scheme: one light palette for all themes, accent-tinted cursor/selection.
  if (scheme === 'light') {
    const accent = theme === 'pro-dark' ? '#6c5ce7' : theme === 'warm' ? '#2bb673' : '#0a64c8';
    const background = '#fbfbfd';
    return {
      ...lightBase,
      background,
      foreground: '#1d1d22',
      cursor: accent,
      // Text under the cursor (block fallback) must match the pane bg — a
      // mismatched accent leaves opaque "ghost" cells when the cursor moves.
      cursorAccent: background,
      selectionBackground: 'rgba(10, 100, 200, 0.2)',
      blue: accent,
    };
  }
  switch (theme) {
    case 'pro-dark': {
      const background = '#0f0f14';
      return {
        ...base,
        background,
        foreground: '#dcdce6',
        cursor: '#6c5ce7',
        cursorAccent: background,
        selectionBackground: 'rgba(108, 92, 231, 0.35)',
        blue: '#7c6cf0',
        brightBlue: '#998cf5',
      };
    }
    case 'warm': {
      const background = '#1a1916';
      return {
        ...base,
        background,
        foreground: '#e8e4da',
        cursor: '#2bb673',
        cursorAccent: background,
        selectionBackground: 'rgba(43, 182, 115, 0.3)',
        green: '#2bb673',
        blue: '#4f9da6',
      };
    }
    case 'native':
    default: {
      const background = '#131318';
      return {
        ...base,
        background,
        foreground: '#e8e8ee',
        cursor: '#0a84ff',
        cursorAccent: background,
        selectionBackground: 'rgba(10, 132, 255, 0.35)',
      };
    }
  }
}
