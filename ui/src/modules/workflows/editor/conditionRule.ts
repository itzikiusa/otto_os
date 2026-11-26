// "Condition on run data" (pure; ux.md §9.3, engine.md §7.4): a field, an
// operator and a value written as an expression in the engine's language
// (otto_core::expr) — and read back from one, so a rule written here opens as
// a rule. Anything else stays a free expression ("Write an expression instead").

export type ConditionOp = 'is' | 'is_not' | 'gt' | 'lt' | 'contains' | 'empty';

export interface ConditionRule {
  /** An expression path or call: `len(run.repos)`, `run.input.pr`, `output.score`. */
  field: string;
  op: ConditionOp;
  value: string;
}

export const CONDITION_OPS: readonly { op: ConditionOp; label: string }[] = [
  { op: 'is', label: 'is' },
  { op: 'is_not', label: 'is not' },
  { op: 'gt', label: '>' },
  { op: 'lt', label: '<' },
  { op: 'contains', label: 'contains' },
  { op: 'empty', label: 'is empty' },
];

const SYMBOL: Record<Exclude<ConditionOp, 'empty'>, string> = { is: '==', is_not: '!=', gt: '>', lt: '<', contains: 'contains' };

/** A typed value as an expression literal: numbers and true/false/null as they are, the rest quoted. */
function literal(v: string): string {
  const t = v.trim();
  if (/^-?\d+(\.\d+)?$/.test(t) || t === 'true' || t === 'false' || t === 'null') return t;
  return JSON.stringify(t);
}

export function ruleExpr(r: ConditionRule): string {
  if (!r.field.trim()) return '';
  return r.op === 'empty' ? `!${r.field.trim()}` : `${r.field.trim()} ${SYMBOL[r.op]} ${literal(r.value)}`;
}

const FIELD = String.raw`(len\(run\.repos\)|[A-Za-z_][\w]*(?:\.[A-Za-z_][\w]*)*)`;

/** The rule an expression was written from, or null (a free expression). */
export function parseRule(expr: string): ConditionRule | null {
  const t = expr.trim();
  const empty = new RegExp(`^!${FIELD}$`).exec(t);
  if (empty) return { field: empty[1], op: 'empty', value: '' };
  const m = new RegExp(`^${FIELD} (==|!=|>|<|contains) (-?\\d+(?:\\.\\d+)?|true|false|null|"(?:[^"\\\\]|\\\\.)*")$`).exec(t);
  if (!m) return null;
  const op = (Object.entries(SYMBOL).find(([, s]) => s === m[2])![0]) as ConditionOp;
  const value = m[3].startsWith('"') ? (JSON.parse(m[3]) as string) : m[3];
  return { field: m[1], op, value };
}

/** The fields the picker offers: the run's repositories, each start input, the previous step's output. */
export function ruleFields(inputKeys: readonly string[]): { field: string; label: string }[] {
  return [
    { field: 'len(run.repos)', label: 'Number of repositories' },
    ...inputKeys.map((k) => ({ field: `run.input.${k}`, label: `Run input · ${k}` })),
    { field: 'output.result', label: 'Previous step · result' },
  ];
}
