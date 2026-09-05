// Public surface of the scene3d module (imported by the Design Arena as
// `../design/scene3d`). Components + pure document helpers; `three` itself is
// only ever loaded lazily from inside the viewport / exporter.
export * from './types';
export * from './ops';
export { validate, parseScene, serializeScene, isSafeId, isHexColor } from './validate';
export type { ValidationIssue, ValidationResult } from './validate';
export { exportSceneToGlb, exportObjectToGlb, glbFileName } from './exportGlb';
export { default as Scene3DViewport } from './Scene3DViewport.svelte';
export { default as Hierarchy } from './Hierarchy.svelte';
export { default as Inspector } from './Inspector.svelte';
