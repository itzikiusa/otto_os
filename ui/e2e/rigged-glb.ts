// A tiny rigged, animated GLB built in code (no binary fixture in the repo): a
// 0.3 × 0.5 × 0.2 m red box skinned to one bone, with one 1 s clip that turns
// the bone. Small on purpose: a standalone model sits on a 20 m floor, so a fit
// that frames the floor leaves it a few pixels tall.

export function riggedGlbBase64(): string {
  const [w, h, d] = [0.15, 0.5, 0.1];
  const pos = [[-w, 0, -d], [w, 0, -d], [w, h, -d], [-w, h, -d], [-w, 0, d], [w, 0, d], [w, h, d], [-w, h, d]].flat();
  const idx = [0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5];
  const parts: { bytes: Uint8Array; target?: number }[] = [
    { bytes: new Uint8Array(new Float32Array(pos).buffer), target: 34962 },
    { bytes: new Uint8Array(new Uint16Array(idx).buffer), target: 34963 },
    { bytes: new Uint8Array(32), target: 34962 }, // JOINTS_0: every vertex on joint 0
    { bytes: new Uint8Array(new Float32Array(Array.from({ length: 8 }, () => [1, 0, 0, 0]).flat()).buffer), target: 34962 },
    { bytes: new Uint8Array(new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]).buffer) }, // inverse bind
    { bytes: new Uint8Array(new Float32Array([0, 1]).buffer) }, // clip times
    { bytes: new Uint8Array(new Float32Array([0, 0, 0, 1, 0, 0.7071, 0, 0.7071]).buffer) }, // clip rotations
  ];
  let offset = 0;
  const bufferViews = parts.map((p) => {
    const v = { buffer: 0, byteOffset: offset, byteLength: p.bytes.length, ...(p.target ? { target: p.target } : {}) };
    offset += p.bytes.length; // every part is a multiple of 4 bytes
    return v;
  });
  const json = {
    asset: { version: '2.0', generator: 'otto-e2e' },
    scene: 0,
    scenes: [{ nodes: [0, 1] }],
    nodes: [{ name: 'Body', mesh: 0, skin: 0 }, { name: 'Root' }],
    meshes: [{ name: 'Body', primitives: [{ attributes: { POSITION: 0, JOINTS_0: 2, WEIGHTS_0: 3 }, indices: 1, material: 0 }] }],
    materials: [{ pbrMetallicRoughness: { baseColorFactor: [1, 0, 0, 1], metallicFactor: 0, roughnessFactor: 1 }, emissiveFactor: [0.6, 0, 0], doubleSided: true }],
    skins: [{ joints: [1], inverseBindMatrices: 4 }],
    animations: [{ name: '100style-angry-forward-walk', channels: [{ sampler: 0, target: { node: 1, path: 'rotation' } }], samplers: [{ input: 5, output: 6 }] }],
    accessors: [
      { bufferView: 0, componentType: 5126, count: 8, type: 'VEC3', min: [-w, 0, -d], max: [w, h, d] },
      { bufferView: 1, componentType: 5123, count: 36, type: 'SCALAR' },
      { bufferView: 2, componentType: 5121, count: 8, type: 'VEC4' },
      { bufferView: 3, componentType: 5126, count: 8, type: 'VEC4' },
      { bufferView: 4, componentType: 5126, count: 1, type: 'MAT4' },
      { bufferView: 5, componentType: 5126, count: 2, type: 'SCALAR', min: [0], max: [1] },
      { bufferView: 6, componentType: 5126, count: 2, type: 'VEC4' },
    ],
    bufferViews,
    buffers: [{ byteLength: offset }],
  };
  const enc = new TextEncoder().encode(JSON.stringify(json));
  const jsonLen = Math.ceil(enc.length / 4) * 4;
  const out = new Uint8Array(12 + 8 + jsonLen + 8 + offset);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, 0x46546c67, true); // "glTF"
  dv.setUint32(4, 2, true);
  dv.setUint32(8, out.length, true);
  dv.setUint32(12, jsonLen, true);
  dv.setUint32(16, 0x4e4f534a, true); // "JSON"
  out.set(enc, 20);
  out.fill(0x20, 20 + enc.length, 20 + jsonLen);
  dv.setUint32(20 + jsonLen, offset, true);
  dv.setUint32(24 + jsonLen, 0x004e4942, true); // "BIN"
  let at = 28 + jsonLen;
  for (const p of parts) {
    out.set(p.bytes, at);
    at += p.bytes.length;
  }
  return Buffer.from(out).toString('base64');
}
