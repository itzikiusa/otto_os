"""Two independent radial limbs striking different drum heads."""
import numpy as np
from .bodyplan import UP, axis_angle


def strike(sk, body, targets):
    from .pose import _slerp_matrix, rotation_between, Unsupported
    plan = body.plan or {}
    chains = [g["chain"] for g in plan.get("leg_chains", []) if len(g["chain"]) >= 3]
    world = sk.world()[:, :3, 3]
    swimming_hub = None
    if len(chains) < 4:
        # A swimming octopus need not have any floor contacts. Its arms fan
        # from one hub, even when the generic body classifier calls them ears.
        for hub in sorted(sk.rig.order, key=lambda k: -len(sk.rig.children[k])):
            if len(sk.rig.children[hub]) < 4:
                break
            branches = []
            for child in sk.rig.children[hub]:
                path = [child]
                while len(sk.rig.children[path[-1]]) == 1:
                    path.append(sk.rig.children[path[-1]][0])
                if (len(path) >= 3 and not sk.rig.children[path[-1]]
                        and np.linalg.norm(world[path[-1]] - world[hub]) >= .22 * body.height):
                    branches.append(path)
            if len(branches) >= 4:
                chains = branches
                swimming_hub = hub
                break
    if len(chains) < 4:
        raise Unsupported("striking with tentacles needs multiple independently articulated limbs")
    def cost(c, target):
        # Prefer the closest reachable tip without turning a chain through its own body.
        return np.linalg.norm(world[c[-1]]-target)
    a, b = min(((a,b) for a in chains for b in chains if not set(a) & set(b)),
               key=lambda cs:cost(cs[0],targets[0])+cost(cs[1],targets[1]))
    if swimming_hub is not None:
        # Lift the free rear arms into a fan behind the two playing arms.
        # Grounded legs remain in their supporting stance.
        centre = world[swimming_hub]
        forward = np.mean(targets, axis=0) - centre
        forward[1] = 0.
        forward /= max(np.linalg.norm(forward), 1e-8)
        for chain in chains:
            if chain in (a, b):
                continue
            outward = world[chain[-1]] - centre
            outward[1] = 0.
            outward /= max(np.linalg.norm(outward), 1e-8)
            behind = -float(outward @ forward)
            if behind > 0.:
                sk.turn(chain[1], axis_angle(np.cross(outward, UP), np.radians(35.) * behind))
    initial = sk.q.copy()
    def key(reverse):
        sk.q = initial.copy()
        for index, (chain, contact) in enumerate(zip((a,b),targets)):
            target = contact + UP * body.height * (.14 if index == reverse else .015)
            for _ in range(5):
                for joint in reversed(chain[:-1]):
                    w = sk.world()
                    origin = w[joint][:3,3]
                    toward, tip = target - origin, w[chain[-1]][:3,3]-origin
                    if min(np.linalg.norm(toward),np.linalg.norm(tip)) < 1e-7:
                        continue
                    change = rotation_between(tip, toward)
                    sk.turn(joint, _slerp_matrix(change, 0.60), w)
        return sk.q.copy()
    q0, q1 = key(0), key(1)
    sk.q = q0.copy()
    return q0, q1, [c[-1] for c in (a,b)], [c[-1] for c in chains]


def cycle(sk, poses):
    q0, q1 = poses[:2]
    q1 = np.where((q0*q1).sum(axis=1)[:,None]<0,-q1,q1)
    times = np.linspace(0.0, 0.8, 25)
    weight = ((1.-np.cos(2*np.pi*times/times[-1]))/2.)[:,None,None]
    frames = (1.-weight)*q0+weight*q1
    frames /= np.linalg.norm(frames,axis=2,keepdims=True)
    return times, frames, np.tile(sk.T[sk.root],(len(times),1)), np.tile(sk.S[sk.root],(len(times),1))
