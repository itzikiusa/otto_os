import type {Object3D} from 'three';
export interface VehiclePose {x:number;y:number;z:number;yaw:number}
/** Smooth the fixed-tick presentation without modifying authoritative physics.
 * Exponential damping is frame-rate independent; angle wrap must take the short arc.
 * A recovery teleport is intentional and must not animate through scenery.
 */
export function followVehicle(current:VehiclePose,target:VehiclePose,dt:number):VehiclePose {
 if(Math.hypot(target.x-current.x,target.y-current.y,target.z-current.z)>12)return {...target};
 const blend=1-Math.exp(-26*Math.max(0,dt));
 const angle=Math.atan2(Math.sin(target.yaw-current.yaw),Math.cos(target.yaw-current.yaw));
 return {x:current.x+(target.x-current.x)*blend,y:current.y+(target.y-current.y)*blend,z:current.z+(target.z-current.z)*blend,yaw:current.yaw+angle*blend};
}

/** Steering is around vertical Y before wheel spin around local axle X. */
export function vehicleWheels(root:Object3D):Object3D[] {
 return ['WheelFL','WheelFR','WheelRL','WheelRR'].map(name=>root.getObjectByName(name)).filter((wheel):wheel is Object3D=>!!wheel).map(wheel=>{wheel.rotation.order='YXZ';return wheel;});
}
