// Cross-module hand-off into the DB Explorer's "Run on…" sheet.
//
// The Workbench's "Send to → Database — Run on…" parks a script + its
// placeholder values here and navigates to the Explorer; QueryEditor consumes
// it as soon as a connection is queryable: the script opens in a NEW query tab
// (never replacing the user's buffer), the values become the tab's variables
// (a comma/newline list = a sweep, e.g. `brand` = `1,2,3,4`) and the Run on…
// sheet opens on the setup stage. Nothing runs until the user previews and
// confirms in the sheet.

export interface DbRunOnPrefill {
  statement: string;
  /** Placeholder name → value text (as typed; the sheet splits lists). */
  vars: Record<string, string>;
}

class DbHandoff {
  pending: DbRunOnPrefill | null = $state(null);

  /** Park a prefill; the next queryable QueryEditor takes it (once). */
  runOn(prefill: DbRunOnPrefill): void {
    this.pending = { statement: prefill.statement, vars: { ...prefill.vars } };
  }

  /** Take (and clear) the pending prefill. */
  take(): DbRunOnPrefill | null {
    const p = this.pending;
    this.pending = null;
    return p;
  }
}

export const dbHandoff = new DbHandoff();

/** `number` when every list item is numeric (so `1,2,3,4` renders unquoted),
 *  else `string` — the sheet's default type for that placeholder. */
export function prefillVarType(value: string): 'number' | 'string' {
  const parts = (value.includes('\n') ? value.split('\n') : value.split(','))
    .map((v) => v.trim())
    .filter(Boolean);
  return parts.length > 0 && parts.every((v) => /^-?\d+(\.\d+)?$/.test(v)) ? 'number' : 'string';
}
