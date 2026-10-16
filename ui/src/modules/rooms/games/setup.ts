/** Builds match seats: humans in member-slot order, computer players fill the rest. */
import { RACERS, type RacerId } from './kart/roster';
import type { KartSeat } from './kart/sim';
import { FIGHTERS, type FighterId } from './shooter/catalog';

export interface HumanSeat { slot: number; name: string; pick: string }

export function kartSeats(humans: HumanSeat[], cpuFill: boolean, total = 8): KartSeat[] {
  const seats: KartSeat[] = humans.slice(0, total).map(h => ({ racer: (RACERS.some(r => r.id === h.pick) ? h.pick : 'oobi') as RacerId, human: true, slot: h.slot, name: h.name }));
  if (!cpuFill) return seats;
  const taken = new Set(seats.map(s => s.racer));
  const pool = [...RACERS.filter(r => !taken.has(r.id)), ...RACERS.filter(r => taken.has(r.id))];
  for (let i = 0; seats.length < total; i++) {
    const r = pool[i % pool.length];
    seats.push({ racer: r.id, human: false, slot: -1, name: r.name });
  }
  return seats;
}

export interface ShooterSeat { fighter: FighterId; human: boolean; slot: number; name: string }
export function shooterSeats(humans: HumanSeat[], bots: number): ShooterSeat[] {
  const seats: ShooterSeat[] = humans.map(h => ({ fighter: (FIGHTERS.some(f => f.id === h.pick) ? h.pick : 'male-a') as FighterId, human: true, slot: h.slot, name: h.name }));
  const taken = new Set(seats.map(s => s.fighter));
  const pool = [...FIGHTERS.filter(f => !taken.has(f.id)), ...FIGHTERS.filter(f => taken.has(f.id))];
  for (let i = 0; i < bots; i++) {
    const f = pool[i % pool.length];
    seats.push({ fighter: f.id, human: false, slot: -1, name: `${f.name}${i >= pool.length ? ' II' : ''}` });
  }
  return seats;
}
