import {test} from 'node:test';
import assert from 'node:assert/strict';
import * as T from 'three';
import {batchScenery} from '../src/modules/rooms/games/static-batch.ts';
test('textured road signs retain their UV coordinates and material after batching',()=>{
 const scene=new T.Scene(),texture=new T.Texture();
 const sign=new T.Mesh(new T.PlaneGeometry(2,1),new T.MeshBasicMaterial({map:texture}));
 scene.add(sign);batchScenery(scene,[]);
 assert.ok(scene.children.includes(sign));
 assert.ok(sign.geometry.getAttribute('uv'));
 assert.equal(sign.material.map,texture);
});
