# Otto walkthrough films (Remotion)

The current production is **eight original narrated films**, organized around
complete workflows, with 33 chapters covering the current built-in sidebar.
Each film includes original music, interface sounds, readable demonstrations,
and timed captions. The combined film is 12 minutes 30 seconds.

Start with [the production guide](studio-v2/README.md) for the series,
prerequisites, render commands, audio provenance, checks, and release setup.
[Read the complete transcript](studio-v2/TRANSCRIPT.md).

```bash
npm ci
npm run check
npm run soundtrack
npm run studio
npm run stills
npm run render-all
npm run package:films
npm run screening
```

The screening room opens at `http://127.0.0.1:8780`. Media outputs live in
`out-v2/`. The in-app player uses the new edition when
`VITE_WALKTHROUGHS_V2_BASE` points at a host containing those files.

The previous published edition is preserved in `src/` and `out/`; its
[original maintenance notes](README.legacy.md) are retained for compatibility.
Use `npm run studio:legacy` or `npm run render-all:legacy` for that edition.
No new production script publishes or changes installed/remote state.
