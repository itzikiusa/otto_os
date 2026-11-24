// The query editor's actions, for the workbench header (DatabasePage): the
// header carries Run (the one primary) and the one ⋯; QueryEditor owns what
// they do and publishes a fresh snapshot here whenever its state changes
// (withdrawn on unmount).

import type { MenuItem } from '../../lib/contextmenu.svelte';

export interface EditorBar {
  readonly canRun: boolean;
  /** Why Run is disabled, else what it will run. */
  readonly runTitle: string;
  readonly running: boolean;
  /** Statements in the buffer ("Run all 3"). */
  readonly stmtCount: number;
  /** The tab's results are masked (shown as a chip that turns it off). */
  readonly masked: boolean;
  run(): void;
  runAll(): void;
  stop(): void;
  unmask(): void;
  /** The Query group of the header's ⋯ (Run all, Run on…, plan, format,
   *  save, Ask AI, settings, wrap, clipboard, shortcuts). */
  menu(trigger: HTMLElement | null): MenuItem[];
}

export const editorBar = $state<{ cur: EditorBar | null }>({ cur: null });
