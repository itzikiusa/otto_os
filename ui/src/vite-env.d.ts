/// <reference types="vite/client" />

// Build-time env vars the UI reads via `import.meta.env`. Vite only exposes
// `VITE_*`-prefixed vars to the client bundle.
interface ImportMetaEnv {
  /** `'1'` → serve the in-memory mock API instead of talking to ottod. */
  readonly VITE_OTTO_MOCK?: string;
  /**
   * Base URL the Walkthroughs page streams its MP4s from (no trailing slash).
   * Defaults to the rolling `walkthroughs` GitHub release of itzikiusa/otto_os;
   * point it at a mirror/CDN for air-gapped or self-hosted builds.
   */
  readonly VITE_WALKTHROUGHS_BASE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
