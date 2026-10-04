// The one way a failed ACTION is toasted: a "Couldn’t <verb> <object>" title
// as given, the human cause (loadErrorText) as the body — never the raw
// exception, never "Unknown error". Failed LOADS render inline via LoadState.
import { toasts } from './toast.svelte';
import { loadErrorText } from './loadError';

export function toastError(title: string, e: unknown): number {
  return toasts.error(title, loadErrorText(e));
}
