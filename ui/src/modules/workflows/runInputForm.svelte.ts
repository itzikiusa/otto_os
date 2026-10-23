// The Run… panel's form (W2): the free-text prompt, the JSON run input the
// Start trigger emits, and the per-run review-mode override. Lifted out of
// WorkflowsPage with RunInputPanel.svelte (its markup); the page owns when a
// run starts (`execRun`) and builds the request from `request()`.

import type { ReviewMode, RunWorkflowReq } from '../../lib/api/types';
import { findPlaceholder, resultDestinations, runInputExample, runInputSkeleton } from './runInput';

export class RunInputForm {
  open = $state(false);
  // Prompt box shown above the JSON textarea in the run popover — merged into
  // the parsed input as `prompt` (non-empty only) on run.
  prompt = $state('');
  // Per-run review-mode override, posted as `review_mode` only when set. Wins
  // over every review_run node's own `params.mode` for this run.
  reviewMode = $state<'' | ReviewMode>('');
  // Invalid run-input JSON is a field error: said inline under the field (the
  // panel opens if a "Run from here" found it), never as a toast. Editing the
  // input clears it.
  error = $state('');
  #text = $state('');
  get text(): string {
    return this.#text;
  }
  set text(v: string) {
    this.#text = v;
    this.error = '';
  }

  /** Every node kind in the graph (the template depends on it). */
  private readonly kinds: () => Set<string>;
  constructor(kinds: () => Set<string>) {
    this.kinds = kinds;
  }

  // The run-input template is guidance only (W2): the full annotated example
  // is the textarea placeholder; "Suggest" inserts just the `<…>` slots this
  // graph needs. Placeholder values are rejected before the run is sent.
  placeholder = $derived.by(() => runInputExample(this.kinds()));
  destinations = $derived.by(() => {
    try {
      return resultDestinations(JSON.parse(this.#text));
    } catch {
      return [];
    }
  });

  suggest(): void {
    this.text = runInputSkeleton(this.kinds());
  }

  /** The parsed input: undefined for an empty field, null when it is invalid
   *  (the reason is in `error`, and the panel opens to show it). */
  parse(): Record<string, unknown> | undefined | null {
    const t = this.#text.trim();
    if (!t) return undefined;
    try {
      const parsed = JSON.parse(t);
      const slot = findPlaceholder(parsed);
      if (slot) {
        this.error = `Replace the placeholder in \`${slot}\` with a real value, or remove the key.`;
        this.open = true;
        return null;
      }
      return parsed;
    } catch (e) {
      this.error = `Run input isn’t valid JSON (${e instanceof Error ? e.message : 'parse error'}). Fix it, or clear the field to run with no input.`;
      this.open = true;
      return null; // signal: invalid
    }
  }

  /** Merge the Prompt box into the parsed run input as `prompt`, only when
   *  non-empty — leaves `input` untouched otherwise (including `undefined`,
   *  which still means "run with no input"). */
  mergePrompt(input: Record<string, unknown> | undefined): Record<string, unknown> | undefined {
    const p = this.prompt.trim();
    if (!p) return input;
    return { ...(input ?? {}), prompt: p };
  }

  /** The run request for this form (plus `extra`), or null when the input is
   *  invalid. */
  request(extra: Partial<RunWorkflowReq> = {}): RunWorkflowReq | null {
    const input = this.parse();
    if (input === null) return null;
    const merged = this.mergePrompt(input);
    return {
      ...extra,
      ...(merged !== undefined ? { input: merged } : {}),
      ...(this.reviewMode ? { review_mode: this.reviewMode } : {}),
    };
  }
}
