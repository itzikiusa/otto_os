/**
 * Kart atmosphere shaders: the sky dome (gradient, sun disc and halo,
 * drifting procedural clouds, stars at dusk), the lagoon / lake water
 * (rolling normals, Fresnel sky reflection, sun glitter, depth tint and
 * shoreline foam from a baked depth map), molten lava (flowing crust with
 * glowing cracks), and a patch for lit materials that adds underwater
 * caustics and the red glow lava throws on nearby rock. Every animated
 * shader reads one shared `time` uniform, set once per frame.
 */
import * as THREE from 'three';

/** Uniforms shared by every animated kart shader (one write per frame). */
export interface SharedUniforms { time: { value: number } }

const NOISE = /* glsl */ `
float hash12(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
float vnoise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x), mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm4(vec2 p) { float s = 0.0, a = 0.5; for (int k = 0; k < 4; k++) { s += a * vnoise(p); p = p * 2.03 + vec2(17.1, 9.2); a *= 0.5; } return s; }
`;

export interface SkyOptions { top: number; horizon: number; ground: number; sun: number; sunDir: THREE.Vector3; cloud: number; cover: number; stars: number }
/**
 * Sky dome material for GameView.setSky: horizon-to-zenith gradient, a
 * sun disc with a wide warm halo, clouds (fbm projected on a high plane,
 * lit from the sun side, thinning toward the horizon), and stars where
 * `stars` > 0. `under` (0..1) washes it to the underwater colour.
 */
export function skyMaterial(o: SkyOptions, shared: SharedUniforms): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    side: THREE.BackSide, depthWrite: false, fog: false,
    uniforms: {
      time: shared.time, top: { value: new THREE.Color(o.top) }, horizon: { value: new THREE.Color(o.horizon) }, ground: { value: new THREE.Color(o.ground) },
      sunColor: { value: new THREE.Color(o.sun) }, sunDir: { value: o.sunDir.clone().normalize() }, cloud: { value: new THREE.Color(o.cloud) },
      cover: { value: o.cover }, stars: { value: o.stars }, under: { value: 0 }, deep: { value: new THREE.Color(0x0d5f78) },
    },
    vertexShader: /* glsl */ `varying vec3 vDir; void main(){ vDir = normalize(position); vec4 p = modelViewMatrix * vec4(position, 1.0); gl_Position = projectionMatrix * p; gl_Position.z = gl_Position.w; }`,
    fragmentShader: /* glsl */ `
      uniform float time, cover, stars, under; uniform vec3 top, horizon, ground, sunColor, sunDir, cloud, deep;
      varying vec3 vDir;
      ${NOISE}
      void main() {
        vec3 d = normalize(vDir);
        float h = clamp(d.y, -1.0, 1.0);
        vec3 col = mix(horizon, top, pow(clamp(h * 1.5, 0.0, 1.0), 0.65));
        col = mix(col, ground, smoothstep(0.0, -0.25, h));
        float sd = max(dot(d, sunDir), 0.0);
        // Warm halo around the sun, brighter near the horizon.
        col += sunColor * (pow(sd, 6.0) * 0.22 + pow(sd, 48.0) * 0.35) * (1.0 - 0.4 * clamp(h, 0.0, 1.0));
        if (stars > 0.0 && h > 0.05) {
          vec2 sp = d.xz / (d.y + 0.35) * 220.0;
          float st = step(0.9965, hash12(floor(sp))) * (0.6 + 0.4 * sin(time * 2.0 + hash12(floor(sp) + 7.0) * 30.0));
          col += vec3(st) * stars * smoothstep(0.05, 0.4, h);
        }
        // Clouds on a plane far overhead, drifting with the wind.
        if (h > 0.0) {
          vec2 cp = d.xz / (d.y + 0.12) * 1.6 + vec2(time * 0.012, time * 0.005);
          float n = fbm4(cp);
          float c = smoothstep(1.0 - cover, 1.15 - cover * 0.6, n) * smoothstep(0.0, 0.18, h);
          float lit = 0.75 + 0.35 * pow(sd, 3.0) + 0.25 * (fbm4(cp * 2.0 + 3.0) - 0.5);
          vec3 cc = cloud * lit;
          col = mix(col, cc, c * 0.92);
        }
        // The sun disc itself, in front of everything.
        col += sunColor * smoothstep(0.9993, 0.99965, sd) * 2.5;
        col = mix(col, deep, under);
        gl_FragColor = vec4(col, 1.0);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }`,
  });
}

export interface WaterOptions { deep: number; shallow: number; top: number; horizon: number; sun: number; sunDir: THREE.Vector3 }
/**
 * Water surface for a flat plane at `level`: four rolling wave trains plus a
 * fine ripple give the normal; Fresnel mixes the depth-tinted water with the
 * reflected sky gradient, the sun leaves a glitter path, and `depth` (metres
 * of water above the land, baked into a texture over `rect` = x0, z0, width,
 * depth) drives the shallow tint, the transparency and the foam bands that
 * lap the shore.
 */
export function waterMaterial(o: WaterOptions, depth: THREE.Texture, rect: THREE.Vector4, shared: SharedUniforms): THREE.ShaderMaterial {
  const m = new THREE.ShaderMaterial({
    transparent: true, depthWrite: false, side: THREE.DoubleSide, fog: true,
    uniforms: THREE.UniformsUtils.merge([THREE.UniformsLib.fog, {
      deepColor: { value: new THREE.Color(o.deep) }, shallowColor: { value: new THREE.Color(o.shallow) },
      skyTop: { value: new THREE.Color(o.top) }, skyHorizon: { value: new THREE.Color(o.horizon) },
      sunColor: { value: new THREE.Color(o.sun) }, sunDir: { value: o.sunDir.clone().normalize() },
      depthMap: { value: null }, rect: { value: new THREE.Vector4() },
    }]),
    vertexShader: /* glsl */ `
      varying vec3 vWorld;
      #include <fog_pars_vertex>
      void main() {
        vec4 wp = modelMatrix * vec4(position, 1.0);
        vWorld = wp.xyz;
        vec4 mvPosition = viewMatrix * wp;
        gl_Position = projectionMatrix * mvPosition;
        #include <fog_vertex>
      }`,
    fragmentShader: /* glsl */ `
      uniform float time; uniform vec3 deepColor, shallowColor, skyTop, skyHorizon, sunColor, sunDir;
      uniform sampler2D depthMap; uniform vec4 rect;
      varying vec3 vWorld;
      #include <fog_pars_fragment>
      ${NOISE}
      vec2 waves(vec2 p, float t) {
        vec2 g = vec2(0.0);
        g += vec2(0.8, 0.6) * 0.14 * 0.35 * cos(dot(vec2(0.8, 0.6), p) * 0.35 + t * 1.1);
        g += vec2(-0.5, 0.86) * 0.1 * 0.6 * cos(dot(vec2(-0.5, 0.86), p) * 0.6 + t * 1.5);
        g += vec2(0.2, -0.98) * 0.06 * 1.3 * cos(dot(vec2(0.2, -0.98), p) * 1.3 + t * 2.3);
        g += vec2(-0.9, -0.3) * 0.04 * 2.1 * cos(dot(vec2(-0.9, -0.3), p) * 2.1 + t * 3.1);
        return g;
      }
      void main() {
        float t = time;
        vec2 p = vWorld.xz;
        vec2 g = waves(p, t);
        // Fine ripple: gradient of moving value noise.
        vec2 q = p * 0.9 + vec2(t * 0.25, -t * 0.18);
        float e = 0.15, n0 = vnoise(q);
        g += vec2(vnoise(q + vec2(e, 0.0)) - n0, vnoise(q + vec2(0.0, e)) - n0) / e * 0.06;
        vec3 n = normalize(vec3(-g.x, 1.0, -g.y));
        vec3 v = normalize(cameraPosition - vWorld);
        if (!gl_FrontFacing) n = -n;
        float fres = 0.03 + 0.97 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
        vec3 r = reflect(-v, n);
        vec3 sky = mix(skyHorizon, skyTop, pow(clamp(r.y * 1.5, 0.0, 1.0), 0.65));
        vec2 uv = (p - rect.xy) / rect.zw;
        float depth = texture2D(depthMap, clamp(uv, 0.0, 1.0)).r * 8.0;
        vec3 water = mix(shallowColor, deepColor, smoothstep(0.2, 6.0, depth));
        vec3 col = mix(water, sky, fres * 0.85);
        float spec = pow(max(dot(reflect(-sunDir, n), v), 0.0), 220.0);
        col += sunColor * spec * 3.0;
        // Foam: a solid lip at the waterline and bands rolling in from deeper water.
        float shore = 1.0 - smoothstep(0.0, 0.35, depth);
        float bands = smoothstep(0.6, 0.9, sin(depth * 9.0 - t * 2.2 + vnoise(p * 0.4) * 4.0) * 0.5 + 0.5) * (1.0 - smoothstep(0.2, 1.4, depth));
        float foam = clamp(shore + bands * 0.7, 0.0, 1.0) * (0.65 + 0.35 * vnoise(p * 1.7 + t * 0.4));
        col = mix(col, vec3(0.96, 0.99, 1.0), foam * 0.9);
        float alpha = clamp(0.5 + smoothstep(0.0, 4.0, depth) * 0.35 + fres * 0.3 + foam * 0.5, 0.0, 0.94);
        if (!gl_FrontFacing) { col = mix(deepColor, shallowColor, 0.4) + sunColor * spec; alpha = 0.55; }
        gl_FragColor = vec4(col, alpha);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
        #include <fog_fragment>
      }`,
  });
  // UniformsUtils.merge clones values: link the shared clock and the baked depth map afterwards.
  m.uniforms.time = shared.time;
  m.uniforms.depthMap.value = depth;
  m.uniforms.rect.value = rect;
  return m;
}

/**
 * Lava: a slow two-layer flow of dark crust over molten rock; the glow
 * shows through the cracks and pulses. Unlit (it is the light), fogged.
 */
export function lavaMaterial(hot: number, shared: SharedUniforms): THREE.ShaderMaterial {
  const m = new THREE.ShaderMaterial({
    fog: true,
    uniforms: THREE.UniformsUtils.merge([THREE.UniformsLib.fog, { hot: { value: new THREE.Color(hot) } }]),
    vertexShader: /* glsl */ `
      varying vec3 vWorld;
      #include <fog_pars_vertex>
      void main() {
        vec4 wp = modelMatrix * vec4(position, 1.0); vWorld = wp.xyz; vec4 mvPosition = viewMatrix * wp; gl_Position = projectionMatrix * mvPosition;
        #include <fog_vertex>
      }`,
    fragmentShader: /* glsl */ `
      uniform float time; uniform vec3 hot;
      varying vec3 vWorld;
      #include <fog_pars_fragment>
      ${NOISE}
      void main() {
        vec2 p = vWorld.xz * 0.07;
        float a = fbm4(p + vec2(time * 0.02, time * 0.013));
        float b = fbm4(p * 2.3 - vec2(time * 0.035, -time * 0.02) + a * 1.5);
        float crack = smoothstep(0.42, 0.62, b);
        float pulse = 0.85 + 0.15 * sin(time * 1.7 + a * 9.0);
        vec3 crust = vec3(0.09, 0.05, 0.04) * (0.6 + a);
        vec3 molten = mix(hot, vec3(1.0, 0.86, 0.35), smoothstep(0.75, 1.0, crack + a * 0.3)) * pulse * 1.6;
        gl_FragColor = vec4(mix(crust, molten, 1.0 - crack * 0.85), 1.0);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
        #include <fog_fragment>
      }`,
  });
  m.uniforms.time = shared.time;
  return m;
}

export interface WorldPatch { caustics?: number; glow?: { level: number; color: number } }
/**
 * Patches a lit material (Lambert or Standard, not instanced) in place:
 * `caustics` (the water level) dances light over whatever lies below the
 * surface; `glow` adds emissive heat to anything within a few metres above
 * the lava. Cheap: a world-position varying and a handful of ALU per pixel.
 */
export function patchWorld(material: THREE.Material, patch: WorldPatch, shared: SharedUniforms): void {
  if (patch.caustics === undefined && !patch.glow) return;
  const key = `kart-world-${patch.caustics !== undefined ? 'c' : ''}${patch.glow ? 'g' : ''}`;
  material.customProgramCacheKey = () => key;
  material.onBeforeCompile = shader => {
    shader.uniforms.time = shared.time;
    shader.uniforms.waterLevel = { value: patch.caustics ?? -1e4 };
    shader.uniforms.lavaLevel = { value: patch.glow?.level ?? -1e4 };
    shader.uniforms.lavaColor = { value: new THREE.Color(patch.glow?.color ?? 0) };
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 vKartWorld;')
      .replace('#include <begin_vertex>', '#include <begin_vertex>\nvKartWorld = (modelMatrix * vec4(transformed, 1.0)).xyz;');
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', `#include <common>\nvarying vec3 vKartWorld;\nuniform float time, waterLevel, lavaLevel;\nuniform vec3 lavaColor;\n${patch.caustics !== undefined ? 'float caustic(vec2 p, float t){ vec2 a = p * 0.55 + vec2(t * 0.31, t * 0.17); vec2 b = p * 0.75 - vec2(t * 0.23, -t * 0.29); float c = abs(sin(a.x + sin(a.y * 1.3))) * abs(sin(b.y + sin(b.x * 1.1))); return pow(1.0 - c, 6.0); }' : ''}`)
      .replace('#include <opaque_fragment>', `${patch.caustics !== undefined ? 'float under = smoothstep(waterLevel + 0.05, waterLevel - 0.6, vKartWorld.y);\noutgoingLight += diffuseColor.rgb * caustic(vKartWorld.xz, time) * under * (1.1 - clamp((waterLevel - vKartWorld.y) / 14.0, 0.0, 0.9));\noutgoingLight = mix(outgoingLight, outgoingLight * vec3(0.55, 0.85, 1.0), under * 0.6);\n' : ''}${patch.glow ? 'float heat = smoothstep(lavaLevel + 7.0, lavaLevel, vKartWorld.y);\noutgoingLight += lavaColor * heat * heat * (0.85 + 0.15 * sin(time * 1.7 + vKartWorld.x * 0.1));\n' : ''}#include <opaque_fragment>`);
  };
}

/**
 * Patches an instanced Lambert material for the crowd: every spectator
 * bounces and sways on its own beat (phase from its seat position), harder
 * while `cheer` is up (the pack passing, the finish).
 */
export function patchCrowd(material: THREE.Material, shared: SharedUniforms, cheer: { value: number }): void {
  material.customProgramCacheKey = () => 'kart-crowd';
  material.onBeforeCompile = shader => {
    shader.uniforms.time = shared.time;
    shader.uniforms.cheer = cheer;
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nuniform float time, cheer;')
      .replace('#include <begin_vertex>', `#include <begin_vertex>
        vec3 seat = vec3(instanceMatrix[3]);
        float ph = fract(sin(dot(seat.xz, vec2(12.9898, 78.233))) * 43758.5453);
        float beat = time * (5.0 + ph * 4.0) + ph * 6.2832;
        transformed.y += max(0.0, sin(beat)) * (0.08 + 0.32 * cheer) * smoothstep(0.2, 0.9, position.y + 0.5);
        transformed.x += sin(beat * 0.5) * 0.05 * position.y;`);
  };
}
