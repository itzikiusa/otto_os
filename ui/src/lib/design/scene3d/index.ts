// Public surface of the scene3d module (imported by the Design Arena as
// `../design/scene3d` and by Design Hall's 3D Studio). Components + pure
// document helpers; `three` itself is only ever loaded lazily from inside the
// viewport / exporters / embed runtime, and gltf-transform only from
// `optimize.ts` (its own lazy chunk).
export * from './types';
export * from './ops';
export { validate, parseScene, serializeScene, isSafeId, isHexColor, isColorRef, isDesignUri } from './validate';
export type { ValidationIssue, ValidationResult } from './validate';
export { exportSceneToGlb, exportObjectToGlb, exportObjectToUsdz, glbFileName } from './exportGlb';
export * from './presets';
export * from './tokens';
export * from './states';
export { default as Scene3DViewport } from './Scene3DViewport.svelte';
export { default as Hierarchy } from './Hierarchy.svelte';
export { default as Inspector } from './Inspector.svelte';
