// One instruction for several threads at once ("Message every pane…",
// "Ask every seat…"): a multi-line sheet that says who receives it and who
// can't (PaneMessageSheet.svelte, mounted by ThreadCanvas). Off the canvas
// (the stage is up) it falls back to the shared one-line prompt sheet.
import { confirmer } from '../../../lib/confirm.svelte';

export interface PaneMessageReq {
  title: string;
  /** What will happen, in words ("3 seats (1 stopped — it will restart)"). */
  intro: string;
  /** Who can't receive it, when anyone. */
  note?: string;
  confirmLabel: string;
  placeholder: string;
}

class PaneMessage {
  req = $state<PaneMessageReq | null>(null);
  /** Mounted sheets (ThreadCanvas). */
  hosts = 0;
  private resolve: ((v: string | null) => void) | null = null;

  /** The text to send (trimmed), or null when cancelled. */
  ask(req: PaneMessageReq): Promise<string | null> {
    if (this.hosts === 0) {
      const msg = req.note ? `${req.intro} ${req.note}` : req.intro;
      return confirmer.promptText(msg, { title: req.title, confirmLabel: req.confirmLabel, placeholder: req.placeholder }).then((t) => t?.trim() || null);
    }
    this.done(null);
    this.req = req;
    return new Promise((r) => (this.resolve = r));
  }

  done(text: string | null): void {
    const r = this.resolve;
    this.resolve = null;
    this.req = null;
    r?.(text?.trim() || null);
  }
}

export const paneMessage = new PaneMessage();
