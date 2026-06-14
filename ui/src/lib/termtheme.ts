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

export function terminalTheme(theme: ThemeName): ITheme {
  switch (theme) {
    case 'pro-dark':
      return {
        ...base,
        background: '#0f0f14',
        foreground: '#dcdce6',
        cursor: '#6c5ce7',
        selectionBackground: 'rgba(108, 92, 231, 0.35)',
        blue: '#7c6cf0',
        brightBlue: '#998cf5',
      };
    case 'warm':
      return {
        ...base,
        background: '#1a1916',
        foreground: '#e8e4da',
        cursor: '#2bb673',
        selectionBackground: 'rgba(43, 182, 115, 0.3)',
        green: '#2bb673',
        blue: '#4f9da6',
      };
    case 'native':
    default:
      return {
        ...base,
        background: '#131318',
        foreground: '#e8e8ee',
        cursor: '#0a84ff',
        selectionBackground: 'rgba(10, 132, 255, 0.35)',
      };
  }
}
