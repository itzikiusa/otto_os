"""Original expressive drivers and adventure scenery, built in Blender 5.2."""
import bpy, math, sys, random, json
from pathlib import Path
from mathutils import Vector
P=Path(__file__).resolve();sys.path.insert(0,str(P.parent));import build_assets as A
A.palette();random.seed(484)
for n,c,r in [('FoxFur',(.72,.19,.025),.82),('FoxDark',(.28,.048,.009),.9),('FurCream',(.86,.74,.51),.94),('PandaWhite',(.81,.84,.78),.9),('PandaBlack',(.017,.026,.037),.84),('RabbitFur',(.52,.47,.67),.83),('InnerEar',(.7,.23,.26),.85),('EyeWhite',(.98,.98,.9),.27),('EyeBlack',(.003,.007,.014),.12),('IrisGold',(.86,.39,.027),.25),('IrisGreen',(.032,.4,.19),.25),('SuitTeal',(.009,.19,.19),.66),('SuitBerry',(.4,.019,.16),.68),('SuitGold',(.69,.34,.028),.66),('WoodPaint',(.1,.33,.28),.9),('RoofRed',(.34,.048,.022),.95),('Plaster',(.66,.5,.29),.98),('CoralRose',(.72,.08,.19),.85),('CoralGold',(.92,.38,.03),.78),('OceanLeaf',(.008,.25,.19),.83),('FishBlue',(.014,.19,.56),.5)]:A.M[n]=A.mat(n,c,0,r)
def uv(n,p,s,m,g,seg=16):return A.uv(n,p,s,m,g,seg)
def eye(g,x,y,z,iris='IrisGold',width=.065,height=.085):
 uv('Eye white',(x,y,z),(width,height,.039),'EyeWhite',g)
 uv('Colored iris',(x,y+.003,z+.033),(width*.61,height*.66,.018),iris,g)
 uv('Bright pupil',(x,y+.005,z+.049),(width*.35,height*.48,.012),'EyeBlack',g)
 uv('Eye catchlight',(x-.014,y+.029,z+.06),(.014,.019,.007),'EyeWhite',g,12)
def curve(n,points,r,m,g):
 # Smooth segmented curves remain editable mesh geometry in the export.
 for a,b in zip(points,points[1:]):A.beam(n,a,b,r,m,g)
 for p in points[1:-1]:uv(n,p,(r,r,r),m,g,12)
def ear(g,n,x,y,z,w,h,fur,lean=0):
 # Soft tapered animal ears with a separate inset; bases merge into the head.
 sections=[(0,w,.115,0),(h*.38,w*.86,.1,0),(h*.83,w*.46,.06,lean*.45),(h,w*.12,.027,lean)]
 outer=A.armor(n,(x,y,z),sections,fur,g)
 inner=A.armor(n+' inner',(x,y+.048,z+.068),[(0,w*.62,.015,0),(h*.36,w*.55,.018,0),(h*.73,w*.24,.014,lean*.4),(h*.83,.013,.008,lean*.8)],'InnerEar',g)
 return outer

def animal(kind):
 name=kind.title();g=A.group('Driver'+name);fur={'fox':'FoxFur','panda':'PandaWhite','rabbit':'RabbitFur','robot':'SuitTeal'}[kind];suit={'fox':'SuitTeal','panda':'SuitGold','rabbit':'SuitBerry','robot':'Ivory'}[kind]
 uv('Plump racing suit',(0,.055,0),(.24,.29,.205),suit,g,20)
 uv('Suit belly',(0,.025,.142),(.18,.22,.083),'Ivory' if kind=='robot' else 'FurCream',g,16)
 A.torus('Ribbed suit collar',(0,.245,0),.14,.042,suit,g)
 for x in [-1,1]:
  # Bent elbows, paws at the wheel, seated thighs and racing boots.
  curve('Soft racing sleeve',[(x*.17,.17,.015),(x*.25,.075,.18),(x*.25,-.027,.36)],.076,suit,g)
  uv('Wheel gripping paw',(x*.25,-.029,.365),(.09,.075,.08),fur if kind!='robot' else 'Gunmetal',g)
  for digit in [-.034,0,.034]:uv('Finger',(x*.25+digit,-.041,.421),(.018,.046,.023),fur if kind!='robot' else 'Titanium',g,12)
  uv('Seated thigh',(x*.125,-.13,.17),(.105,.106,.22),suit,g)
  uv('Race boot',(x*.13,-.205,.4),(.11,.085,.15),'Gunmetal',g)
  A.box('Boot trim',(x*.13,-.196,.529),(.13,.035,.014),'Ivory',g,.006)
  A.box('Suit shoulder stripe',(x*.215,.2,.043),(.09,.025,.14),'Ivory',g,.008)
 # Small zipper, racing badge and five-point harness read without textures.
 A.box('Suit zipper',(0,.11,.218),(.018,.14,.012),'Gunmetal',g,.004)
 uv('Racing badge',(-.105,.145,.189),(.042,.047,.018),'Yellow',g)
 head=A.group(name+'Head',g,(0,.445,.015))
 if kind=='fox':
  uv('Fox head',(0,0,0),(.305,.278,.245),'FoxFur',head,20)
  for side in [-1,1]:
   uv('Cheek ruff',(side*.22,-.08,.065),(.14,.12,.17),'FurCream',head)
   ear(head,'Pointed fox ear',side*.19,.185,-.028,.18,.29,'FoxFur',-.01)
   eye(head,side*.112,.036,.222,'IrisGold')
   o=A.box('Playful brow',(side*.109,.147,.218),(.13,.029,.03),'FoxDark',head,.012);o.rotation_euler[1]=-side*.13
   uv('Fox muzzle',(side*.068,-.093,.226),(.097,.074,.098),'FurCream',head)
  uv('Fox nose',(0,-.053,.321),(.052,.039,.035),'EyeBlack',head)
  curve('Fox smile',[(-.092,-.144,.27),(0,-.168,.283),(.105,-.13,.27)],.009,'FoxDark',head)
  uv('Forehead blaze',(0,.142,.191),(.053,.104,.027),'FurCream',head)
  tail=A.group('FoxTail',g,(-.13,-.085,-.14));curve('Fox tail',[(-.04,0,0),(-.23,.02,-.08),(-.37,.16,-.09),(-.39,.35,-.02)],.105,'FoxFur',tail)
  uv('White tail tip',(-.39,.39,0),(.1,.14,.1),'FurCream',tail)
  # Neck scarf gives the driver a recognizable accent from behind.
  A.torus('Fox scarf',(0,.285,-.008),.145,.034,'Ember',g)
  A.armor('Trailing scarf',(.12,.24,-.16),[(-.2,.1,.018,-.17),(-.03,.14,.018,-.02),(.04,.1,.022,0)],'Ember',g)
 elif kind=='panda':
  uv('Panda head',(0,0,0),(.32,.282,.256),'PandaWhite',head,20)
  for side in [-1,1]:
   uv('Round panda ear',(side*.229,.222,-.035),(.12,.13,.075),'PandaBlack',head)
   uv('Ear inset',(side*.229,.223,.032),(.069,.079,.015),'FoxDark',head)
   patch=uv('Eye patch',(side*.119,.025,.227),(.095,.124,.048),'PandaBlack',head);patch.rotation_euler[1]=side*.23
   eye(head,side*.11,.048,.268,'IrisGreen',.054,.067)
   uv('Plump cheek',(side*.162,-.101,.19),(.113,.085,.08),'PandaWhite',head)
  uv('Panda muzzle',(0,-.092,.249),(.119,.077,.054),'FurCream',head)
  uv('Panda nose',(0,-.056,.3),(.048,.037,.025),'EyeBlack',head)
  curve('Panda smile',[(-.065,-.126,.291),(0,-.146,.295),(.065,-.12,.29)],.007,'PandaBlack',head)
  uv('Tiny panda tail',(0,-.035,-.243),(.085,.085,.075),'PandaBlack',g)
  A.torus('Panda racing band',(0,.277,0),.146,.026,'Azure',g)
 elif kind=='rabbit':
  uv('Rabbit head',(0,0,0),(.285,.272,.235),'RabbitFur',head,20)
  ear(head,'Tall rabbit ear',-.15,.175,-.043,.135,.5,'RabbitFur',.045)
  ear(head,'Swept rabbit ear',.145,.182,-.042,.145,.44,'RabbitFur',-.1)
  for side in [-1,1]:
   eye(head,side*.107,.039,.207,'IrisGreen',.062,.082)
   uv('Rabbit cheek',(side*.116,-.104,.194),(.12,.079,.075),'PandaWhite',head)
   uv('Rosy cheek',(side*.211,-.086,.166),(.04,.033,.019),'InnerEar',head)
   for j in range(2):curve('Whisker',[(side*.19,-.09-j*.028,.213),(side*.31,-.09-j*.045,.207)],.004,'Ivory',head)
  uv('Bunny nose',(0,-.058,.271),(.039,.031,.024),'InnerEar',head)
  for x in [-.021,.021]:A.box('Buck tooth',(x,-.158,.243),(.036,.065,.025),'EyeWhite',head,.009)
  uv('Cotton tail',(0,-.08,-.244),(.125,.125,.12),'PandaWhite',g)
  A.torus('Rabbit neck warmer',(0,.272,0),.145,.027,'Yellow',g)
 elif kind=='robot':
  uv('Round robot shell',(0,0,0),(.29,.267,.237),'SuitTeal',head,20)
  A.box('Friendly face frame',(0,.016,.181),(.45,.31,.133),'Gunmetal',head,.09)
  A.box('Face glass',(0,.013,.255),(.394,.26,.048),'Glass',head,.07)
  for side in [-1,1]:
   uv('Digital eye',(side*.099,.047,.283),(.047,.065,.016),'CyanLight',head)
   uv('Digital catchlight',(side*.11,.07,.296),(.012,.013,.007),'EyeWhite',head,12)
   A.cyl('Ear speaker',(side*.287,.007,0),.087,.046,'Ivory',head,'X')
   A.cyl('Ear center',(side*.316,.007,0),.046,.015,'Yellow',head,'X')
  curve('Digital smile',[(-.061,-.073,.287),(0,-.094,.291),(.061,-.073,.287)],.012,'CyanLight',head)
  A.beam('Antenna',(0,.23,-.055),(0,.38,-.055),.017,'Titanium',head)
  uv('Antenna signal',(0,.405,-.055),(.05,.05,.05),'Yellow',head)
  for j in range(3):A.box('Robot status',(0,.11-j*.065,.22),(.095,.022,.018),'CyanLight',g,.007)
 return g

def portrait(root,kind):
 for o in bpy.context.scene.objects:
  if o.parent is None:
   o.hide_render=(o!=root)
   for child in o.children_recursive:child.hide_render=(o!=root)
 scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.use_denoising=True
 scene.render.resolution_x=640;scene.render.resolution_y=640;scene.render.resolution_percentage=100;scene.render.film_transparent=True
 bpy.ops.object.camera_add(location=A.xyz((1.35,1.2,2.8)));cam=bpy.context.object;cam.rotation_euler=(Vector(A.xyz((0,.32,.02)))-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.type='ORTHO';cam.data.ortho_scale=1.65;scene.camera=cam
 lights=[]
 for p,power,size,col in [((2,4,4),420,4,(1,.87,.72)),((-3,2,2),300,3,(.65,.82,1)),((1,3,-3),450,3,(.2,.65,1))]:
  bpy.ops.object.light_add(type='AREA',location=A.xyz(p));o=bpy.context.object;o.data.energy=power;o.data.size=size;o.data.color=col;o.rotation_euler=(Vector(A.xyz((0,.3,0)))-o.location).to_track_quat('-Z','Y').to_euler();lights.append(o)
 scene.view_settings.view_transform='AgX';scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.filepath=str(A.OUT/('driver-'+kind+'.png'));bpy.ops.render.render(write_still=True)
 for o in lights+[cam]:bpy.data.objects.remove(o,do_unlink=True)
 for o in bpy.context.scene.objects:o.hide_render=False
 scene.render.film_transparent=False

def drivers():
 A.reset();roots={k:animal(k) for k in ['fox','panda','rabbit','robot']};A.export('drivers.glb')
 for k,r in roots.items():portrait(r,k)

def ground_roots():
 bpy.context.view_layer.update()
 for o in [o for o in bpy.context.scene.objects if o.parent is None]:
  points=[c.matrix_world@Vector(v) for c in o.children_recursive if c.type=='MESH' for v in c.bound_box]
  if points:
   low=min(v.z for v in points)
   for c in o.children:c.location.z-=low

def roof(g,w,h,d,y,mat='RoofRed'):
 for side in [-1,1]:
  o=A.box('Pitched roof',(side*w*.255,y+h*.5,0),(w*.59,.16,d),mat,g,.024);o.rotation_euler[1]=side*math.atan2(h,w*.5)
def window(g,x,y,z,w=.7):
 A.box('Window casing',(x,y,z),(w+.15,w+.2,.11),'Ivory',g,.018);A.box('Window glass',(x,y,z+.065),(w,w,.025),'Glass',g,.008)
 A.box('Window mullion',(x,y,z+.09),(.046,w,.025),'Ivory',g,.005);A.box('Window crossbar',(x,y,z+.09),(w,.046,.025),'Ivory',g,.005)
 A.box('Flower box',(x,y-w*.63,z+.16),(w+.08,.15,.21),'WoodPaint',g,.02)
 for j in range(4):uv('Flowers',(x-w*.35+j*w*.23,y-w*.55,z+.2),(.09,.075,.065),'CoralRose' if j%2 else 'Yellow',g,12)
def building(kind):
 g=A.group(kind);barn=kind=='Barn';w,d,h=(6,5,3) if barn else (4.8,3.8,2.6)
 A.box('Foundation',(0,.15,0),(w+.25,.3,d+.25),'Rock',g,.035);A.box('Wall body',(0,h/2,0),(w,h,d),'Ember' if barn else 'Plaster',g,.035)
 roof(g,w+ .5,1.25,d+.55,h-.01)
 # Gable closing triangle.
 for z in [-d/2,d/2]:
  vertices=[A.xyz((-w/2,h,z)),A.xyz((w/2,h,z)),A.xyz((0,h+1.25,z))];m=bpy.data.meshes.new('Gable');m.from_pydata(vertices,[],[(0,1,2)]);o=bpy.data.objects.new('Gable',m);bpy.context.collection.objects.link(o);A.finish(o,'Gable','WoodPaint',g)
 for x in [-w/2,w/2]:
  for z in [-d/2,d/2]:A.box('Corner timber',(x,h/2,z),(.15,h,.15),'Ivory',g,.015)
 if barn:
  for x in [-.74,.74]:
   A.box('Barn door',(x,1.25,d/2+.045),(1.4,2.4,.16),'WoodPaint',g,.02)
   for sign in [-1,1]:A.beam('Door crossbrace',(x-.58,.2 if sign>0 else 2.3,d/2+.15),(x+.58,2.3 if sign>0 else .2,d/2+.15),.04,'Ivory',g)
  for x in [-2.35,2.35]:window(g,x,1.5,d/2+.03,.65)
  A.box('Loft shutter',(0,3.08,d/2+.05),(.8,.68,.12),'Ivory',g,.012)
  for x in [-.22,0,.22]:A.box('Loft slat',(x,3.08,d/2+.125),(.08,.52,.018),'WoodPaint',g,.003)
 else:
  A.box('Front door',(0,.96,d/2+.045),(.88,1.9,.14),'WoodPaint',g,.028);A.cyl('Door handle',(.27,.94,d/2+.137),.042,.034,'Yellow',g,'Z')
  for x in [-1.48,1.48]:window(g,x,1.5,d/2+.04,.75)
  A.box('Chimney',(-1.3,3.51,-.9),(.48,1.6,.55),'RoofRed',g,.025)
  A.box('Porch step',(0,.12,d/2+.4),(1.5,.24,.7),'Rock',g,.028)
 return g

def scenery():
 A.reset();building('Cottage');building('Barn')
 g=A.group('Windmill');A.cone('Mill tower',(0,3,0),1.1,.69,6,'Plaster',g,16);A.cone('Mill cap',(0,6.45,0),1.02,0,1.1,'RoofRed',g,16)
 A.box('Mill door',(0,.75,1.045),(.65,1.5,.12),'WoodPaint',g,.025)
 A.cyl('Sail axle',(0,4.8,1.04),.24,.6,'Titanium',g,'Z');sails=A.group('WindmillSails',g,(0,4.8,1.35))
 for i in range(4):
  t=i*math.pi/2
  def q(x,y):return (x*math.cos(t)-y*math.sin(t),x*math.sin(t)+y*math.cos(t),0)
  A.beam('Sail spar',q(0,0),q(0,3.7),.07,'Bark',sails)
  for j in range(7):A.beam('Sail lattice',q(-.04,1+j*.38),q(.62,1+j*.38),.045,'Ivory',sails)
  A.beam('Sail edge',q(.62,1),q(.62,3.3),.048,'Bark',sails)
 for name in ['Fence','BridgeRail']:
  g=A.group(name);w=3 if name=='Fence' else 4
  for x in [-w/2,0,w/2]:A.box('Fence post',(x,.6,0),(.15,1.2,.15),'WoodPaint' if name=='Fence' else 'Bark',g,.02)
  for y in [.36,.87]:A.box('Fence rail',(0,y,0),(w+.16,.14,.12),'Ivory',g,.018)
 g=A.group('CoralFan')
 for i in range(11):
  ang=-1.15+i*.23;end=(math.sin(ang)*(1.1+random.random()*.25),.6+math.cos(ang)*1.35,random.uniform(-.08,.08));mid=(end[0]*.4,.65,end[2]);curve('Coral branch',[(0,.05,0),mid,end],.055,'CoralRose',g)
  for side in [-1,1]:curve('Coral twig',[(end[0]*.7,end[1]*.77,end[2]),(end[0]+side*.17,end[1]-.05,end[2])],.033,'CoralRose',g)
 g=A.group('CoralCluster')
 for i in range(7):
  a=i*2.399;r=.12+i*.11;x,z=math.cos(a)*r,math.sin(a)*r;h=.35+(i%3)*.32
  A.cone('Tube coral',(x,h/2,z),.19,.14,h,'CoralGold',g,12);A.torus('Coral tube lip',(x,h,z),.12,.042,'CoralRose',g);A.cyl('Coral opening',(x,h+.003,z),.092,.018,'FoxDark',g)
 g=A.group('Seaweed')
 for i in range(7):
  x=(i-3)*.16;h=1.3+(i%3)*.26;vertices=[]
  for j in range(8):
   y=h*j/7;xx=x+math.sin(j*.7+i)*.15;w=.06*math.sin((j/7)*math.pi)+.02
   vertices.extend([A.xyz((xx-w,y,math.sin(i)*.15)),A.xyz((xx+w,y,math.sin(i)*.15))])
  faces=[(j*2,j*2+1,j*2+3,j*2+2) for j in range(7)];m=bpy.data.meshes.new('Seaweed ribbon');m.from_pydata(vertices,[],faces);m.update();o=bpy.data.objects.new('Seaweed ribbon',m);bpy.context.collection.objects.link(o);A.finish(o,'Seaweed ribbon','OceanLeaf',g)
 g=A.group('Fish');uv('Fish body',(0,.27,0),(.14,.23,.37),'FishBlue',g,16)
 # Flattened tail and dorsal fins with bright edges and visible eyes on both sides.
 A.armor('Fish tail',(0,.27,-.37),[(-.19,.055,.2,-.04),(0,.045,.07,0),(.19,.055,.2,-.04)],'Yellow',g)
 A.armor('Dorsal fin',(0,.44,0),[(0,.045,.28,0),(.17,.02,.11,-.06)],'Yellow',g)
 for side in [-1,1]:
  uv('Fish eye',(side*.127,.31,.2),(.024,.053,.053),'EyeWhite',g,12);uv('Fish pupil',(side*.148,.31,.21),(.009,.025,.024),'EyeBlack',g,12)
  A.armor('Fish side fin',(side*.13,.22,-.02),[(-.07,.11,.12,-.04),(.04,.045,.1,0)],'CoralGold',g)
 g=A.group('ReefRock')
 A.rock(g,(0,.55,0),(1.5,.85,1.2),'Rock');A.rock(g,(.8,.3,.4),(.8,.48,.6),'SandLight')
 g=A.group('Buoy');A.uv('Buoy float',(0,.4,0),(.37,.42,.37),'Ember',g);A.cyl('Buoy stem',(0,1.0,0),.07,1.0,'Ivory',g);A.uv('Buoy beacon',(0,1.55,0),(.13,.13,.13),'AmberLight',g)
 A.join_static();ground_roots();bpy.context.view_layer.update();A.export('adventure-kit.glb')
 roots=[o for o in bpy.context.scene.objects if o.parent is None]
 for i,o in enumerate(roots):o.location=A.xyz(((i%4)*7,0,-(i//4)*8))
 A.render('adventure-kit',(31,25,29),(10,1,-7),35)

if __name__=='__main__':drivers();scenery();A.manifest()

def kart_previews():
 """Verify the actual driver/socket attachment in front and rear views."""
 for kind,color in [('fox','azure'),('panda','ember'),('rabbit','azure'),('robot','ember')]:
  A.reset();bpy.ops.import_scene.gltf(filepath=str(A.OUT/('kart-'+color+'.glb')));kart=bpy.data.objects['Kart'];socket=bpy.data.objects['DriverSocket']
  bpy.ops.import_scene.gltf(filepath=str(A.OUT/'drivers.glb'));driver=bpy.data.objects['Driver'+kind.title()]
  for root in [o for o in bpy.context.scene.objects if o.parent is None and o not in [driver,kart]]:
   for child in list(root.children_recursive):bpy.data.objects.remove(child,do_unlink=True)
   bpy.data.objects.remove(root,do_unlink=True)
  driver.parent=socket;driver.location=(0,0,0)
  A.render('kart-'+kind,(3.3,2.3,4),(0,.8,0),4.3)
  if kind=='fox':A.render('kart-fox-rear',(3.1,2.3,-4),(0,.8,0),4.3)
