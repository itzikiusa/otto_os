<script lang="ts">
  // Extracted modal host for Studio3D: Generate / Stage Pipeline modal,
  // 3D Provider Manager, Workspace Export, and Optimize for Web modal.
  import { router } from '../../../lib/router.svelte';
  import * as mapi from '../../../lib/api/media';
  import { glbFileName } from '../../../lib/design/scene3d';
  import type { DesignArtifact, DesignAssistTurn, Media3dPipelineStage } from '../../../lib/api/types';
  import GenerateModal from './GenerateModal.svelte';
  import OptimizeModal, { type OptimizeState } from './OptimizeModal.svelte';
  import Provider3DManagerModal from './Provider3DManagerModal.svelte';
  import WorkspaceExportModal from './WorkspaceExportModal.svelte';
  import type { Gen3dContext, Gen3dKind } from './providers';

  interface Props {
    artifact: DesignArtifact;
    genKind: Gen3dKind | null;
    genStage: Media3dPipelineStage;
    genInitialPrompt: string;
    genCtx: Gen3dContext;
    canEdit: boolean;
    canAdmin3d: boolean;
    readonly: boolean;
    effectiveVersionId: string | null;
    providersOpen: boolean;
    exportWsOpen: boolean;
    opt: OptimizeState | null;
    savingGlb: boolean;
    onclosegen: () => void;
    onstarted: (turn: DesignAssistTurn, label: string) => void;
    oncloseproviders: () => void;
    onproviderschanged?: () => void;
    oncloseexportws: () => void;
    oncloseopt: () => void;
    ondownloadopt: (bytes: Uint8Array, name: string) => void;
    onsaveopt: () => void;
  }
  let {
    artifact,
    genKind,
    genStage,
    genInitialPrompt,
    genCtx,
    canEdit,
    canAdmin3d,
    readonly,
    effectiveVersionId,
    providersOpen,
    exportWsOpen,
    opt,
    savingGlb,
    onclosegen,
    onstarted,
    oncloseproviders,
    onproviderschanged,
    oncloseexportws,
    oncloseopt,
    ondownloadopt,
    onsaveopt,
  }: Props = $props();
</script>

{#if genKind}
  <GenerateModal
    kind={genKind}
    {artifact}
    ctx={{ ...genCtx, canEdit }}
    stage={genStage}
    sourceVersionId={effectiveVersionId}
    initialPrompt={genInitialPrompt}
    onclose={onclosegen}
    {onstarted}
    onartifactready={(aid) => {
      if (aid !== artifact.id) router.go(`design/a/${encodeURIComponent(aid)}`);
    }}
  />
{/if}

{#if providersOpen}
  <Provider3DManagerModal canAdmin={canAdmin3d} onclose={oncloseproviders} onchanged={onproviderschanged} />
{/if}

{#if exportWsOpen}
  <WorkspaceExportModal
    title="Export 3D model to workspace"
    defaultPath={`assets/models/${glbFileName(artifact.title, artifact.format === 'obj' ? 'obj' : 'glb')}`}
    hint="Writes the 3D asset safely inside the active workspace root."
    onexport={(rel, overwrite) => mapi.export3dArtifactToWorkspace(artifact.id, { relative_path: rel, version_id: effectiveVersionId, overwrite })}
    onclose={oncloseexportws}
  />
{/if}

{#if opt}
  <OptimizeModal
    {opt}
    title={artifact.title}
    {readonly}
    saving={savingGlb}
    onclose={oncloseopt}
    ondownload={ondownloadopt}
    onsave={onsaveopt}
  />
{/if}
