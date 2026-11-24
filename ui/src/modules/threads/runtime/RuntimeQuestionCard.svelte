<script lang="ts">
  import { SvelteMap } from 'svelte/reactivity';
  import type { QuestionAnswer, RuntimeRequest } from '../../../lib/api/types';
  import { offersOther, otherAnswer, questionAnswerError, questionOutcome } from './questions';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { announce } from '../../../lib/announce.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { openExternal } from '../../../lib/external';
  import { urlToOpen } from './questions';

  // `docked`: the live form in the attention dock — its fields scroll inside
  // a bounded body and Send answer stays in view at the bottom (never below
  // the fold). Otherwise the settled record ("Previous questions").
  let { request, sessionId, agent, onanswered, docked = false }: { request: RuntimeRequest; sessionId: string; agent: string; onanswered: () => void; docked?: boolean } = $props();
  const answers = new SvelteMap<string, QuestionAnswer>();
  const other = new SvelteMap<string, boolean>();
  let busy = $state(false);
  let error = $state<string | null>(null);
  const live = $derived(request.status === 'pending' && request.response_capability.type === 'live');
  const questions = $derived(request.questions ?? []);
  function initializeEmptyAnswers(): void {
    for (const q of questions) {
      if (q.allow_empty && !answers.has(q.id)) {
        if (q.kind === 'text') answers.set(q.id, { question_id: q.id, type: 'text', text: '' });
        if (q.kind === 'multi') answers.set(q.id, { question_id: q.id, type: 'multi', values: [] });
      }
    }
  }
  $effect(initializeEmptyAnswers);
  function toggle(id: string, value: string, checked: boolean): void {
    const answer = answers.get(id);
    const values = answer?.type === 'multi' ? answer.values : [];
    answers.set(id, { question_id: id, type: 'multi', values: checked ? [...values, value] : values.filter((v) => v !== value) });
  }
  async function dismiss(action: 'decline' | 'cancel'): Promise<void> {
    if (busy || !live) return;
    busy = true; error = null;
    try {
      await runtimeThreads.dismissQuestion(sessionId, request.id, action);
      announce('Question dismissed'); onanswered();
    } catch (e) { error = `Couldn’t dismiss the question. ${loadErrorText(e)} Try again.`; }
    finally { busy = false; }
  }
  async function send(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || !live) return;
    const values = [...answers.values()];
    error = questionAnswerError(questions, values);
    if (error) return;
    busy = true;
    // An agent's "open this page" (ACP url mode): only a question Otto's
    // driver marked (`open_url`), only when the person chose its open option,
    // and exactly the URL the card shows.
    const link = urlToOpen(questions, values);
    if (link) void openExternal(link);
    try {
      await runtimeThreads.respondQuestion(sessionId, request.id, values);
      announce('Answer sent');
      onanswered();
    } catch (e) {
      error = `Couldn’t send the answer. ${loadErrorText(e)} Try again.`;
    } finally {
      busy = false;
    }
  }
</script>

<form class="question-card" class:docked onsubmit={send} data-testid="runtime-question-card">
  {#if !docked}<p class="question-by">{agent} · {questionOutcome(request)}</p>{/if}
  <div class="qc-body">
  {#if request.question_title}<p class="question-title" dir="auto">{request.question_title}</p>{/if}
  {#each questions as q, index (q.id)}
    <fieldset disabled={!live || busy}>
      <legend dir="auto">{q.header}{q.required ? '' : ' (optional)'}</legend>
      <p class="question-prompt" dir="auto">{q.prompt}</p>
      {#if live}
        {#each q.options as option (option.value)}
          <label class="question-option">
            <input type={q.kind === 'multi' ? 'checkbox' : 'radio'} name={`${request.id}-${index}`}
              checked={!other.get(q.id) && (answers.get(q.id)?.type === 'single' ? (answers.get(q.id) as Extract<QuestionAnswer, { type: 'single' }>).value === option.value : answers.get(q.id)?.type === 'multi' && (answers.get(q.id) as Extract<QuestionAnswer, { type: 'multi' }>).values.includes(option.value))}
              onchange={(e) => { other.set(q.id, false); if (q.kind === 'multi') toggle(q.id, option.value, e.currentTarget.checked); else answers.set(q.id, { question_id: q.id, type: 'single', value: option.value }); }} />
            <span dir="auto">{option.label}{#if option.description}<small>{option.description}</small>{/if}</span>
          </label>
        {/each}
        {#if offersOther(q)}
          <label class="question-option"><input type="radio" name={`${request.id}-${index}`} checked={other.get(q.id) === true}
            onchange={() => { other.set(q.id, true); answers.set(q.id, otherAnswer(q.id, '')); }} data-testid="runtime-question-other" />Other</label>
        {/if}
        {#if q.kind === 'text' || other.get(q.id)}
          <label class="question-text">Your answer
            <textarea rows="2" dir="auto" value={answers.get(q.id)?.type === 'text' ? (answers.get(q.id) as Extract<QuestionAnswer, { type: 'text' }>).text : ''}
              oninput={(e) => { answers.set(q.id, otherAnswer(q.id, e.currentTarget.value)); }}></textarea>
          </label>
        {/if}
      {:else}
        {@const answer = request.answers?.find((a) => a.question_id === q.id)}
        {#if answer}<p dir="auto">{answer.type === 'text' ? answer.text : answer.type === 'single' ? answer.value : answer.values.join(', ')}</p>{/if}
      {/if}
    </fieldset>
  {/each}
  </div>
  <p class="question-error" role="alert">{error ?? ''}</p>
  {#if live}
    <div class="qc-foot">
      <button class="btn qc-btn" type="button" disabled={busy} onclick={() => void dismiss('decline')}>Skip question</button>
      <button class="btn primary qc-btn" type="submit" disabled={busy} data-testid="runtime-question-send">{busy ? 'Sending…' : 'Send answer'}</button>
    </div>
  {/if}
</form>

<style>
  .question-card { padding: 12px; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); min-width: 0; }
  .question-by { margin: 0 0 8px; font-size: var(--fs-xs); color: var(--text-dim); }
  fieldset { border: 0; padding: 0; margin: 0 0 12px; min-width: 0; }
  legend { font-size: var(--fs-s); font-weight: 600; }
  .question-prompt { margin: 4px 0 8px; overflow-wrap: anywhere; }
  .question-option { display: flex; align-items: start; gap: 8px; padding: 6px 0; min-height: var(--hit-min); font-size: var(--fs-s); overflow-wrap: anywhere; }
  .question-option input { margin-block-start: 4px; flex-shrink: 0; }
  .question-option small { display: block; color: var(--text-dim); font-size: var(--fs-xs); }
  .question-text { display: grid; gap: 4px; font-size: var(--fs-s); }
  textarea { resize: vertical; width: 100%; box-sizing: border-box; min-height: 56px; }
  .question-error:empty { margin: 0; }
  .question-error { color: var(--danger); font-size: var(--fs-s); }
  .question-title { margin: 0 0 8px; font-weight: 600; overflow-wrap: anywhere; }
  .qc-foot { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
  /* Docked: the card is the dock's body — no border of its own, fields
     scroll, the actions stay put. */
  .question-card.docked { display: flex; flex-direction: column; flex: 1 1 auto; min-block-size: 0; padding: 0; border: 0; background: transparent; }
  .docked .qc-body { flex: 1 1 auto; min-block-size: 0; overflow-y: auto; overscroll-behavior: contain; padding-inline-end: 2px; }
  .docked .qc-foot { flex: none; padding-block-start: 6px; border-block-start: 1px solid var(--border); }
  .docked .question-error:not(:empty) { flex: none; margin: 4px 0 0; }
  @media (pointer: coarse) { .qc-btn { min-block-size: calc(var(--hit-min) + 8px); padding-inline: 14px; } }
</style>
