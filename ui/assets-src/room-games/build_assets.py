"""Original Rooms Arcade art. Blender 5.2 headless; no external dependencies."""
import bpy, bmesh, math, json, hashlib, struct, random
from pathlib import Path
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'public/room-games'; PRE=Path(__file__).resolve().parent/'previews'
OUT.mkdir(parents=True,exist_ok=True); PRE.mkdir(exist_ok=True)
random.seed(1809)
def xyz(p): return (p[0],-p[2],p[1])
def reset():
 bpy.ops.object.select_all(action='SELECT'); bpy.ops.object.delete(use_global=False)
 for a in list(bpy.data.actions): bpy.data.actions.remove(a)
def mat(n,c,metal=0,rough=.4,em=0):
 m=bpy.data.materials.get(n)
 if m:return m
 m=bpy.data.materials.new(n);m.diffuse_color=(*c,1);m.use_nodes=True
 b=m.node_tree.nodes.get('Principled BSDF');b.inputs['Base Color'].default_value=(*c,1);b.inputs['Metallic'].default_value=metal;b.inputs['Roughness'].default_value=rough
 b.inputs['Emission Color'].default_value=(*c,1);b.inputs['Emission Strength'].default_value=em
 return m
M={}
def palette():
 for n,c,met,r,e in [('Gunmetal',(.045,.065,.095),.8,.3,0),('Obsidian',(.013,.02,.029),.3,.47,0),('Titanium',(.23,.3,.33),.75,.37,0),('Ivory',(.54,.61,.59),.18,.37,0),('Azure',(.008,.13,.34),.32,.32,0),('Ember',(.62,.043,.015),.27,.33,0),('Copper',(.47,.19,.075),.8,.32,0),('CyanLight',(.005,.42,.7),.1,.3,2),('AmberLight',(1,.38,.035),.2,.3,3),('PinkLight',(.98,.055,.41),.25,.2,3),('Glass',(.013,.09,.15),.8,.15,0),('Sand',(.64,.39,.21),0,.9,0),('SandLight',(.57,.38,.16),0,.92,0),('Rock',(.32,.39,.41),0,.88,0),('Bark',(.19,.105,.047),0,.92,0),('Leaf',(.014,.125,.034),0,.85,0),('LeafLight',(.048,.23,.07),0,.83,0),('Rubber',(.016,.022,.027),0,.85,0),('Road',(.085,.1,.13),0,.95,0),('Yellow',(1,.66,.045),.2,.4,0),('SkinAzure',(.5,.255,.14),0,.8,0),('SkinEmber',(.25,.1,.048),0,.83,0),('EyeWhite',(.75,.82,.79),0,.35,0),('EyeIris',(.035,.17,.19),0,.3,0),('Fabric',(.025,.036,.047),0,.94,0)]:M[n]=mat(n,c,met,r,e)
def group(n,parent=None,p=(0,0,0)):
 o=bpy.data.objects.new(n,None);bpy.context.collection.objects.link(o);o.location=xyz(p);o.parent=parent;return o
def finish(o,n,m,parent):
 o.name=n;o.data.materials.append(M[m]);o.parent=parent
 return o
def box(n,p,s,m,parent=None,bevel=.035):
 bpy.ops.mesh.primitive_cube_add(size=1,location=xyz(p));o=bpy.context.object;o.scale=(s[0],s[2],s[1]);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
 if bevel:
  mod=o.modifiers.new('Machined edges','BEVEL');mod.width=bevel;mod.segments=2;bpy.context.view_layer.objects.active=o;bpy.ops.object.modifier_apply(modifier=mod.name)
  mod=o.modifiers.new('Weighted normals','WEIGHTED_NORMAL');bpy.ops.object.modifier_apply(modifier=mod.name)
 return finish(o,n,m,parent)
def uv(n,p,s,m,parent=None,seg=16):
 bpy.ops.mesh.primitive_uv_sphere_add(segments=seg,ring_count=8,location=xyz(p));o=bpy.context.object;o.scale=(s[0],s[2],s[1]);bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
 for f in o.data.polygons:f.use_smooth=True
 return finish(o,n,m,parent)
def cyl(n,p,r,d,m,parent=None,axis='Y',verts=16):
 bpy.ops.mesh.primitive_cylinder_add(vertices=verts,radius=r,depth=d,location=xyz(p));o=bpy.context.object
 if axis=='X':o.rotation_euler[1]=math.pi/2
 if axis=='Z':o.rotation_euler[0]=math.pi/2
 return finish(o,n,m,parent)
def cone(n,p,r1,r2,d,m,parent=None,verts=12):
 bpy.ops.mesh.primitive_cone_add(vertices=verts,radius1=r1,radius2=r2,depth=d,location=xyz(p));return finish(bpy.context.object,n,m,parent)
def torus(n,p,r,minor,m,parent=None,axis='Y'):
 bpy.ops.mesh.primitive_torus_add(major_segments=24,minor_segments=8,location=xyz(p),major_radius=r,minor_radius=minor);o=bpy.context.object
 if axis=='X':o.rotation_euler[1]=math.pi/2
 if axis=='Z':o.rotation_euler[0]=math.pi/2
 for f in o.data.polygons:f.use_smooth=True
 return finish(o,n,m,parent)
def beam(n,a,b,r,m,parent):
 aa=Vector(xyz(a));bb=Vector(xyz(b));mid=(aa+bb)/2
 bpy.ops.mesh.primitive_cylinder_add(vertices=10,radius=r,depth=(bb-aa).length,location=mid);o=bpy.context.object;o.rotation_euler=(bb-aa).to_track_quat('Z','Y').to_euler();return finish(o,n,m,parent)
def armor(n,p,sections,m,parent=None,longitudinal=False):
 # Chamfered eight-point cross sections produce designed taper and silhouette.
 ring=[(-1,-.65),(-.65,-1),(.65,-1),(1,-.65),(1,.65),(.65,1),(-.65,1),(-1,.65)]
 verts=[]
 for height,width,depth,offset in sections:
  for a,b in ring:
   q=(p[0]+a*width/2,p[1]+height,p[2]+b*depth/2+offset) if not longitudinal else (p[0]+a*width/2,p[1]+b*depth/2+offset,p[2]+height)
   verts.append(xyz(q))
 faces=[tuple(range(7,-1,-1))]
 for j in range(len(sections)-1):
  for i in range(8):faces.append((j*8+i,j*8+(i+1)%8,(j+1)*8+(i+1)%8,(j+1)*8+i))
 faces.append(tuple((len(sections)-1)*8+i for i in range(8)))
 mesh=bpy.data.meshes.new(n);mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=bm.faces);bm.to_mesh(mesh);bm.free();o=bpy.data.objects.new(n,mesh);bpy.context.collection.objects.link(o)
 bpy.context.view_layer.objects.active=o;o.select_set(True)
 bevel=o.modifiers.new('Precision chamfers','BEVEL');bevel.width=.013;bevel.segments=2;bpy.ops.object.modifier_apply(modifier=bevel.name)
 return finish(o,n,m,parent)
def slash(p,s,m,parent,angle=.3):
 o=box('Graphic slash',p,s,m,parent,.004);o.rotation_euler[1]=angle;return o
def bolt(p,parent):return cyl('Fastener',p,.018,.012,'Titanium',parent,'Z',8)
def panel(p,s,parent,color='Azure'):
 box('Inset panel',p,s,color,parent,.025)
 for x in [-1,1]:
  for y in [-1,1]:bolt((p[0]+x*(s[0]/2-.06),p[1]+y*(s[1]/2-.06),p[2]+s[2]/2+.006),parent)
def join_static():
 # Collapse rigid detail to one mesh per parent, retaining articulation nodes.
 buckets={}
 for o in list(bpy.context.scene.objects):
  if o.type=='MESH':buckets.setdefault(o.parent,[]).append(o)
 for parent,objects in buckets.items():
  if len(objects)<2:continue
  bpy.ops.object.select_all(action='DESELECT')
  for o in objects:o.select_set(True)
  bpy.context.view_layer.objects.active=objects[0];bpy.ops.object.join();objects[0].name=(parent.name if parent else 'Object')+'_Surface'
def export(n):
 join_static();bpy.ops.object.select_all(action='SELECT')
 bpy.ops.export_scene.gltf(filepath=str(OUT/n),export_format='GLB',use_selection=True,export_yup=True,export_animations=True,export_animation_mode='NLA_TRACKS',export_force_sampling=True,export_frame_range=False,export_materials='EXPORT')
def anim(o,clip,length,keys):
 o.animation_data_create();a=bpy.data.actions.new(clip+'_'+o.name);o.animation_data.action=a
 for f,rot,loc in keys:
  o.rotation_euler=rot;o.location=loc;o.keyframe_insert(data_path='rotation_euler',frame=f);o.keyframe_insert(data_path='location',frame=f)
 track=o.animation_data.nla_tracks.new();track.name=clip;track.strips.new(clip,1,a);o.animation_data.action=None
 o.rotation_euler=keys[0][1]
 # retain the rest translation
 o.location=keys[0][2]
def weapon(parent=None,origin=(0,0,0)):
 g=group('PulseRifle',parent,origin)
 box('Receiver',(0,0,.02),(.13,.17,.42),'Gunmetal',g,.027);box('Upper rail',(0,.1,.08),(.09,.055,.48),'Titanium',g,.015)
 box('Butt stock',(0,-.025,-.27),(.13,.15,.18),'Obsidian',g,.04);box('Grip',(0,-.135,-.055),(.075,.16,.075),'Rubber',g,.018)
 cyl('Barrel',(0,.025,.35),.047,.3,'Titanium',g,'Z');cyl('MuzzleShroud',(0,.025,.52),.068,.09,'Gunmetal',g,'Z');cyl('Muzzle emitter',(0,.025,.569),.031,.008,'CyanLight',g,'Z')
 for x in [-1,1]:
  box('Energy cell',(x*.074,.012,.03),(.018,.08,.25),'Azure',g,.008)
  for i in range(5):box('Cooling slot',(x*.086,.015,-.06+i*.045),(.012,.04,.015),'CyanLight',g,.003)
 box('Reflex sight',(0,.152,.03),(.075,.075,.035),'Obsidian',g,.008);box('Sight lens',(0,.155,.052),(.052,.04,.005),'CyanLight',g,.002)
 group('Muzzle',g,(0,.025,.58));return g
def fighter(color):
 reset();g=group('Fighter');hips=group('Hips',g,(0,.885,0));body=group('Torso',hips,(0,.28,0))
 uv('Tactical undersuit',(0,.015,0),(.235,.3,.145),'Fabric',body);armor('Swept breastplate',(0,0,.075),[(-.14,.29,.2,0),(.02,.51,.29,0),(.21,.54,.26,-.01),(.25,.35,.2,0)],color,body)
 armor('Sternum shield',(0,.05,.224),[(-.08,.1,.025,0),(.055,.25,.035,0),(.12,.3,.025,0)],'Ivory',body);box('Reactor light',(0,.13,.25),(.11,.035,.016),'CyanLight',body,.007)
 for side in [-1,1]:
  slash((side*.18,.14,.211),(.13,.03,.018),'Gunmetal',body,side*.35)
  for j in range(3):slash((side*(.195+j*.01),.01-j*.034,.225),(.055,.015,.024),'Titanium',body,side*.35)
 for x in [-1,1]:
  for i in range(3):box('Rib plating',(x*.22,-.12+i*.066,.018),(.06,.044,.22),'Gunmetal',body,.014)

 for j in range(3):armor('Articulated abdomen',(0,-.14-j*.065,.075),[(-.035,.19-j*.015,.18,0),(.035,.26-j*.025,.21,0)],'Gunmetal',body)
 armor('Reactor backpack',(0,.07,-.18),[(-.21,.29,.15,0),(-.04,.4,.23,-.015),(.18,.33,.2,0)],color,body)
 box('Backpack spine',(0,.045,-.31),(.09,.31,.035),'Titanium',body,.01)
 for j in range(5):box('Backpack ventilation',(0,-.06+j*.049,-.335),(.16,.017,.021),'Obsidian',body,.003)
 for side in [-1,1]:
  box('Rear identifier',(side*.145,.13,-.302),(.045,.12,.021),'CyanLight',body,.006)
  beam('Braided power feed',(side*.15,-.11,-.2),(side*.21,-.25,-.05),.025,'Copper',body)
 for x in [-.12,.12]:
  cyl('Thruster',(x,.05,-.26),.061,.24,'Titanium',body);cyl('Thruster port',(x,-.09,-.26),.043,.04,'CyanLight',body)
 head=group('Head',body,(0,.39,0));armor('Faceted command helmet',(0,0,0),[(-.17,.22,.2,.018),(-.1,.34,.31,0),(.1,.39,.34,-.02),(.21,.26,.24,-.035)],color,head)
 armor('Visor frame',(0,0,.149),[(-.075,.28,.04,0),(.095,.365,.075,0),(.125,.32,.05,0)],'Obsidian',head)
 uv('Visible human face',(0,-.027,.19),(.136,.145,.055),'Skin'+color,head,20)
 uv('Nose bridge',(0,-.025,.244),(.023,.041,.027),'Skin'+color,head)
 for side in [-1,1]:
  uv('Human eye',(side*.059,.024,.239),(.033,.011,.009),'EyeWhite',head,12)
  uv('Human iris',(side*.06,.024,.251),(.01,.009,.004),'EyeIris',head,12)
  slash((side*.06,.058,.242),(.072,.016,.012),'Fabric',head,-side*.12)
  armor('Cheek armor',(side*.142,-.079,.14),[(-.068,.063,.072,0),(.054,.098,.092,0)],'Ivory',head)
 armor('Helmet crest',(0,.14,-.04),[(-.025,.068,.31,0),(.06,.043,.21,-.02)],'Ivory',head)
 armor('Chin guard',(0,-.125,.16),[(-.05,.16,.045,0),(.028,.22,.063,0)],'Gunmetal',head)
 box('Lower respirator',(0,-.126,.227),(.14,.052,.031),'Fabric',head,.015)
 for x in [-.043,0,.043]:box('Respirator valve',(x,-.126,.247),(.019,.026,.012),'Titanium',head,.003)
 for x in [-1,1]:
  cyl('Comms',(x*.19,.02,0),.063,.047,'Titanium',head,'X');cyl('Comms light',(x*.219,.02,0),.033,.012,'CyanLight',head,'X')
 cyl('Antenna',(.2,.18,-.05),.012,.17,'Titanium',head);uv('Antenna tip',(.2,.27,-.05),(.022,.022,.022),'AmberLight',head)
 box('Pelvis',(0,-.015,0),(.32,.2,.23),'Fabric',hips,.055)
 box('Utility belt',(0,.037,.024),(.39,.075,.29),'Obsidian',hips,.026)
 box('Belt buckle',(0,.037,.176),(.076,.061,.025),'Titanium',hips,.01)
 for side in [-1,1]:
  box('Ammo pouch',(side*.165,-.039,.152),(.106,.14,.075),'Fabric',hips,.018)
  box('Pouch flap',(side*.165,.018,.193),(.097,.038,.018),color,hips,.008)
  beam('Chest harness',(side*.2,.2,.228),(side*.15,-.13,.198),.019,'Fabric',body)
  box('Pack side pouch',(side*.216,.001,-.147),(.13,.22,.15),'Fabric',body,.025)
  for j in range(3):box('Magazine rib',(side*.224,-.059+j*.051,-.228),(.07,.019,.015),'Titanium',body,.003)
 limbs=[]
 for side,x in [('L',-.19),('R',.19)]:
  leg=group('UpperLeg'+side,hips,(x,-.075,0));cyl('Hip rotary',(0,0,0),.09,.17,'Titanium',leg,'X');armor('Tapered thigh',(0,0,0),[(-.33,.15,.18,.02),(-.17,.2,.23,.025),(-.035,.18,.2,0)],color,leg)
  shin=group('LowerLeg'+side,leg,(0,-.35,0));uv('Knee actuator',(0,0,.025),(.095,.095,.105),'Obsidian',shin);box('Kneepad',(0,0,.1),(.15,.115,.06),'Ivory',shin,.025)
  armor('Shin shell',(0,0,.04),[(-.32,.155,.17,0),(-.15,.19,.2,.005),(-.04,.16,.18,0)],color,shin);box('Shin light',(0,-.15,.117),(.036,.15,.013),'CyanLight',shin,.007)
  box('Boot',(0,-.39,.065),(.22,.14,.34),'Obsidian',shin,.045);box('Toe cap',(0,-.37,.191),(.21,.09,.11),'Titanium',shin,.025)
  arm=group('UpperArm'+side,body,(x*1.8,.18,0));uv('Shoulder actuator',(0,0,0),(.105,.105,.105),'Titanium',arm)
  armor('Swept shoulder pauldron',(0,0,0),[(-.13,.2,.23,.015),(.045,.29,.31,0),(.12,.21,.23,-.025)],color,arm);box('Shoulder rank',(0,.134,-.015),(.13,.022,.085),'Ivory',arm,.007);uv('Suit bicep',(0,-.17,.015),(.085,.15,.085),'Fabric',arm);armor('Bicep guard',(0,-.15,.069),[(-.09,.115,.085,0),(.065,.15,.09,0)],color,arm)
  fore=group('Forearm'+side,arm,(0,-.3,.015));uv('Elbow',(0,0,0),(.075,.075,.075),'Titanium',fore)
  box('Forearm armor',(0,-.1,.115),(.16,.17,.27),color,fore,.04);box('Glove',(0,-.14,.275),(.12,.105,.14),'Fabric',fore,.025)
  for digit in [-.04,-.013,.013,.04]:box('Armored finger',(digit,-.144,.345),(.021,.075,.026),'Gunmetal',fore,.006)
  box('Wrist light',(0,-.004,.14),(.065,.015,.095),'CyanLight',fore,.005)
  limbs.append((leg,shin,arm,fore))
 rifle=weapon(limbs[1][3],(0,-.11,.33))
 bpy.context.scene.render.fps=30
 animated=[hips,head]+[o for ll in limbs for o in ll]
 for clip,length in [('Idle',60),('Run',24),('Shoot',10),('Death',35)]:
  for o in animated:
   base=o.location.copy();keys=[]
   for f in [1,1+length//4,1+length//2,1+3*length//4,1+length]:
    t=(f-1)/length;rot=[0,0,0];loc=base.copy()
    if o.name.startswith('UpperArm'):rot[0]=-.95
    if o.name.startswith('Forearm'):rot[0]=.95
    if clip=='Idle':
     if o==hips:loc.z+=math.sin(t*math.tau)*.012
     if o==head:rot[2]=math.sin(t*math.tau)*.07
    elif clip=='Run':
     phase=1 if o.name.endswith('L') else -1
     if o.name.startswith('UpperLeg'):rot[0]=math.sin(t*math.tau)*.65*phase
     if o.name.startswith('LowerLeg'):rot[0]=max(0,math.sin(t*math.tau)*phase)*.85
     if o.name.startswith('UpperArm'):rot[0]+=math.sin(t*math.tau)*.1*-phase
     if o==hips:loc.z+=abs(math.sin(t*math.tau))*.055;rot[0]=-.1
    elif clip=='Shoot':
     if o.name=='UpperArmR':rot[0]+=math.sin(t*math.pi)*.1
     if o==hips:loc.y+=math.sin(t*math.pi)*.035
    elif clip=='Death':
     if o==hips:rot[0]=-t*1.42;loc.z-=t*.64
     elif o.name.startswith('UpperArm'):rot[1]=t*(.55 if o.name.endswith('L') else -.55)
     elif o==head:rot[0]=t*.4
    keys.append((f,rot,loc))
   anim(o,clip,length,keys)
 export('fighter-'+color.lower()+'.glb')
 render('fighter-'+color.lower(),(3,2.1,4),(0,.95,0),4)
 if color=='Azure':render('fighter-rear',(3,2,-4),(0,.95,0),4)
def kart(color):
 reset();g=group('Kart')
 box('Carbon chassis',(0,.28,0),(1.22,.18,1.95),'Obsidian',g,.11)
 armor('Swept monocoque',(0,0,0),[(-.6,.92,.26,.4),(-.15,1.05,.3,.43),(.42,.83,.31,.45),(.94,.62,.22,.4),(1.23,.4,.11,.32)],color,g,True)
 armor('Hood race stripe',(0,0,0),[(.37,.13,.013,.61),(.91,.105,.015,.516),(1.18,.08,.012,.39)],'Ivory',g,True)
 for x in [-1,1]:
  armor('Nose canard',(x*.49,0,0),[(.65,.18,.08,.29),(1.08,.23,.05,.29),(1.25,.12,.035,.27)],'Gunmetal',g,True)
  box('Front running lamp',(x*.24,.4,1.055),(.14,.04,.03),'CyanLight',g,.01)
 box('Front splitter',(0,.24,1.14),(1.4,.08,.25),'Gunmetal',g,.03)
 for x in [-1,1]:
  armor('Sculpted sidepod',(x*.55,0,0),[(-.67,.25,.2,.39),(-.35,.35,.36,.4),(.24,.31,.28,.39),(.57,.19,.19,.35)],color,g,True)
  box('Side sill',(x*.58,.25,-.03),(.29,.08,1.28),'Gunmetal',g,.025)
  box('Sidepod intake',(x*.55,.42,.39),(.26,.15,.05),'Obsidian',g,.025)
  for j in range(3):box('Intake fin',(x*.55,.375+j*.045,.426),(.2,.018,.015),'Titanium',g,.003)
  beam('Wing support',(x*.47,.48,-.79),(x*.47,.82,-.93),.04,'Titanium',g)
  box('Wing endplate',(x*.74,.89,-.97),(.065,.26,.45),color,g,.025)
 armor('Dual element rear wing',(0,0,0),[(-1.2,1.48,.025,.92),(-1.02,1.5,.11,.83),(-.82,1.47,.055,.83)],color,g,True)
 box('Wing upper blade',(0,.94,-1.11),(1.37,.035,.11),'Gunmetal',g,.01)
 for x in [-.43,.43]:box('Wing stripe',(x,.899,-.965),(.13,.02,.19),'Ivory',g,.004)
 box('Rear luminous bar',(0,.79,-1.206),(1.24,.03,.025),'CyanLight',g,.006)
 armor('Engine housing',(0,0,0),[(-.85,.54,.28,.44),(-.6,.66,.4,.48),(-.37,.51,.35,.48)],'Gunmetal',g,True)
 box('Rear diffuser',(0,.19,-.94),(1.25,.07,.37),'Obsidian',g,.012)
 for x in [-.46,-.23,0,.23,.46]:box('Diffuser vane',(x,.25,-1.04),(.025,.17,.3),'Titanium',g,.008)
 for x in [-.49,.49]:
  box('Tail lamp housing',(x,.44,-.93),(.17,.11,.1),'Obsidian',g,.024)
  box('Tail lamp',(x,.46,-.989),(.125,.035,.02),'PinkLight',g,.006)
  beam('Roll cage diagonal',(x*.7,.4,-.61),(x*.6,.89,-.35),.03,'Titanium',g)
 for x in [-.18,.18]:
  cyl('Turbine',(x,.45,-.83),.115,.25,'Titanium',g,'Z');cyl('Exhaust glow',(x,.45,-.965),.083,.025,'AmberLight',g,'Z')
 for i in range(6):box('Motor cooling fin',(0,.68,-.74+i*.055),(.48,.025,.018),'Titanium',g,.003)
 box('Seat',(0,.4,-.16),(.48,.16,.55),'Rubber',g,.09);box('Seat back',(0,.68,-.4),(.51,.56,.13),'Rubber',g,.09)
 wheels=[]
 for tag,x,z in [('FL',-.75,.67),('FR',.75,.67),('RL',-.75,-.66),('RR',.75,-.66)]:
  wheel=group('Wheel'+tag,g,(x,.31,z));wheels.append(wheel)
  torus('Tire',(0,0,0),.225,.088,'Rubber',wheel,'X')
  for tread in range(24):
   a=tread*math.tau/24
   for lane in [-1,1]:
    o=box('Tread block',(lane*.041,.311*math.cos(a),.311*math.sin(a)),(.066,.017,.044),'Obsidian',wheel,0);o.rotation_euler[0]=a
  for side in [-1,1]:torus('Sidewall bead',(side*.088,0,0),.246,.012,'Obsidian',wheel,'X')
  cyl('Brake disc',(0,0,0),.163,.15,'Copper',wheel,'X')
  cyl('Alloy rim',(0,0,0),.19,.19,'Gunmetal',wheel,'X')
  for side in [-1,1]:
   cyl('Rim lip',(side*.105,0,0),.18,.026,'Titanium',wheel,'X');cyl('Rim face',(side*.122,0,0),.14,.016,color,wheel,'X');cyl('Hub',(side*.135,0,0),.055,.025,'Titanium',wheel,'X')
   for a in range(6):
    ang=a*math.tau/6;beam('Spoke',(side*.14,.04*math.cos(ang),.04*math.sin(ang)),(side*.14,.133*math.cos(ang),.133*math.sin(ang)),.017,'Titanium',wheel)
  beam('Suspension',(x*.6,.28,z),(x,.29,z),.045,'Titanium',g)
 group('DriverSocket',g,(0,.67,-.15))
 torus('SteeringWheel',(0,.64,.21),.21,.024,'Rubber',g,'Z');beam('Steering column',(0,.48,.27),(0,.64,.21),.026,'Titanium',g)
 export('kart-'+color.lower()+'.glb');render('kart-'+color.lower(),(3.3,2.3,4),(0,.65,0),4.3)
 if color=='Azure':render('kart-rear',(3,2.1,-4),(0,.65,0),4.3)
def rock(g,p,s,m):
 bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2,radius=1,location=xyz(p));o=bpy.context.object
 for v in o.data.vertices:v.co*=random.uniform(.82,1.15)
 o.scale=(s[0],s[2],s[1]);finish(o,'Weathered rock',m,g)
def env():
 reset()
 g=group('CargoCrate');box('Container',(0,.7,0),(1.4,1.4,1.2),'Gunmetal',g,.08)
 for x in [-1,1]:box('Reinforced corner',(x*.66,.7,0),(.14,1.46,1.23),'Titanium',g,.027)
 for z in [-1,1]:
  panel((0,.71,z*.62),(1.08,.97,.08),g)
  for y in [.25,1.15]:box('Warning strip',(0,y,z*.67),(.75,.055,.02),'Yellow',g,.004)
  box('Latch',(0,.7,z*.69),(.12,.24,.06),'Obsidian',g,.014)
 g=group('Barrier');box('Wide base',(0,.12,0),(3.2,.24,.85),'Gunmetal',g,.06);box('Armored cover',(0,.7,0),(3,1.15,.4),'Ivory',g,.08)
 for x in [-1,1]:box('Cover post',(x*1.28,.75,0),(.25,1.42,.52),'Azure',g,.045)
 box('Cover inset',(0,.74,.216),(2.22,.69,.04),'Gunmetal',g,.027);box('Upper accent',(0,1.25,.23),(2.15,.038,.027),'CyanLight',g,.007)
 for i in range(7):box('Armor rib',(-.96+i*.32,.72,.251),(.045,.57,.045),'Titanium',g,.008)
 g=group('StationColumn');box('Foot',(0,.17,0),(1.35,.34,1.35),'Gunmetal',g,.08);box('Column',(0,2,0),(.85,3.8,.85),'Ivory',g,.09);box('Capital',(0,3.8,0),(1.4,.3,1.4),'Gunmetal',g,.04)
 for x in [-1,1]:
  for z in [-1,1]:box('Luminous channel',(x*.35,2,z*.44),(.085,3.05,.04),'CyanLight',g,.01)
 g=group('StationWall');box('Bulkhead',(0,1.5,0),(4,3,.32),'Gunmetal',g,.04)
 for x in [-1.05,1.05]:panel((x,1.5,.18),(1.85,2.56,.09),g,'Ivory');box('Display',(x,1.6,.236),(1.25,.55,.025),'Glass',g,.035)
 for y in [.19,2.81]:box('Light strip',(0,y,.23),(3.7,.05,.035),'CyanLight',g,.008)
 g=group('Reactor');cyl('Reactor plinth',(0,.2,0),1.1,.4,'Gunmetal',g,verts=24);cyl('Core',(0,1.6,0),.61,2.6,'Glass',g,verts=24)
 for a in range(12):
  t=a*math.tau/12;beam('Reactor energy channel',(.618*math.cos(t),.4,.618*math.sin(t)),(.618*math.cos(t),2.8,.618*math.sin(t)),.022,'CyanLight',g)
 for y in [.5,1.15,2,2.75]:torus('Confinement ring',(0,y,0),.78,.13,'Titanium',g)
 for a in range(6):
  t=a*math.tau/6;x,z=.84*math.cos(t),.84*math.sin(t);box('Containment pillar',(x,1.6,z),(.16,2.7,.16),'Gunmetal',g,.03)
 g=group('FoundryFurnace');box('Furnace body',(0,1.3,0),(2.3,2.6,1.65),'Gunmetal',g,.13);box('Hot interior',(0,1.15,.846),(1.53,1.24,.04),'AmberLight',g,.06)
 for x in [-1,1]:box('Door jamb',(x*.9,1.1,.97),(.25,1.7,.27),'Copper',g,.035)
 for y in [.35,1.9]:box('Door lintel',(0,y,.97),(2,.19,.27),'Titanium',g,.03)
 for i in range(8):box('Furnace grate',(-.7+i*.2,1.12,.945),(.07,1.4,.09),'Obsidian',g,.014)
 for x in [-.68,.68]:cyl('Chimney',(x,3,0),.22,1.2,'Copper',g);torus('Chimney band',(x,3.35,0),.23,.055,'Titanium',g)
 g=group('PipeRack')
 for x in [-1,1]:box('Rack upright',(x*1.35,1.2,0),(.17,2.4,.55),'Gunmetal',g,.025)
 for y in [.5,1.2,1.9]:
  cyl('Transport pipe',(0,y,0),.17,3.1,'Copper',g,'X')
  for x in [-1.05,0,1.05]:torus('Pipe coupling',(x,y,0),.18,.045,'Titanium',g,'X')
 g=group('DesertRock');rock(g,(0,1.15,0),(1.45,1.4,1.05),'Sand');rock(g,(.85,.45,.35),(.74,.6,.65),'SandLight')
 g=group('DesertArch')
 for x in [-2.5,2.5]:
  rock(g,(x,1.7,0),(1.05,2.2,1.03),'Sand');rock(g,(x*.9,3.2,0),(1.2,1.4,.95),'SandLight')
 rock(g,(0,4.1,0),(2.6,.9,.95),'Sand')
 g=group('PalmTree')
 for i in range(9):
  x=math.sin(i*.13)*.6;cone('Palm trunk',(x,.2+i*.38,0),.19-i*.007,.165-i*.007,.43,'Bark',g)
 crown=(.52,3.65,0)
 for a in range(9):
  t=a*math.tau/9;forward=Vector((math.cos(t),0,math.sin(t)));right=Vector((-math.sin(t),0,math.cos(t)))
  vertices=[];faces=[]
  def center(u):return Vector((.52,3.73+.48*math.sin(u*math.pi)-u*.75,0))+forward*(u*2.25)
  for j in range(9):
   u=j/9;beam('Frond central rib',center(u),center((j+1)/9),.013,'LeafLight',g)
   if j==0:continue
   width=.46*math.sin(u*math.pi)**.5
   for side in [-1,1]:
    base=center(u-.035);tip=center(u+.15)+right*width*side;ridge=center(u+.045)+right*width*.45*side;ridge.y+=.075
    points=[base,center(u+.045),tip,ridge];start=len(vertices);vertices.extend(xyz(v) for v in points)
    faces.extend([(start,start+1,start+3),(start+1,start+2,start+3),(start+2,start,start+3)])
  mesh=bpy.data.meshes.new('Feathered palm frond');mesh.from_pydata(vertices,[],faces);mesh.update();obj=bpy.data.objects.new('Feathered palm frond',mesh);bpy.context.collection.objects.link(obj);finish(obj,'Feathered palm frond','Leaf' if a%2 else 'LeafLight',g)
 for x in [-.15,.15]:uv('Coconut',(.52+x,3.5,.1),(.12,.16,.13),'Bark',g)
 g=group('PineTree');cyl('Trunk',(0,1.6,0),.18,3.2,'Bark',g)
 for y,r,h in [(1.3,1.3,1.5),(2,1.08,1.5),(2.75,.8,1.35),(3.35,.54,1.2)]:
  cone('Pine boughs',(0,y,0),r,.035,h,'Leaf',g,11)
  cone('Sunlit boughs',(0,y+.12,0),r*.91,.02,h*.88,'LeafLight',g,11)
 g=group('BroadleafTree');cone('Trunk',(0,1.3,0),.32,.16,2.6,'Bark',g)
 for a in range(6):
  t=a*math.tau/6;x,z=math.cos(t)*.85,math.sin(t)*.85;beam('Branch',(0,1.3,0),(x,2.8,z),.085,'Bark',g);rock(g,(x,3,z),(1.05,.85,1.05),'Leaf' if a%2 else 'LeafLight')
 rock(g,(0,3.65,0),(1.1,.9,1.1),'LeafLight')
 g=group('CoastalRock');rock(g,(0,.48,0),(1.2,.7,.85),'Rock');rock(g,(-.55,.35,.38),(.55,.48,.62),'SandLight')
 g=group('NeonTower');box('Tower',(0,4,0),(3.2,8,2.6),'Gunmetal',g,.12);box('Roof',(0,8.05,0),(3.5,.2,2.9),'Obsidian',g,.05)
 for z in [-1,1]:
  for ix in range(5):
   for iy in range(11):box('Window',(-1.2+ix*.6,.7+iy*.63,z*1.313),(.28,.32,.025),'CyanLight' if (ix+iy)%4 else 'PinkLight',g,.012)
 for x in [-1.58,1.58]:box('Tower neon',(x,4,1.34),(.045,7.6,.025),'PinkLight',g,.008)
 g=group('NeonSign');box('Sign foot',(0,.1,0),(1.1,.2,.7),'Gunmetal',g);box('Sign post',(0,1,0),(.18,2,.18),'Titanium',g);box('Signboard',(0,2,0),(2.8,1.1,.2),'Obsidian',g,.12)
 for y in [1.51,2.49]:box('Sign trim',(0,y,.115),(2.52,.025,.018),'PinkLight',g,.008)
 for i in range(3):
  for sign in [-1,1]:
   o=box('Arrow',(-.65+i*.6,2+sign*.16,.13),(.4,.065,.025),'CyanLight',g,.012);o.rotation_euler[1]=sign*-.7
 g=group('RaceGantry')
 for x in [-4.5,4.5]:
  box('Gantry base',(x,.2,0),(1.1,.4,1.5),'Gunmetal',g,.09);box('Upright',(x,2,0),(.4,4,.4),'Titanium',g,.04);box('Banner support',(x,3.5,0),(.7,.6,.7),'Azure',g,.06)
 box('Gantry header',(0,4.15,0),(9.7,.85,.5),'Gunmetal',g,.07)
 for i in range(24):
  for j in range(2):box('Checkered finish',(-4.5+i*.39,3.95+j*.36,.27),(.36,.34,.025),'Ivory' if (i+j)%2 else 'Obsidian',g,.001)
 for x in [-1,0,1]:uv('Start lamp',(x*.6,3.46,.05),(.17,.17,.1),'AmberLight',g)
 g=group('TireStack')
 for y in [.13,.36,.59]:torus('Tire',(0,y,0),.31,.12,'Rubber',g);cyl('Tire interior',(0,y,0),.2,.15,'Obsidian',g)
 g=group('TrackBarrier');box('Barrier base',(0,.12,0),(3,.24,.7),'Gunmetal',g,.04);box('Safety wall',(0,.49,0),(2.9,.72,.38),'Ivory',g,.05)
 for side in [-1,1]:
  for x in [-1.16,-.39,.39,1.16]:
   o=box('Racing stripe',(x,.5,side*.207),(.38,.59,.026),'Ember',g,.006);o.rotation_euler[1]=.22
  box('Barrier rail',(0,.79,side*.217),(2.71,.036,.03),'Titanium',g,.006)
 g=group('BoostPad');box('Boost base',(0,.025,0),(2.4,.05,3),'Gunmetal',g,.08)
 for j in range(4):
  for sign in [-1,1]:
   o=box('Boost chevron',(sign*.36,.057,-.95+j*.58),(.95,.02,.13),'CyanLight',g,.015);o.rotation_euler[2]=sign*.52
 g=group('Pickup');uv('Pickup core',(0,.6,0),(.27,.27,.27),'CyanLight',g);torus('Pickup orbit',(0,.6,0),.42,.05,'Titanium',g,'Z');torus('Pickup orbit',(0,.6,0),.42,.05,'Yellow',g,'X')
 export('environment-kit.glb')
 # Stage representative kit as a contact sheet; exported roots remain at zero.
 roots=[o for o in bpy.context.scene.objects if o.parent is None]
 for i,o in enumerate(roots):o.location=xyz(((i%5)*7,0,-(i//5)*8))
 render('environment-kit',(37,31,34),(14,1,-10),43)
def render(n,cam,target,scale):
 scene=bpy.context.scene
 scene.render.engine='CYCLES';scene.cycles.samples=24;scene.cycles.use_denoising=True
 scene.render.resolution_x=1200;scene.render.resolution_y=1000;scene.render.resolution_percentage=100
 scene.world.color=(.12,.12,.12)
 floor=box('Preview ground',(target[0],-.06,target[2]),(100,.12,100),'Road',bevel=0)
 bpy.ops.object.camera_add(location=xyz(cam));camera=bpy.context.object;camera.rotation_euler=(Vector(xyz(target))-camera.location).to_track_quat('-Z','Y').to_euler();camera.data.type='ORTHO';camera.data.ortho_scale=scale;scene.camera=camera
 lights=[]
 for p,power,size,col in [((4,8,5),1700,6,(.72,.88,1)),((-4,4,1),1100,5,(1,.52,.28)),((0,6,-5),2100,4,(.15,.62,1))]:
  bpy.ops.object.light_add(type='AREA',location=xyz((p[0]+target[0],p[1]+target[1],p[2]+target[2])));o=bpy.context.object;o.data.energy=power;o.data.shape='DISK';o.data.size=size;o.data.color=col;o.rotation_euler=(Vector(xyz(target))-o.location).to_track_quat('-Z','Y').to_euler();lights.append(o)
 scene.view_settings.view_transform='AgX';scene.render.image_settings.file_format='PNG';scene.render.filepath=str(PRE/(n+'.png'));bpy.ops.render.render(write_still=True)
 for o in lights+[camera,floor]:bpy.data.objects.remove(o,do_unlink=True)
def manifest():
 files={}
 for p in sorted(OUT.glob('*.glb')):
  data=p.read_bytes();length,kind=struct.unpack_from('<II',data,12);doc=json.loads(data[20:20+length]);tris=sum(doc['accessors'][q['indices']]['count']//3 for m in doc.get('meshes',[]) for q in m['primitives'])
  files[p.name]={'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest(),'triangles':tris,'nodes':[x.get('name') for x in doc.get('nodes',[])],'animations':[x.get('name') for x in doc.get('animations',[])],'meshes':len(doc.get('meshes',[])),'materials':len(doc.get('materials',[]))}
 previews={p.name:{'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'width':struct.unpack_from('>I',p.read_bytes(),16)[0],'height':struct.unpack_from('>I',p.read_bytes(),20)[0]} for p in sorted(OUT.glob('*.png'))}
 (OUT/'manifest.json').write_text(json.dumps({'version':1,'units':'metres','up':'+Y','forward':'+Z','texture_max_px':0,'files':files,'previews':previews},indent=2)+'\n')
 print(json.dumps({k:{x:v[x] for x in ['bytes','triangles','animations','meshes']} for k,v in files.items()},indent=2))
 assert sum(f['bytes'] for f in files.values())+sum(f['bytes'] for f in previews.values())<20*1024**2
if __name__ == '__main__':
 palette()
 for c in ['Azure','Ember']:fighter(c);kart(c)
 env();reset();rifle=weapon();rifle.name='Rifle';export('rifle.glb');manifest()
