"""Re-import the shipped GLBs and assert their runtime contract."""
import bpy,json,sys,math,hashlib,struct
from pathlib import Path
from mathutils import Vector
P=Path(__file__).resolve();OUT=P.parents[2]/'public/room-games'
expected=['CargoCrate','Barrier','StationColumn','StationWall','Reactor','FoundryFurnace','PipeRack','DesertRock','DesertArch','PalmTree','PineTree','BroadleafTree','CoastalRock','NeonTower','NeonSign','RaceGantry','TireStack','TrackBarrier','BoostPad','Pickup']
results={}
def bounds(objects):
 points=[o.matrix_world@Vector(c) for o in objects if o.type=='MESH' for c in o.bound_box]
 low=[min(p[i] for p in points) for i in range(3)];high=[max(p[i] for p in points) for i in range(3)]
 assert all(math.isfinite(v) for v in low+high)
 return {'min':[round(low[0],4),round(low[2],4),round(-high[1],4)],'max':[round(high[0],4),round(high[2],4),round(-low[1],4)],'size':[round(high[0]-low[0],4),round(high[2]-low[2],4),round(high[1]-low[1],4)]}
for p in sorted(OUT.glob('*.glb')):
 bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
 for a in list(bpy.data.actions):bpy.data.actions.remove(a)
 bpy.ops.import_scene.gltf(filepath=str(p));bpy.context.view_layer.update()
 d=p.read_bytes();n=struct.unpack_from('<I',d,12)[0];j=json.loads(d[20:20+n]);names=[x.get('name','') for x in j['nodes']]
 roots=[o for o in bpy.context.scene.objects if o.parent is None]
 result={'bounds':bounds(list(bpy.context.scene.objects)),'root_nodes':[o.name for o in roots],'animations':[a['name'] for a in j.get('animations',[])],'nodes':len(names),'meshes':len(j.get('meshes',[])),'sha256':hashlib.sha256(d).hexdigest(),'bytes':len(d)}
 if p.name.startswith('fighter'):
  assert result['animations']==['Idle','Run','Shoot','Death'],result
  assert 'Fighter' in names and 'Muzzle' in names
  assert 1.75<result['bounds']['size'][1]<2.1
  assert abs(result['bounds']['min'][1])<.02
 if p.name.startswith('kart'):
  assert all(x in names for x in ['Kart','WheelFL','WheelFR','WheelRL','WheelRR','DriverHead'])
 if p.name=='rifle.glb':assert 'Rifle' in names and 'Muzzle' in names
 if p.name=='environment-kit.glb':
  assert set(o.name for o in roots)==set(expected)
  result['items']={o.name:bounds([o]+list(o.children_recursive)) for o in roots}
 assert not j.get('images'), 'Texture-free palette is expected'
 results[p.name]=result
assert sum(r['bytes'] for r in results.values())<20*1024**2
(P.parent/'validation.json').write_text(json.dumps({'blender':bpy.app.version_string,'checks':'GLB re-import, finite bounds, floor-origin fighters, root nodes, clips, zero textures, total budget','files':results},indent=2)+'\n')
print('VALIDATION PASSED',json.dumps({k:v['bounds'] for k,v in results.items()},indent=2))
