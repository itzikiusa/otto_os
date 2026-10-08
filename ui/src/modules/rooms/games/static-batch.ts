import * as T from 'three';
import {mergeGeometries} from 'three/examples/jsm/utils/BufferGeometryUtils.js';
/** Authored scenery shares materials; batch it once, leaving animated actors/pickups alone. */
export function batchScenery(scene:T.Scene,moving:T.Object3D[]):void {
 const excluded=new Set<T.Object3D>();moving.forEach(root=>root.traverse(o=>excluded.add(o)));
 const groups=new Map<string,{material:T.Material;shadow:boolean;geometries:T.BufferGeometry[]}>();
 const originals:T.Mesh[]=[];
 scene.updateMatrixWorld(true);
 scene.traverse(o=>{
  if(!(o instanceof T.Mesh)||excluded.has(o)||Array.isArray(o.material))return;
  const material=o.material;
  if(material instanceof T.MeshStandardMaterial&&material.map)return;
  const geometry=o.geometry.index?o.geometry.toNonIndexed():o.geometry.clone();geometry.applyMatrix4(o.matrixWorld);
  for(const key of Object.keys(geometry.attributes))if(key!=='position'&&key!=='normal')geometry.deleteAttribute(key);
  if(!geometry.attributes.normal)geometry.computeVertexNormals();
  const key=`${material.uuid}:${o.castShadow}`;
  const group=groups.get(key)??{material,shadow:o.castShadow,geometries:[] as T.BufferGeometry[]};group.geometries.push(geometry);groups.set(key,group);originals.push(o);
 });
 for(const group of groups.values()){
  const geometry=mergeGeometries(group.geometries);if(!geometry)throw new Error('Could not prepare scenery geometry');
  const object=new T.Mesh(geometry,group.material);object.castShadow=group.shadow;object.receiveShadow=true;scene.add(object);
  group.geometries.forEach(g=>g.dispose());
 }
 originals.forEach(o=>{o.removeFromParent();o.geometry.dispose();});
}
