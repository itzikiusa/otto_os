// Opens the Goal form from anywhere a goal starts (the Run… picker's "A
// goal…", ⌘K "New goal…", an old loop's Continue as a goal). GoalHost, mounted
// on the Workflows page and the older-loops page, renders the form.
import type { GoalFields } from './goalForm';

class GoalLauncher {
  /** The form's starting fields while it is open; null when closed. */
  fields = $state.raw<GoalFields | null>(null);
  /** Bumped per open so a second open starts a fresh form. */
  generation = $state(0);

  open(prefill?: GoalFields): void {
    this.fields = prefill ?? null;
    this.generation += 1;
    this.isOpen = true;
  }

  close(): void {
    this.isOpen = false;
    this.fields = null;
  }

  isOpen = $state(false);
}

export const goalLauncher = new GoalLauncher();

/** Open the Goal form, empty or pre-filled. */
export function openGoal(prefill?: GoalFields): void {
  goalLauncher.open(prefill);
}
