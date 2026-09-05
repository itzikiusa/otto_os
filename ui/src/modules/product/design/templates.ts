// Design Arena — Canva-like static starters (design §4.1). Each template is a
// plain file under `./templates/` imported as raw text at build time, so adding
// one is: drop the file, add a row here. `scene3d` starters follow the §2.3
// schema exactly (the viewer validates before rendering).
import type { DesignFormat } from '../types';
import mobileAppScreen from './templates/mobile-app-screen.html?raw';
import dashboard from './templates/dashboard.html?raw';
import landingPage from './templates/landing-page.html?raw';
import userFlow from './templates/user-flow.mmd?raw';
import wireframeBoard from './templates/wireframe-board.excalidraw?raw';
import gameLevelBlockout from './templates/game-level-blockout.scene3d.json?raw';
import productShot from './templates/product-shot.scene3d.json?raw';

export interface DesignTemplate {
  id: string;
  name: string;
  description: string;
  format: DesignFormat;
  /** Suggested file name (the arena de-duplicates against existing rows). */
  filename: string;
  source: string;
}

export const DESIGN_TEMPLATES: readonly DesignTemplate[] = [
  {
    id: 'mobile-app-screen',
    name: 'Mobile app screen',
    description: 'Wallet home: balance card, activity list, tab bar. Pairs with the iPhone frame.',
    format: 'html',
    filename: 'mobile-home.html',
    source: mobileAppScreen,
  },
  {
    id: 'dashboard',
    name: 'Dashboard',
    description: 'Sidebar + KPI row + chart + table, dark. Pairs with the desktop frame.',
    format: 'html',
    filename: 'dashboard.html',
    source: dashboard,
  },
  {
    id: 'landing-page',
    name: 'Landing page',
    description: 'Nav, hero with CTA, logo strip, three features, footer.',
    format: 'html',
    filename: 'landing.html',
    source: landingPage,
  },
  {
    id: 'user-flow',
    name: 'User flow',
    description: 'A Mermaid flowchart with a decision, a happy path and a fallback.',
    format: 'mermaid',
    filename: 'user-flow.mmd',
    source: userFlow,
  },
  {
    id: 'wireframe-board',
    name: 'Wireframe board',
    description: 'Two Excalidraw frames as artboards (mobile + desktop) on an 8-pt grid.',
    format: 'excalidraw',
    filename: 'wireframes.excalidraw',
    source: wireframeBoard,
  },
  {
    id: 'game-level-blockout',
    name: 'Game level blockout',
    description: 'Greybox level: ground, walls, platforms, cover, a goal marker. Metres, y-up.',
    format: 'scene3d',
    filename: 'level-blockout.scene3d.json',
    source: gameLevelBlockout,
  },
  {
    id: 'product-shot',
    name: 'Product shot',
    description: 'Three-point lighting on a plinth with a backdrop sweep — swap the hero for a GLB.',
    format: 'scene3d',
    filename: 'product-shot.scene3d.json',
    source: productShot,
  },
];

/** Blank starters for **New ▾** (no template): the smallest valid document. */
export function blankSource(format: DesignFormat, emptyScene: () => unknown): string {
  switch (format) {
    case 'html':
      return (
        '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n' +
        '<meta name="viewport" content="width=device-width, initial-scale=1">\n' +
        '<title>Screen</title>\n<style>\n  body { margin: 0; font: 15px/1.5 -apple-system, Inter, system-ui, sans-serif; color: #0f172a; background: #fff; }\n' +
        '  main { padding: 24px; }\n</style>\n</head>\n<body>\n<main>\n  <h1>New screen</h1>\n  <p>Describe it to the assistant, or edit the source.</p>\n</main>\n</body>\n</html>\n'
      );
    case 'mermaid':
      return 'flowchart LR\n  A[Start] --> B{Decision}\n  B -- yes --> C[Do the thing]\n  B -- no --> D[Fallback]\n';
    case 'excalidraw':
      return JSON.stringify(
        { type: 'excalidraw', version: 2, source: 'otto', elements: [], appState: { viewBackgroundColor: '#ffffff', gridSize: 8 }, files: {} },
        null,
        2,
      );
    case 'scene3d':
      return JSON.stringify(emptyScene(), null, 2);
  }
}
