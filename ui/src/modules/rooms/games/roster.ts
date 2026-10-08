export const DRIVERS = [
 {id:'fox',root:'DriverFox',name:'Rory',species:'Fox',motto:'Always chasing the next corner.'},
 {id:'panda',root:'DriverPanda',name:'Bao',species:'Panda',motto:'Big heart. Bigger overtakes.'},
 {id:'rabbit',root:'DriverRabbit',name:'Pip',species:'Rabbit',motto:'Born for the big jumps.'},
 {id:'robot',root:'DriverRobot',name:'Bolt',species:'Robot',motto:'A little metal. A lot of mischief.'},
] as const;
export type DriverId = typeof DRIVERS[number]['id'];
export function driverFor(id:string){return DRIVERS.find(d=>d.id===id)??DRIVERS[0];}
