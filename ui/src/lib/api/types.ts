// TS mirror of crates/otto-core (domain.rs + api.rs). Keep in lockstep — the
// Rust side is the source of truth (docs/contracts/api.md).
//
// Barrel: the contract mirror is split by area under ./types/. Import from
// `lib/api/types` as before; every name resolves through these re-exports.

export * from './types/activity';
export * from './types/api-client';
export * from './types/auth';
export * from './types/automation';
export * from './types/aws';
export * from './types/brokers';
export * from './types/browser';
export * from './types/connections';
export * from './types/core';
export * from './types/database';
export * from './types/design';
export * from './types/design-assist';
export * from './types/events';
export * from './types/git';
export * from './types/git-merge';
export * from './types/goal-loops';
export * from './types/insights-share';
export * from './types/integrations';
export * from './types/k8s';
export * from './types/k8s-monitor';
export * from './types/mcp';
export * from './types/memory';
export * from './types/notifications';
export * from './types/orchestrator';
export * from './types/personal-agents';
export type { ProductStopTurnReq, ProductStopTurnResp, ProductTurnIdentity } from './types/product';
export * from './types/review';
export * from './types/sessions';
export * from './types/skills-context';
export * from './types/skills-eval';
export * from './types/telemetry';
export * from './types/transcript';
export * from './types/ui-control';
export * from './types/vault';
export * from './types/workbench';
export * from './types/workflows';
export * from './types/ws-term';

export type * from './room-types';
export type * from './room-recap-types';
export type * from './db-multirun-types';
export type * from './maintenance-types';
// Product publication and paged transcript contracts (full set: modules/product/types).
export type {
  ReviewedContent, PublishAsRfcReq, PublishAsStoryReq, ProductTranscriptSummary,
  ProductTranscriptPage, ProductTranscriptSearchPage,
} from './types/product';
export type { ProductPublicationPreview } from './types/product-studio';
