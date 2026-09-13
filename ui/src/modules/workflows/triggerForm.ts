import type { TriggerKind, WorkflowTrigger } from '../../lib/api/types';

export const EVENT_KINDS = [
  ['review_changed', 'Review changed'], ['budget_exceeded', 'Budget exceeded'],
  ['product_changed', 'Product changed'], ['swarm_status', 'Swarm status'],
  ['improvement_run_finished', 'Improvement finished'], ['insight_ready', 'Insight ready'],
] as const;

export function defaultTriggerForm() {
  return { kind: 'schedule' as TriggerKind, cadence: 'interval', everyMin: 60, atTime: '09:00', weekday: 0,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC', cron: '0 9 * * 1-5', prompt: '',
    eventKind: 'review_changed', filter: '{}', chatChannel: 'slack', chatId: '', chatThread: '', chatMentionOnly: false,
    resultChannel: '', resultChat: '', resultThread: '', resultWebhook: '' };
}
export type TriggerForm = ReturnType<typeof defaultTriggerForm>;

export function formFromTrigger(trigger: WorkflowTrigger): TriggerForm {
  const spec = trigger.spec as Record<string, unknown>;
  const form = defaultTriggerForm();
  const fields = { cadence: 'cadence', atTime: 'at', timezone: 'timezone', cron: 'expr', prompt: 'prompt',
    eventKind: 'event_kind', chatChannel: 'channel', chatId: 'chat', chatThread: 'thread',
    resultChannel: 'result_channel', resultChat: 'result_chat', resultThread: 'result_thread', resultWebhook: 'result_webhook' } as const;
  for (const [field, key] of Object.entries(fields)) if (typeof spec[key] === 'string') {
    (form as unknown as Record<string, unknown>)[field] = spec[key];
  }
  form.kind = trigger.kind;
  form.timezone = typeof spec.timezone === 'string' ? spec.timezone : 'UTC';
  form.everyMin = typeof spec.every_min === 'number' ? spec.every_min : 60;
  form.weekday = typeof spec.weekday === 'number' ? spec.weekday : 0;
  form.chatMentionOnly = spec.mention_only === true;
  form.filter = JSON.stringify(spec.filter_json ?? {}, null, 2);
  return form;
}

/** Preserve server-owned webhook tokens/cursors and unedited extension fields. */
export function buildTriggerSpec(form: TriggerForm, original: Record<string, unknown> = {}): Record<string, unknown> {
  const spec = { ...original };
  if (form.kind === 'schedule') {
    Object.assign(spec, { cadence: form.cadence, timezone: form.timezone.trim() || 'UTC', enabled: true, prompt: form.prompt });
    if (form.cadence === 'interval') spec.every_min = form.everyMin;
    else if (form.cadence === 'cron') spec.expr = form.cron.trim();
    else { spec.at = form.atTime; if (form.cadence === 'weekly') spec.weekday = form.weekday; }
  } else if (form.kind === 'event') {
    const filter: unknown = JSON.parse(form.filter || '{}');
    if (!filter || typeof filter !== 'object' || Array.isArray(filter)) throw new Error('Event filter must be a JSON object of field/value pairs.');
    Object.assign(spec, { event_kind: form.eventKind, filter_json: filter });
  } else if (form.kind === 'chat') {
    Object.assign(spec, { channel: form.chatChannel, chat: form.chatId.trim(), thread: form.chatThread.trim(),
      mention_only: form.chatChannel === 'slack' && form.chatMentionOnly });
  }
  Object.assign(spec, { result_channel: form.resultChannel, result_chat: form.resultChat.trim(),
    result_thread: form.resultThread.trim(), result_webhook: form.resultWebhook.trim() });
  return spec;
}
