// The start-a-run form (pure; ux.md §7): its fields come from the Start
// step's declared `inputs`, else the run-input keys the chat trigger fills
// (pr, jira_ticket, pr_branch, base, goals, msg). Empty optional fields are
// left out, goals become a list and the message is also the prompt — the
// same keys a chat message with the same values gives the run. JSON is the
// escape hatch: keys without a field live there only.
import { findPlaceholder } from '../runInput.ts';

type Input = Record<string, unknown>;

export interface StartField {
  key: string;
  label: string;
  required?: boolean;
  multiline?: boolean;
  hint?: string;
  /** One value per line → a list (goals). */
  lines?: boolean;
  placeholder?: string;
}

/** The run-input keys the exported workflows use, in the chat message's order. */
export const DEFAULT_FIELDS: readonly StartField[] = [
  { key: 'pr', label: 'Pull request', placeholder: '#412, a link, or repo #412' },
  { key: 'jira_ticket', label: 'Jira ticket', placeholder: 'PROJ-123' },
  { key: 'pr_branch', label: 'Branch', placeholder: 'The PR’s source branch' },
  { key: 'base', label: 'Base branch', placeholder: 'main' },
  { key: 'goals', label: 'Goals', multiline: true, lines: true, hint: 'one per line' },
  { key: 'msg', label: 'Message to the agents', placeholder: 'Anything the agents should know' },
];

const KEY = /^[a-z][a-z0-9_]{0,63}$/;

/** The form's fields for a Start step's params: its declared inputs, else the
 *  defaults (+ "Working folder" when the Start step fixes one). */
export function startFields(trigger: unknown): StartField[] {
  const p = (trigger && typeof trigger === 'object' ? trigger : {}) as Input;
  const declared = Array.isArray(p.inputs)
    ? (p.inputs as Input[]).filter((f) => typeof f?.key === 'string' && KEY.test(f.key as string))
    : [];
  if (declared.length) {
    return declared.map((f) => ({
      key: f.key as string,
      label: typeof f.label === 'string' && f.label.trim() ? f.label.trim() : (f.key as string),
      ...(f.required === true ? { required: true } : {}),
      ...(f.multiline === true ? { multiline: true } : {}),
      ...(typeof f.hint === 'string' && f.hint.trim() ? { hint: f.hint.trim() } : {}),
    }));
  }
  const fields = [...DEFAULT_FIELDS];
  const wd = typeof p.working_directory === 'string' ? p.working_directory.trim() : '';
  if (wd) fields.push({ key: 'working_directory', label: 'Working folder', placeholder: wd });
  return fields;
}

/** Where a chat-started run reported: offered (never preselected) on Run again. */
export interface ChatOrigin {
  channel: string;
  chat: string;
  thread?: string;
}

/** Keys a run's own start wrote (who, where, the raw message) or its result
 *  routing: dropped when its inputs are reused, so Run again posts to this
 *  page unless the person picks the thread. */
const START_ONLY = new Set([
  'trigger', 'origin_workspace_id', 'channel', 'chat', 'thread', 'user', 'name', 'raw',
  'result_channel', 'result_chat', 'result_thread', 'result_webhook', 'attachments', 'chat_updates',
]);

export function chatOrigin(input: unknown): ChatOrigin | null {
  const o = (input && typeof input === 'object' ? input : {}) as Input;
  const s = (k: string): string => (typeof o[k] === 'string' ? (o[k] as string).trim() : '');
  const chat = s('result_chat') || s('chat');
  const channel = s('result_channel') || s('channel');
  if (!chat || (channel !== 'slack' && channel !== 'telegram')) return null;
  const thread = s('result_chat') ? s('result_thread') : s('thread');
  return { channel, chat, ...(thread ? { thread } : {}) };
}

export interface FormState {
  values: Record<string, string>;
  /** Keys without a field (JSON only). */
  extra: Input;
}

function split(fields: readonly StartField[], input: Input, drop: (k: string) => boolean): FormState {
  const values: Record<string, string> = {};
  const extra: Input = {};
  const keys = new Set(fields.map((f) => f.key));
  for (const [k, v] of Object.entries(input)) {
    if (drop(k) || v === null || v === undefined) continue;
    if (!keys.has(k)) extra[k] = v;
    else values[k] = Array.isArray(v) ? v.map(String).join('\n') : typeof v === 'object' ? JSON.stringify(v) : String(v);
  }
  // The message doubles as the prompt: the prompt is not a key of its own.
  if (keys.has('msg') && typeof extra.prompt === 'string') {
    if (!values.msg) values.msg = extra.prompt;
    if (values.msg === extra.prompt) delete extra.prompt;
  }
  return { values, extra };
}

/** "Run again with these inputs": an earlier run's input on this form. */
export function prefill(fields: readonly StartField[], input: unknown): FormState {
  const o = input && typeof input === 'object' && !Array.isArray(input) ? (input as Input) : {};
  return split(fields, o, (k) => START_ONLY.has(k));
}

/** The JSON field edited: every key it holds, sorted into fields and extra. */
export function fromJson(fields: readonly StartField[], text: string): FormState | { error: string } {
  const t = text.trim();
  if (!t) return { values: {}, extra: {} };
  let parsed: unknown;
  try {
    parsed = JSON.parse(t);
  } catch (e) {
    return { error: `The input isn’t valid JSON (${e instanceof Error ? e.message : 'parse error'}).` };
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { error: 'The input must be a JSON object: { "key": value, … }.' };
  return split(fields, parsed as Input, () => false);
}

/** The run input: extra keys, then the filled fields (empty ones left out),
 *  then where the result goes. */
export function buildInput(fields: readonly StartField[], state: FormState, sendTo: ChatOrigin | null = null): Input {
  const out: Input = { ...state.extra };
  for (const f of fields) {
    const raw = (state.values[f.key] ?? '').trim();
    if (!raw) {
      delete out[f.key];
      continue;
    }
    out[f.key] = f.lines ? raw.split('\n').map((l) => l.trim()).filter(Boolean) : raw;
  }
  if (typeof out.msg === 'string') out.prompt = out.msg;
  if (sendTo) {
    out.result_channel = sendTo.channel;
    out.result_chat = sendTo.chat;
    if (sendTo.thread) out.result_thread = sendTo.thread;
  }
  return out;
}

/** Why Run can't start yet: a required field left empty, or a template value. */
export function startProblem(fields: readonly StartField[], state: FormState, input: Input): string | null {
  const missing = fields.filter((f) => f.required && !(state.values[f.key] ?? '').trim()).map((f) => f.label);
  if (missing.length) return `Fill in ${missing.join(', ')}.`;
  const slot = findPlaceholder(input);
  return slot ? `Replace the placeholder in “${slot}” with a real value, or remove it.` : null;
}
