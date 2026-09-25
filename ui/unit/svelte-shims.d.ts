// Type-only shims for the unit tsconfig (no svelte-check here): pure lib
// modules may `import type` from a component's `<script module>`, which plain
// `tsc` can't resolve. Declare just the names the unit-tested modules use.
declare module '*/components/Icon.svelte' {
  export type IconName = string;
}
