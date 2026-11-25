# otto-debug

Bundled Otto skill (Skills Lab → Skills → Bundled → Install). It teaches an agent to debug in
Otto's IDE when the user asks for it: breakpoints at decision points, one start the user confirms,
an explanation of at most three sentences at each stop, then waiting for "proceed".

It needs Otto's agent UI control (the per-session Allow); the `otto_ui_ide_debug_*` tools work
without the skill, the skill only makes their use consistent. See `references/tools.md` and
`examples/session.md`.
