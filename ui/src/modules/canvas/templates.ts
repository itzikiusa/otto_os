// Starter-scene gallery for the empty-state hero. Each template builds a fresh,
// fully-formed `Scene` (new ids every call via `makeNode`/`genId`) so a user can
// drop a real diagram on the canvas in one click instead of staring at a blank
// page. Kept pure — no Svelte state, no side effects.

import { genId, makeNode } from './scene';
import type { CanvasEdge, Scene } from './types';

export interface Template {
  id: string;
  name: string;
  /** One-line description for the gallery card. */
  hint: string;
  /** Icon name (must exist in Icon.svelte). */
  icon: string;
  build: () => Scene;
}

/** A bare scene shell with grid on. */
function shell(title: string): Scene {
  return { schema: 1, title, nodes: [], edges: [], slides: [], appState: { grid: true } };
}

/** Convenience arrow edge between two node ids. */
function arrow(source: string, target: string, label?: string): CanvasEdge {
  return { id: genId('e'), source, target, label, kind: 'arrow' };
}

export const TEMPLATES: Template[] = [
  // 1 — Sequence diagram (one mermaid node). The canonical "service A calls B".
  {
    id: 'sequence',
    name: 'Sequence diagram',
    hint: 'Actors exchanging messages over time',
    icon: 'send',
    build() {
      const s = shell('Sequence diagram');
      const m = makeNode('mermaid', 80, 80);
      m.mermaid = {
        src: [
          'sequenceDiagram',
          '  participant Client',
          '  participant API',
          '  participant DB',
          '  Client->>API: request',
          '  API->>DB: query',
          '  DB-->>API: rows',
          '  API-->>Client: response',
        ].join('\n'),
        kind: 'sequence',
      };
      s.nodes = [m];
      // One slide that steps through the messages of this sequence node.
      s.slides = [
        {
          id: genId('slide'),
          title: 'Sequence',
          mermaidNodeId: m.id,
          reveal: [{ nodeIds: [m.id] }],
        },
      ];
      return s;
    },
  },

  // 2 — Flowchart (one mermaid node).
  {
    id: 'flowchart',
    name: 'Flowchart',
    hint: 'Decisions and steps as a graph',
    icon: 'branch',
    build() {
      const s = shell('Flowchart');
      const m = makeNode('mermaid', 80, 80);
      m.w = 460;
      m.h = 420;
      m.mermaid = {
        src: [
          'flowchart TD',
          '  Start([Start]) --> Input[/Collect input/]',
          '  Input --> Check{Valid?}',
          '  Check -- yes --> Process[Process]',
          '  Check -- no --> Input',
          '  Process --> Done([Done])',
        ].join('\n'),
        kind: 'flowchart',
      };
      s.nodes = [m];
      return s;
    },
  },

  // 3 — Architecture: a small box-and-arrow diagram with real shape nodes.
  {
    id: 'architecture',
    name: 'Architecture',
    hint: 'Boxes and arrows — services & stores',
    icon: 'box',
    build() {
      const s = shell('Architecture');
      const web = makeNode('shape', 80, 200, { variant: 'roundrect', label: 'Web' });
      const api = makeNode('shape', 320, 200, { variant: 'roundrect', label: 'API' });
      const db = makeNode('shape', 560, 120, { variant: 'cylinder', label: 'Database' });
      const cache = makeNode('shape', 560, 300, { variant: 'cylinder', label: 'Cache' });
      api.shape = { variant: 'roundrect', fill: 'var(--accent)' };
      api.label = 'API';
      s.nodes = [web, api, db, cache];
      s.edges = [
        arrow(web.id, api.id, 'HTTP'),
        arrow(api.id, db.id, 'SQL'),
        arrow(api.id, cache.id, 'get/set'),
      ];
      return s;
    },
  },

  // 4 — UML class diagram (mermaid classDiagram node).
  {
    id: 'uml-class',
    name: 'UML class',
    hint: 'Classes, fields and relationships',
    icon: 'grid',
    build() {
      const s = shell('UML class diagram');
      const m = makeNode('mermaid', 80, 80);
      m.w = 480;
      m.h = 400;
      m.mermaid = {
        src: [
          'classDiagram',
          '  class Order {',
          '    +String id',
          '    +Date placedAt',
          '    +total() Money',
          '  }',
          '  class LineItem {',
          '    +String sku',
          '    +int qty',
          '  }',
          '  class Customer {',
          '    +String name',
          '  }',
          '  Customer "1" --> "*" Order : places',
          '  Order "1" *-- "*" LineItem : contains',
        ].join('\n'),
        kind: 'class',
      };
      s.nodes = [m];
      return s;
    },
  },

  // 5 — User journey: a horizontal lane of sticky notes (one step each).
  {
    id: 'journey',
    name: 'User journey',
    hint: 'A row of steps as sticky notes',
    icon: 'note',
    build() {
      const s = shell('User journey');
      const steps = ['Discover', 'Sign up', 'Onboard', 'Activate', 'Retain'];
      const colors = ['#ffe9a8', '#bfe3ff', '#cdeccd', '#ffd6e0', '#e3d4ff'];
      s.nodes = steps.map((label, i) => {
        const n = makeNode('sticky', 60 + i * 200, 140);
        n.sticky = { value: label, color: colors[i % colors.length] };
        return n;
      });
      // A title text above the lane.
      const title = makeNode('text', 60, 60, { label: 'User journey' });
      title.text = { value: 'User journey', align: 'left', size: 22 };
      s.nodes.unshift(title);
      return s;
    },
  },

  // 6 — Blank frame: a single frame + a slide bound to it (a clean slate that's
  // already presentation-ready).
  {
    id: 'blank-frame',
    name: 'Blank frame',
    hint: 'An empty slide frame to fill in',
    icon: 'square',
    build() {
      const s = shell('Untitled scene');
      const frame = makeNode('frame', 80, 80, { label: 'Slide 1' });
      s.nodes = [frame];
      s.slides = [
        {
          id: genId('slide'),
          title: 'Slide 1',
          frameNodeId: frame.id,
          reveal: [{ nodeIds: [frame.id] }],
        },
      ];
      return s;
    },
  },
];
