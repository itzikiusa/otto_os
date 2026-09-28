// Type-only shims for the unit tsconfig (no svelte-check here): pure lib
// modules may `import type` from a component's `<script module>`, which plain
// `tsc` can't resolve. Declare just the names the unit-tested modules use.
declare module '*/components/Icon.svelte' {
  export type IconName = string;
}
// Once a unit file pulls in svelte's own types (runesHarness imports
// `svelte/compiler`), its global `declare module '*.svelte'` ties with the
// pattern above and wins, so `IconName` vanishes. Merge the name into that
// wildcard too; it's type-only and `string`, like the shim above.
declare module '*.svelte' {
  export type IconName = string;
}
