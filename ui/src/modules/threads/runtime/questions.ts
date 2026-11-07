import type { QuestionAnswer, RuntimeQuestion, RuntimeRequest } from '../../../lib/api/types';

export function pendingQuestions(requests: RuntimeRequest[]): RuntimeRequest[] {
  return requests.filter((r) => r.kind === 'question' && r.status === 'pending' && r.response_capability.type === 'live');
}

export function questionAnswerError(questions: RuntimeQuestion[], answers: QuestionAnswer[]): string | null {
  const seen = new Set<string>();
  for (const answer of answers) {
    if (seen.has(answer.question_id)) return 'Duplicate question answer.';
    seen.add(answer.question_id);
    const q = questions.find((q) => q.id === answer.question_id);
    if (!q) return 'Unknown question. Refresh the thread and try again.';
    if (answer.type === 'text') {
      if (!q.allow_free_text && q.kind !== 'text') return 'Choose an offered answer.';
      if (q.required && !q.allow_empty && !answer.text.trim()) return 'Answer every required question.';
      continue;
    }
    if (q.kind !== answer.type) return q.kind === 'single' ? 'Select one answer.' : 'Choose answers for this question.';
    const values = answer.type === 'single' ? [answer.value] : answer.values;
    if (q.required && !q.allow_empty && values.length === 0) return 'Answer every required question.';
    if (new Set(values).size !== values.length) return 'Duplicate selection.';
    if (values.some((v) => !q.options.some((o) => o.value === v))) return 'Choose an offered answer.';
  }
  return questions.some((q) => q.required && !seen.has(q.id)) ? 'Answer every required question.' : null;
}

export function questionOutcome(request: RuntimeRequest): string {
  switch (request.status) {
    case 'resolved': return 'Answered';
    case 'expired': return 'Expired after restart — ask the agent again';
    case 'cancelled': return request.cancel_reason === 'agent' ? 'Withdrawn by the agent' : 'Canceled';
    default: return 'Needs your answer';
  }
}
