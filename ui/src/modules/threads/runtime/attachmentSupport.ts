// What a runtime thread's agent takes besides text: the capability helpers
// live in the provider registry (shared with the IDE agent dock).

export { attachmentSupport, capsOf, takesFile, type TakesCaps } from '../../../lib/providerRegistry.ts';
