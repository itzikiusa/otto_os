import type { FileDiff, PrComment } from '../../../lib/api/types';

/** Context is separate from the person's draft and only assembled on send. */
export interface ComposerContext {
  id: string;
  kind: 'review-comment' | 'diff';
  label: string;
  text: string;
  url?: string;
  path?: string;
  author?: string;
}

export function addComposerContext(current: readonly ComposerContext[], added: ComposerContext): ComposerContext[] {
  const index = current.findIndex((c) => c.id === added.id);
  if (index < 0) return [...current, added];
  return current.map((c, i) => i === index ? added : c);
}

export function assembleComposerMessage(text: string, context: readonly ComposerContext[]): string {
  if (!context.length) return text;
  return [text, ...context.map((c) => `--- ${c.kind === 'diff' ? 'Diff context' : 'Review conversation'}: ${c.label} ---\n${c.text}`)].filter(Boolean).join('\n\n');
}

export function reviewCommentContext(repoId: string, prUrl: string, comment: PrComment): ComposerContext {
  const location = comment.path ? `${comment.path}${comment.line != null ? `:${comment.line}` : ''}` : 'General comment';
  function conversation(c: PrComment): string {
    const anchor = c.path ? ` (${c.path}${c.line != null ? `:${c.line}` : ''}${c.side === 'old' ? ', old side' : ''})` : '';
    return [`${c.author}${anchor}:\n${c.body}`, ...c.replies.map(conversation)].join('\n\n');
  }
  return {
    id: `review:${repoId}:${prUrl}:${comment.id}`, kind: 'review-comment',
    label: `${comment.author} · ${location}`, author: comment.author, url: prUrl,
    ...(comment.path ? { path: comment.path } : {}),
    text: `Pull request: ${prUrl}\nComment: ${comment.id}\nLocation: ${location}${comment.side === 'old' ? ' (old side)' : ''}${comment.outdated ? ' (outdated)' : ''}\n${comment.resolved ? 'Resolved conversation\n' : ''}\n${conversation(comment)}`,
  };
}

/** Keep the displayed selection, including old/new numbering for deleted lines. */
export function diffContext(repoId: string, file: FileDiff, selection?: { hunk: number; line?: number }, target = 'working'): ComposerContext {
  const hunk = selection ? file.hunks[selection.hunk] : null;
  const line = selection?.line != null ? hunk?.lines[selection.line] : null;
  const location = line ? `${file.path}:${line.new_line ?? line.old_line ?? ''}` : file.path;
  const hunks = selection ? (hunk ? [hunk] : []) : file.hunks;
  const body = hunks.map((h) => [h.header, ...(line ? [line] : h.lines).map((l) => {
    const nums = [l.old_line != null ? `old ${l.old_line}` : '', l.new_line != null ? `new ${l.new_line}` : ''].filter(Boolean).join(', ');
    return `[${nums}] ${l.origin === 'add' ? '+' : l.origin === 'del' ? '-' : ' '}${l.content.replace(/\n$/, '')}`;
  })].join('\n')).join('\n');
  return {
    id: `diff:${repoId}:${target}:${file.path}:${selection ? `${hunk?.header ?? selection.hunk}:${line ? `${line.origin}:${line.old_line}:${line.new_line}` : 'hunk'}` : 'file'}`,
    kind: 'diff', label: `${location}${selection && !line ? ' · hunk' : ''}`, path: file.path,
    text: `Repository: ${repoId}\nDiff: ${target === 'working' ? 'Working tree (uncommitted)' : target}\nFile: ${file.path}${file.old_path ? `\nPrevious path: ${file.old_path}` : ''}\n\n${body}`,
  };
}
