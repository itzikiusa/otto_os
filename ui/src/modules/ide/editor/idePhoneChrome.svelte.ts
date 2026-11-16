// The phone IDE's chrome state, reactive (the decision itself: idePhoneChromeModel.ts).
// IdePage reports where focus is; the keyboard comes from the shell's one source of
// truth (lib/stores/keyboard.svelte.ts ← lib/viewportFit.ts).

import { keyboard } from '../../../lib/stores/keyboard.svelte';
import { ideLayout, STACK_MAX_H, STACK_MAX_W } from '../shell/ideLayout.svelte';
import { idePhoneChromeFor, type PhoneChrome } from './idePhoneChromeModel';

/** Focus targets that mean "typing": the code and the agent composer. */
export const IDE_TYPING_SELECTOR = '.cm-editor, [data-testid="ide-agent-composer"], .ide-agent-wrap textarea';

class IdePhoneChrome {
  /** Focus is in a typing target (IdePage's focusin/focusout). */
  typing = $state(false);

  get current(): PhoneChrome {
    return idePhoneChromeFor({
      stack: ideLayout.stack,
      pageH: ideLayout.height,
      short: ideLayout.height > 0 && ideLayout.height < STACK_MAX_H,
      wide: ideLayout.width >= STACK_MAX_W,
      typing: this.typing,
      kbOpen: keyboard.open,
      visibleH: keyboard.visibleHeight,
    });
  }
}

export const idePhoneChrome = new IdePhoneChrome();
