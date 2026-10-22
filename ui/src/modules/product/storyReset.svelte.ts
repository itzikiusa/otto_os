// "Reset this tab when the story changes" for the Product tabs that also run
// a result poller (Rewrite, Plan, Test cases). The effect depends on the story
// id ONLY: the reset and the teardown run untracked. Calling `clearPoll()` in
// a tracked effect body used to read the tab's `$state.raw` poller, so the
// `startPolling()` write from Generate re-ran the effect — its teardown
// stopped the poller just started, re-enabled Generate and wiped the tab.
import { untrack } from 'svelte';

/** Inside a component's init: run `reset(id)` now and whenever `storyId()`
 *  changes, and `teardown()` before each re-run and on unmount. Nothing
 *  either callback reads becomes a dependency. */
export function onStoryChange(
  storyId: () => string | null,
  reset: (id: string | null) => void,
  teardown: () => void,
): void {
  $effect(() => {
    const id = storyId();
    untrack(() => reset(id));
    return () => untrack(teardown);
  });
}
