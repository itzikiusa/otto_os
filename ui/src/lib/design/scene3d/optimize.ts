// "Optimize for web" — shrink a GLB for site embeds with gltf-transform (MIT)
// + meshoptimizer (MIT), entirely in the webview. Both are imported lazily, so
// they form their own chunk that only loads when someone clicks the action.
// Nothing is fetched: meshoptimizer ships its WASM inline.
//
// Pipeline: dedup (shared accessors/materials/textures) → prune (unused
// nodes, materials, accessors) → weld (merge identical vertices) → meshopt
// (reorder + quantize + EXT_meshopt_compression). If the meshopt encoder can't
// start, we fall back to plain quantization (KHR_mesh_quantization) so the
// file still shrinks. Draco is deliberately not offered: its encoder WASM is
// not bundled and the proposal keeps the app offline (a later option).
//
// Every scene3d viewer decodes the result: the loaders register three's
// bundled MeshoptDecoder, and KHR_mesh_quantization is native to GLTFLoader.
import { budgetStatus, WEB_BUDGET_BYTES } from './presets';

export interface OptimizeResult {
  bytes: Uint8Array<ArrayBuffer>;
  before: number;
  after: number;
  /** Human-readable steps actually applied. */
  steps: string[];
  compression: 'meshopt' | 'quantize';
  budget: ReturnType<typeof budgetStatus>;
}

export async function optimizeGlb(input: ArrayBuffer | Uint8Array, budget = WEB_BUDGET_BYTES): Promise<OptimizeResult> {
  const [core, ext, fn, mo] = await Promise.all([
    import('@gltf-transform/core'),
    import('@gltf-transform/extensions'),
    import('@gltf-transform/functions'),
    import('meshoptimizer'),
  ]);
  const src = input instanceof Uint8Array ? input : new Uint8Array(input);
  const io = new core.WebIO().registerExtensions(ext.ALL_EXTENSIONS);
  let encoderReady = false;
  try {
    await Promise.all([mo.MeshoptEncoder.ready, mo.MeshoptDecoder.ready]);
    io.registerDependencies({ 'meshopt.encoder': mo.MeshoptEncoder, 'meshopt.decoder': mo.MeshoptDecoder });
    encoderReady = true;
  } catch {
    /* WASM unavailable — quantize only */
  }
  const doc = await io.readBinary(src);
  const steps = ['Deduplicated', 'Pruned unused data', 'Welded vertices'];
  await doc.transform(fn.dedup(), fn.prune(), fn.weld());
  let compression: OptimizeResult['compression'] = 'quantize';
  if (encoderReady) {
    try {
      await doc.transform(fn.meshopt({ encoder: mo.MeshoptEncoder, level: 'medium' }));
      compression = 'meshopt';
      steps.push('Meshopt compression (quantized)');
    } catch {
      /* fall through to quantize */
    }
  }
  if (compression === 'quantize') {
    await doc.transform(fn.quantize());
    steps.push('Quantized vertex data');
  }
  const out = await io.writeBinary(doc);
  return { bytes: out, before: src.byteLength, after: out.byteLength, steps, compression, budget: budgetStatus(out.byteLength, budget) };
}
