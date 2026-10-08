"""Compose chooser art from the exact GLBs shipped to the renderer."""
import bpy,sys,math
from pathlib import Path
from mathutils import Vector
P=Path(__file__).resolve();sys.path.insert(0,str(P.parent));import build_assets as A
A.palette()
def load(name):
 before=set(bpy.context.scene.objects);bpy.ops.import_scene.gltf(filepath=str(A.OUT/name));new=set(bpy.context.scene.objects)-before
 return [o for o in new if o.parent is None]
def clear():A.reset()
def kit():return {o.name:o for o in load('environment-kit.glb')}
def remove_unused(objs,used):
 for n,o in objs.items():
  if n not in used:
   for child in list(o.children_recursive):bpy.data.objects.remove(child,do_unlink=True)
   bpy.data.objects.remove(o,do_unlink=True)
def place(o,p,s=1,angle=0):o.location=A.xyz(p);o.scale=(s,s,s);o.rotation_euler.z=angle;return o
def clone(o,p,s=1,angle=0):
 n=o.copy();n.data=o.data.copy() if o.data else None;bpy.context.collection.objects.link(n)
 def children(src,dst):
  for c in src.children:
   cc=c.copy();cc.data=c.data;cc.parent=dst;bpy.context.collection.objects.link(cc);children(c,cc)
 children(o,n);return place(n,p,s,angle)
def hero(name,cam,target,scale):
 scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=48;scene.cycles.use_denoising=True
 scene.render.resolution_x=1280;scene.render.resolution_y=720;scene.render.resolution_percentage=100
 scene.world.color=(.13,.13,.13);scene.view_settings.view_transform='AgX'
 bpy.ops.object.camera_add(location=A.xyz(cam));o=bpy.context.object;o.rotation_euler=(Vector(A.xyz(target))-o.location).to_track_quat('-Z','Y').to_euler();o.data.type='ORTHO';o.data.ortho_scale=scale;scene.camera=o
 for p,power,size,col in [((3,8,6),2300,7,(.66,.85,1)),((-4,5,2),1800,6,(1,.55,.24)),((0,7,-5),3200,5,(.08,.55,1))]:
  bpy.ops.object.light_add(type='AREA',location=A.xyz(p));o=bpy.context.object;o.data.energy=power;o.data.color=col;o.data.shape='DISK';o.data.size=size;o.rotation_euler=(Vector(A.xyz(target))-o.location).to_track_quat('-Z','Y').to_euler()
 scene.render.image_settings.file_format='PNG';scene.render.filepath=str(A.OUT/(name+'.png'));bpy.ops.render.render(write_still=True)
clear();k=kit();used=['Barrier','StationColumn','StationWall','CargoCrate','Reactor'];remove_unused(k,used)
A.box('Arena floor',(0,-.12,0),(22,.24,20),'Road',bevel=.04)
for x in range(-8,9,2):A.box('Floor seam',(x,.008,0),(.018,.015,17),'Titanium',bevel=0)
for z in range(-8,9,2):A.box('Floor seam',(0,.008,z),(18,.015,.018),'Titanium',bevel=0)
place(k['StationWall'],(0,0,-4));clone(k['StationWall'],(-4,0,-4));clone(k['StationWall'],(4,0,-4))
place(k['StationColumn'],(-5,0,-3));clone(k['StationColumn'],(5,0,-3));place(k['Reactor'],(3.5,0,-2))
place(k['Barrier'],(-2.6,0,-.8));place(k['CargoCrate'],(3,0,1));clone(k['CargoCrate'],(4.5,0,.2))
f=load('fighter-azure.glb')[0];place(f,(-.6,0,1),1.25,-.25)
f=load('fighter-ember.glb')[0];place(f,(1.9,0,-1.8),1.15,.4)
hero('arena-preview',(7,4.7,8),(0,1.1,-.2),10.2)
clear();k=kit();used=['PalmTree','CoastalRock','TrackBarrier','RaceGantry'];remove_unused(k,used)
A.box('Island sand',(0,-.15,0),(30,.2,30),'SandLight',bevel=.02);A.box('Racing road',(0,-.035,0),(6.8,.1,26),'Road',bevel=.06)
for x in [-3.5,3.5]:
 for z in range(-12,14):A.box('Kerb',(x,.035,z),(.3,.1,.94),'Ivory' if z%2 else 'Ember',bevel=.015)
for z in range(-12,13,3):A.box('Lane marker',(0,.022,z),(.1,.008,1.2),'Ivory',bevel=0)
place(k['RaceGantry'],(0,0,-7));place(k['TrackBarrier'],(-4.2,0,-3),1,math.pi/2);clone(k['TrackBarrier'],(4.2,0,-3),1,math.pi/2)
place(k['PalmTree'],(-5,0,-5),1.2);clone(k['PalmTree'],(5,0,-4),1.3);clone(k['PalmTree'],(-5,0,3),1.1)
place(k['CoastalRock'],(5,0,2));clone(k['CoastalRock'],(-5,0,-2),1.3)
place(load('kart-azure.glb')[0],(-1.1,.02,1.1),1.1,-.13);place(load('kart-ember.glb')[0],(1.25,.02,-1.1),1.1,.11)
hero('kart-preview',(7.8,5.7,10),(0,.8,0),11.8)

A.manifest()
