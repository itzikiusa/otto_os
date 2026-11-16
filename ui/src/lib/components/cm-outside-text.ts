// Marks a transaction whose text came from OUTSIDE the editor's own typing: a reload from
// disk, the store applying a value (an agent proposal, a reformat), a collab peer.
//
// Not `Transaction.remote`: CodeMirror's history leaves remote changes out of the local undo
// stack, and ⌘Z must still undo an applied proposal or a reformat. This annotation only tells
// position-keeping extensions (the IDE's breakpoints) not to carry their marks along.

import { Annotation } from '@codemirror/state';

export const outsideText = Annotation.define<boolean>();
